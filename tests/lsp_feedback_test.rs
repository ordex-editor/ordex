use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Spawn Ordex for one unsupported-file lookup test rooted at `current_dir`.
fn spawn_lsp_session_in_dir(
    file_path: &std::path::Path,
    current_dir: std::path::PathBuf,
) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file_path.to_str().expect("utf8 fixture path")],
        PtySessionConfig {
            current_dir: Some(current_dir),
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Verify unsupported files stay in place and report a clear error.
#[test]
fn test_goto_definition_reports_unsupported_file() {
    let file = TempFile::with_suffix(".txt").expect("create temp file");
    file.write_all(b"plain text\n").expect("seed file");
    let current_dir = std::env::current_dir().expect("read current directory");
    let mut session = spawn_lsp_session_in_dir(file.path(), current_dir);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "plain text")
        })
        .expect("wait for txt file");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("is not a supported Rust source file")
                && screen.row_contains(1, "plain text")
        })
        .expect("unsupported-file message should be visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify Rust files outside a supported workspace report a clear error.
#[test]
fn test_goto_definition_reports_unsupported_project() {
    let file = TempFile::with_suffix(".rs").expect("create temp file");
    file.write_all(b"fn main() {}\n").expect("seed file");
    let current_dir = std::env::current_dir().expect("read current directory");
    let mut session = spawn_lsp_session_in_dir(file.path(), current_dir);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {}")
        })
        .expect("wait for rust file");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("is not inside a supported Cargo workspace")
                && screen.row_contains(1, "fn main() {}")
        })
        .expect("unsupported-project message should be visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
