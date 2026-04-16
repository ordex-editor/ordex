use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;
use test_utils::{PtySessionConfig, TempTree, spawn_lsp_session_with_config};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Write one executable fake language-server script into `bin_dir`.
fn install_fake_server(
    bin_dir: &Path,
    server_name: &str,
    diagnostics_json: &str,
) -> std::io::Result<()> {
    let script_path = bin_dir.join(server_name);
    let script = format!(
        r#"#!/usr/bin/env python3
import json
import sys

DIAGNOSTICS = {diagnostics_json}

def send(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {{len(body)}}\r\n\r\n".encode("ascii"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

def read_message():
    headers = {{}}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        name, value = line.decode("utf-8").split(":", 1)
        headers[name.lower()] = value.strip()
    length = int(headers.get("content-length", "0"))
    if length == 0:
        return None
    return json.loads(sys.stdin.buffer.read(length))

published = False
while True:
    message = read_message()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize":
        send({{"jsonrpc": "2.0", "id": message["id"], "result": {{"capabilities": {{"textDocumentSync": 1}}}}}})
        continue
    if method == "shutdown":
        send({{"jsonrpc": "2.0", "id": message["id"], "result": None}})
        continue
    if method == "exit":
        break
    if method == "textDocument/didOpen" and not published:
        params = message["params"]["textDocument"]
        send({{
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {{
                "uri": params["uri"],
                "version": params.get("version"),
                "diagnostics": DIAGNOSTICS,
            }},
        }})
        published = True
"#,
    );
    fs::write(&script_path, script)?;
    // The PTY tests invoke these scripts through PATH, so each file must be executable.
    let mut permissions = fs::metadata(&script_path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(script_path, permissions)
}

/// Return a PATH value that prefers the fake-server directory.
fn fake_server_path(bin_dir: &Path) -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    format!("{}:{existing}", bin_dir.display())
}

/// Spawn Ordex with one temporary workspace and fake-server PATH overrides.
fn spawn_with_fake_servers(workspace: &TempTree, file_path: &Path) -> test_utils::PtySession {
    let bin_dir = workspace.path().join("fake-bin");
    let path_env = fake_server_path(&bin_dir);
    // Override only PATH so the test drives the real Ordex binary with fake servers.
    let config = PtySessionConfig {
        current_dir: Some(workspace.path().to_path_buf()),
        env: vec![("PATH".to_string(), path_env)],
        ..Default::default()
    };
    spawn_lsp_session_with_config(ordex_bin(), &[file_path.to_path_buf()], config)
        .expect("spawn ordex")
}

/// Build one Python workspace with fake `ty`, `ruff`, and `pylsp` binaries.
fn python_multiserver_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    let bin_dir = tree.path().join("fake-bin");
    fs::create_dir_all(&bin_dir).expect("create fake-bin");
    tree.write_file(
        "pyproject.toml",
        "[project]\nname = 'fixture'\nversion = '0.1.0'\n",
    )
    .expect("write pyproject");
    tree.write_file("main.py", "first = 1\nsecond = 2\n")
        .expect("write main.py");

    // `ty` participates in sync routing but stays silent so diagnostics come from the other two servers.
    install_fake_server(&bin_dir, "ty", "[]").expect("install ty");
    install_fake_server(
        &bin_dir,
        "ruff",
        r#"[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}},"severity":2,"message":"ruff says first line is suspicious","source":"ruff"}]"#,
    )
    .expect("install ruff");
    install_fake_server(
        &bin_dir,
        "pylsp",
        r#"[{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":6}},"severity":2,"message":"pylsp says second line is suspicious","source":"pylsp"}]"#,
    )
    .expect("install pylsp");
    tree
}

/// Build one standalone C++ workspace with a fake `clangd` binary and no markers.
fn standalone_cpp_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    let bin_dir = tree.path().join("fake-bin");
    fs::create_dir_all(&bin_dir).expect("create fake-bin");
    tree.write_file("main.cpp", "int main() {\n    return 0;\n}\n")
        .expect("write main.cpp");

    // This fake clangd proves Ordex can start C/C++ LSP from the opened file directory alone.
    install_fake_server(
        &bin_dir,
        "clangd",
        r#"[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"severity":2,"message":"clangd fallback diagnostic","source":"clangd"}]"#,
    )
    .expect("install clangd");
    tree
}

/// Verify one Python buffer merges diagnostics from multiple cooperating LSP servers.
#[test]
fn test_python_buffer_merges_diagnostics_from_multiple_servers() {
    let workspace = python_multiserver_workspace();
    let main_py = workspace.path().join("main.py");
    let mut session = spawn_with_fake_servers(&workspace, &main_py);

    // Wait until the buffer is open before checking for asynchronous diagnostics.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "first = 1")
        })
        .expect("wait for main.py");

    // Both fake diagnostics should surface on separate lines in the same buffer.
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(1, "●") && screen.row_contains(2, "●")
        })
        .expect("wait for merged diagnostics");

    // The diagnostics picker should list entries from both contributing servers.
    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker");
    session.send_enter().expect("execute diagnostics picker");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.contains("ruff says first line is suspicious")
                && screen.contains("pylsp says second line is suspicious")
        })
        .expect("diagnostics picker should contain both server messages");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify clangd starts for standalone C++ files even when no marker file is present.
#[test]
fn test_clangd_starts_without_project_markers() {
    let workspace = standalone_cpp_workspace();
    let main_cpp = workspace.path().join("main.cpp");
    let mut session = spawn_with_fake_servers(&workspace, &main_cpp);

    // Wait until the editor opens the C++ file before expecting diagnostics.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "int main() {")
        })
        .expect("wait for main.cpp");

    // A diagnostic marker proves the fake clangd process started and owned the file.
    session
        .wait_until(Duration::from_secs(8), |screen| screen.row_contains(1, "●"))
        .expect("wait for clangd diagnostic");

    // The diagnostics picker confirms the diagnostic content came from the clangd route.
    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker");
    session.send_enter().expect("execute diagnostics picker");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.contains("clangd fallback diagnostic")
        })
        .expect("diagnostics picker should contain the clangd message");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
