//! Liveness coverage for the editor while a language server misbehaves.
//!
//! Language-server traffic runs on worker threads so the editor keeps drawing and
//! reading keys while a server is slow. A server that accepts the connection and
//! then stops draining its stdin must therefore cost nothing but language
//! features: the buffer stays editable and the screen stays live.

// Test fixtures wait for short helper commands, with no event loop to stall.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempTree, wait_for_write_confirmation};

/// Source of the fake language server this suite puts on `PATH`.
const NON_DRAINING_SERVER_SOURCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/non_draining_lsp_server.rs"
);

/// Environment variable the fake server reads to pick when it stops reading.
const WEDGE_POINT_VARIABLE: &str = "ORDEX_FAKE_LSP_WEDGE_AFTER";

/// Time a wedged write needs to reach the pipe and stay there.
///
/// The keystroke that probes for liveness has to arrive after the document write
/// has actually blocked, otherwise it is answered before the transport is
/// contended at all and the assertion proves nothing.
const WEDGE_SETTLE_DELAY: Duration = Duration::from_secs(2);

/// Return the path of the Ordex binary built for this test run.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Compile the fake language server into `directory` under the name Ordex looks for.
///
/// Building it here rather than declaring a Cargo target keeps a test-only helper
/// out of the binaries Ordex ships, and `rustc` is already required to reach this
/// test at all.
fn build_non_draining_server(directory: &Path) {
    let output = Command::new("rustc")
        .args(["--edition", "2024", "-o"])
        .arg(directory.join("rust-analyzer"))
        .arg(NON_DRAINING_SERVER_SOURCE)
        .output()
        .expect("run rustc");
    assert!(
        output.status.success(),
        "compiling the fake language server failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Open a Cargo project on a source file large enough to overflow the server pipe.
///
/// The document notification Ordex sends for this file is far past the 64 KiB
/// pipe buffer, so a server that never reads leaves the write blocked mid-payload.
/// `wedge_point` selects how far the server plays along before it stops reading.
fn spawn_editor_against_non_draining_server(
    tools: &TempTree,
    project: &TempTree,
    wedge_point: &str,
) -> PtySession {
    build_non_draining_server(tools.path());

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
            env: vec![
                ("PATH".to_string(), search_path),
                (WEDGE_POINT_VARIABLE.to_string(), wedge_point.to_string()),
                (
                    "ORDEX_LSP_TRACE".to_string(),
                    project.path().join("lsp-trace.log").display().to_string(),
                ),
                ("RUST_BACKTRACE".to_string(), "1".to_string()),
            ],
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Assert the editor still answers the keyboard by switching into insert mode.
fn assert_editor_answers_keys(session: &mut PtySession) {
    session.send_text("i").expect("send insert key");
    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("INSERT ")
        })
        .expect("editor must react to keystrokes while the server is wedged");
}

#[test]
fn test_opening_frame_precedes_language_server_traffic() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project, "initialize");

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
                && snapshot.row_trimmed_ends_with(1, "fn main() {}")
        })
        .expect("opening frame must not wait for language-server traffic");
}

/// Editing must stay possible while a language server refuses to read its stdin.
///
/// The `didOpen` payload outgrows the pipe buffer, so a client that writes it
/// from a thread holding a session lock parks every later user of that lock,
/// including the editor's own event loop.
#[test]
fn test_editor_stays_responsive_while_language_server_stops_reading() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project, "initialize");

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("opening frame must be painted");

    std::thread::sleep(WEDGE_SETTLE_DELAY);
    assert_editor_answers_keys(&mut session);
}

/// A server that wedges only after the document is open must not freeze editing.
///
/// One edit is enough: the debounced `didChange` carries the whole file, so it
/// blocks on the pipe exactly like the opening notification would, and the
/// editor must still return to normal mode and take the next key.
#[test]
fn test_editor_stays_responsive_after_editing_against_a_wedged_server() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project, "did_open");

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("opening frame must be painted");

    session.send_text("ix").expect("insert one character");
    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.row_trimmed_ends_with(1, "xfn main() {}")
        })
        .expect("the edit must land in the buffer");
    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("editor must return to normal mode");

    std::thread::sleep(WEDGE_SETTLE_DELAY);
    assert_editor_answers_keys(&mut session);
}

