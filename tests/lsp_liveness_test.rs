//! Liveness coverage for the editor while a language server misbehaves.
//!
//! Language-server traffic runs on worker threads so the editor keeps drawing and
//! reading keys while a server is slow. A server that accepts the connection and
//! then stops draining its stdin must therefore cost nothing but language
//! features: the buffer stays editable and the screen stays live.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempTree};

/// Fake language server that completes the handshake and then stops reading.
///
/// It answers `initialize` so Ordex treats the session as healthy, then leaves
/// its stdin unread. Ordex's next document notification fills the pipe buffer and
/// the write blocks part-way through.
const NON_DRAINING_SERVER: &str = r#"#!/usr/bin/env python3
import sys, time

def read_message():
    length = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line in (b"\r\n", b"\n"):
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1].strip())
    return sys.stdin.buffer.read(length)

body = read_message().decode("utf-8", "replace")
request_id = body.split('"id":')[1].split(",")[0].strip()
response = (
    '{"jsonrpc":"2.0","id":%s,"result":{"capabilities":{"textDocumentSync":1}}}' % request_id
).encode()
sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(response) + response)
sys.stdout.buffer.flush()
time.sleep(120)
"#;

/// Return the path of the Ordex binary built for this test run.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Open a Cargo project on a source file large enough to overflow the server pipe.
///
/// The document notification Ordex sends for this file is far past the 64 KiB
/// pipe buffer, so a server that never reads leaves the write blocked mid-payload.
fn spawn_editor_against_non_draining_server(tools: &TempTree, project: &TempTree) -> PtySession {
    let server_path = tools.path().join("rust-analyzer");
    std::fs::write(&server_path, NON_DRAINING_SERVER).expect("write fake server");
    std::fs::set_permissions(&server_path, std::fs::Permissions::from_mode(0o755))
        .expect("make fake server executable");

    project
        .write_file(
            "Cargo.toml",
            "[package]\nname = \"liveness\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest");
    let mut source = String::from("fn main() {}\n");
    for index in 0..8_000 {
        source.push_str(&format!(
            "// filler line {index} padded out so the payload passes the pipe buffer\n"
        ));
    }
    project
        .write_file("src/main.rs", &source)
        .expect("write source");

    let source_path = project.path().join("src/main.rs");
    let search_path = format!(
        "{}:{}",
        tools.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    PtySession::spawn(
        ordex_bin(),
        &[source_path.to_string_lossy().as_ref()],
        PtySessionConfig {
            current_dir: Some(project.path().to_path_buf()),
            env: vec![("PATH".to_string(), search_path)],
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

#[test]
fn test_opening_frame_precedes_language_server_traffic() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project);

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
                && snapshot.row_trimmed_ends_with(1, "fn main() {}")
        })
        .expect("opening frame must not wait for language-server traffic");
}

/// Editing must stay possible while a language server refuses to read its stdin.
///
/// The document write blocks part-way through the payload while it holds the
/// session state mutex, and the editor's main loop takes that same mutex on every
/// iteration, so the editor stops answering the keyboard for as long as the
/// server stays wedged.
#[test]
#[ignore = "known deadlock: the document write holds the session state mutex across a blocking pipe write"]
fn test_editor_stays_responsive_while_language_server_stops_reading() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project);

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("opening frame must be painted");

    // The keystroke has to arrive after the document write has actually blocked,
    // otherwise it is answered before the session state mutex is ever contended.
    std::thread::sleep(Duration::from_secs(2));
    session.send_text("i").expect("send insert key");
    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("INSERT ")
        })
        .expect("editor must react to keystrokes while the server is wedged");
}
