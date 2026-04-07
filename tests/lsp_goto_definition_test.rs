use std::path::PathBuf;
use std::time::Duration;
use test_utils::PtySession;

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Spawn Ordex for one or more LSP fixture files.
fn spawn_lsp_session(file_paths: &[PathBuf]) -> PtySession {
    let args = file_paths
        .iter()
        .map(|path| path.to_str().expect("utf8 fixture path"))
        .collect::<Vec<_>>();
    PtySession::spawn(ordex_bin(), &args, Default::default()).expect("spawn ordex")
}

/// Verify `g d` opens one definition in another file after the real server finishes indexing.
#[test]
fn test_goto_definition_opens_unopened_file_target() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&[main_rs]);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/helper_value()")
        .expect("search for unopened-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the helper_value call");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1:8")
        })
        .expect("definition jump should open lib.rs");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify `g d` can also jump to a definition that lives in the current file.
#[test]
fn test_goto_definition_opens_same_file_target() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&[main_rs]);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/local_value")
        .expect("search for same-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("5:13")
        })
        .expect("cursor should land on the local_value call");

    session
        .send_text("gd")
        .expect("request same-file definition");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(8, "fn local_value() -> i32") && screen.status_line_contains("8:4")
        })
        .expect("definition jump should stay in main.rs");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