/// Saving must not freeze the editor when a watching server stops reading.
///
/// The server registers a watcher for the whole workspace and then wedges, so
/// the save path has both a `didSave` and a `didChangeWatchedFiles` to deliver
/// from the editor's own thread.
#[test]
fn test_editor_stays_responsive_after_saving_against_a_wedged_server() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project, "watched_files");

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("opening frame must be painted");

    session.send_text("ix").expect("insert one character");
    session.send_escape().expect("leave insert mode");
    session.send_text(":w").expect("start the write command");
    session.send_enter().expect("run the write command");
    wait_for_write_confirmation(&mut session);

    std::thread::sleep(WEDGE_SETTLE_DELAY);
    assert_editor_answers_keys(&mut session);
}

/// A lookup the server never answers must leave the editor usable and fail when the server exits.
///
/// The server keeps reading but answers nothing after the handshake, then exits
/// with the definition request still pending. Keys must work while it waits, and
/// the closed transport must fail the lookup; a client that only learns about
/// failures from responses keeps showing "Resolving definition..." forever.
#[test]
fn test_unanswered_lookup_fails_once_the_language_server_exits() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session =
        spawn_editor_against_non_draining_server(&tools, &project, "unanswered_definition");

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("opening frame must be painted");

    let sent_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    session.send_text("gd").expect("request definition");
    let pending = session.wait_until(Duration::from_secs(10), |snapshot| {
        snapshot.message_line_contains("Resolving definition...")
    });
    if let Err(error) = pending {
        let trace = std::fs::read_to_string(project.path().join("lsp-trace.log"))
            .unwrap_or_else(|error| format!("<no trace: {error}>"));
        eprintln!("=== DEBUG gd sent at {sent_at}");
        eprintln!("=== DEBUG LSP trace");
        for line in trace.lines() {
            eprintln!("{}", line.chars().take(400).collect::<String>());
        }
        let raw = session.snapshot().raw().to_string();
        eprintln!("=== DEBUG raw transcript length {}", raw.len());
        for (index, line) in raw.split('\n').enumerate() {
            let lowered = line.to_lowercase();
            if lowered.contains("panic") || lowered.contains("thread") || lowered.contains("backtrace") {
                eprintln!("raw line {index}: {:?}", line.chars().take(2000).collect::<String>());
            }
        }
        let tail_start = raw.len().saturating_sub(4000);
        let tail_start = (tail_start..raw.len())
            .find(|position| raw.is_char_boundary(*position))
            .unwrap_or(raw.len());
        eprintln!("=== DEBUG raw tail {:?}", &raw[tail_start..]);
        panic!("the definition lookup must be pending: {error}");
    }
    assert_editor_answers_keys(&mut session);
    session.send_escape().expect("leave insert mode");

    session
        .wait_until(Duration::from_secs(15), |snapshot| {
            snapshot.message_line_contains("language server transport closed")
        })
        .expect("the closed transport must fail the pending lookup");
}

/// Quitting must not wait on a language server that stopped answering.
///
/// The `shutdown` request is the last thing Ordex asks a server for, and a
/// wedged one never replies, so an editor that waits for that reply never
/// reaches its own exit.
#[test]
fn test_editor_exits_while_the_language_server_is_wedged() {
    let tools = TempTree::with_prefix("ordex_liveness_bin").expect("create tool tree");
    let project = TempTree::with_prefix("ordex_liveness_project").expect("create project");
    let mut session = spawn_editor_against_non_draining_server(&tools, &project, "did_open");

    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("opening frame must be painted");

    session.send_text("ix").expect("insert one character");
    session.send_escape().expect("leave insert mode");
    std::thread::sleep(WEDGE_SETTLE_DELAY);

    session.send_text(":").expect("open the command line");
    session
        .wait_until(Duration::from_secs(10), |snapshot| {
            snapshot.status_line_contains("COMMAND ")
        })
        .expect("command line must open");
    session.send_text("q!").expect("type the quit command");
    session
        .wait_until(Duration::from_secs(10), |snapshot| snapshot.contains(":q!"))
        .expect("the quit command must be typed");
    session.send_enter().expect("run the quit command");

    session
        .wait_for_exit(Duration::from_secs(15))
        .expect("editor must exit while the server is wedged");
}
