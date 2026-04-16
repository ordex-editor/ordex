use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempTree, spawn_lsp_session_with_config};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the filesystem path for `binary` when it exists on `PATH`.
fn command_path(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

/// Return whether every named binary exists on `PATH`.
///
/// Returns `true` when all requested binaries are available, and `false` when at
/// least one binary is missing.
fn commands_available(binaries: &[&str]) -> bool {
    binaries.iter().all(|binary| command_path(binary).is_some())
}

/// Create one symlink to a real binary inside `bin_dir`.
fn link_real_binary(bin_dir: &Path, binary: &str) -> io::Result<()> {
    let target = command_path(binary).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("missing required binary on PATH: {binary}"),
        )
    })?;
    symlink(target, bin_dir.join(binary))
}

/// Spawn Ordex for one file with the supplied `PATH` override.
fn spawn_session_with_path(file_path: &Path, current_dir: &Path, path_env: String) -> PtySession {
    spawn_lsp_session_with_config(
        ordex_bin(),
        &[file_path.to_path_buf()],
        PtySessionConfig {
            current_dir: Some(current_dir.to_path_buf()),
            env: vec![("PATH".to_string(), path_env)],
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Build one standalone Python workspace without project markers.
fn standalone_python_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    tree.write_file(
        "main.py",
        "import os\nprint(missing_name)\n\ndef helper():\n    return 1\n\nhelper()\n",
    )
    .expect("write main.py");
    tree
}

/// Build one standalone C++ workspace without project markers.
fn standalone_cpp_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    tree.write_file("main.cpp", "int main() {\n    return missing_name;\n}\n")
        .expect("write main.cpp");
    tree
}

/// Build a PATH directory that exposes only the selected real binaries.
fn filtered_path_with_real_binaries(tree: &TempTree, binaries: &[&str]) -> String {
    let bin_dir = tree.path().join("real-bin");
    fs::create_dir_all(&bin_dir).expect("create real-bin");
    // Symlink the real binaries so the test can remove `ty` from PATH without
    // substituting another server executable.
    for binary in binaries {
        link_real_binary(&bin_dir, binary).expect("link real binary");
    }
    bin_dir.display().to_string()
}

/// Move the cursor to the Python `helper()` call so `gd` resolves its definition.
fn focus_python_helper_call(session: &mut PtySession) {
    session.send_text("/helper()").expect("search helper");
    session.send_enter().expect("confirm helper search");
    // The first match is the function definition, so advance once to the call site.
    session.send_text("n").expect("move to helper call");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("7:1")
        })
        .expect("wait for helper call");
}

/// Verify a standalone Python file uses real `ty` navigation and real `ruff` diagnostics.
#[test]
fn test_standalone_python_file_uses_real_ty_and_ruff() {
    if !commands_available(&["ty", "ruff"]) {
        eprintln!("skipping Python real-server test because ty or ruff is unavailable");
        return;
    }

    let workspace = standalone_python_workspace();
    let main_py = workspace.path().join("main.py");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_py, workspace.path(), path_env);

    // Wait for the standalone file to open before checking background diagnostics.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "import os")
        })
        .expect("wait for main.py");

    // The merged diagnostics view should include findings from both real servers.
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(1, "●") && screen.row_contains(2, "●")
        })
        .expect("wait for standalone Python diagnostics");
    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker");
    session.send_enter().expect("execute diagnostics command");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.contains("Ruff") && screen.contains("ty") && screen.contains("missing_name")
        })
        .expect("diagnostics picker should show Ruff and ty entries");
    session.exit_to_normal_mode(Duration::from_secs(2));

    // The same buffer should also resolve go-to-definition through ty.
    focus_python_helper_call(&mut session);
    session.send_text("gd").expect("request Python definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(4, "def helper():") && screen.status_line_contains("4:5")
        })
        .expect("definition should jump to helper");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone Python file falls back to real `pylsp` navigation when `ty` is absent.
#[test]
fn test_standalone_python_file_falls_back_to_real_pylsp() {
    if !commands_available(&["pylsp", "ruff"]) {
        eprintln!("skipping pylsp fallback test because pylsp or ruff is unavailable");
        return;
    }

    let workspace = standalone_python_workspace();
    let main_py = workspace.path().join("main.py");
    let path_env = filtered_path_with_real_binaries(&workspace, &["pylsp", "ruff"]);
    let mut session = spawn_session_with_path(&main_py, workspace.path(), path_env);

    // Wait for the standalone file to open before issuing navigation requests.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "import os")
        })
        .expect("wait for main.py");

    // `ty` is absent from PATH here, so the definition lookup should fall back to pylsp.
    focus_python_helper_call(&mut session);
    session.send_text("gd").expect("request Python definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(4, "def helper():") && screen.status_line_contains("4:5")
        })
        .expect("definition should jump to helper through pylsp");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone C++ file uses real `clangd` without project markers.
#[test]
fn test_standalone_cpp_file_uses_real_clangd() {
    if !commands_available(&["clangd"]) {
        eprintln!("skipping clangd test because clangd is unavailable");
        return;
    }

    let workspace = standalone_cpp_workspace();
    let main_cpp = workspace.path().join("main.cpp");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_cpp, workspace.path(), path_env);

    // Wait for the standalone C++ file to open before checking diagnostics.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "int main() {")
        })
        .expect("wait for main.cpp");

    // A real clangd diagnostic confirms the fallback root is sufficient to start the server.
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(2, "●")
        })
        .expect("wait for clangd diagnostics");
    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker");
    session.send_enter().expect("execute diagnostics command");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.contains("Use of undeclared identifier") && screen.contains("clang")
        })
        .expect("diagnostics picker should show clangd output");
    session.exit_to_normal_mode(Duration::from_secs(2));

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
