use std::path::PathBuf;
use std::time::Duration;
use test_utils::spawn_lsp_session;

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Verify insert-mode completion shows rust-analyzer items with a visible kind label.
#[test]
fn test_lsp_completion_popup_shows_function_kind() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/helper_value()")
        .expect("search for helper_value call");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the helper_value call");

    session.send_text("o").expect("open line below");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("5:1")
        })
        .expect("wait for insert mode");

    // Typing a call prefix should keep insert mode responsive while the popup
    // later merges in rust-analyzer results for the active buffer snapshot.
    session
        .send_text("    helper_v")
        .expect("type completion prefix");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("helper_value") && screen.contains("function")
        })
        .expect("wait for LSP completion popup");

    session.send_escape().expect("leave insert mode");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
