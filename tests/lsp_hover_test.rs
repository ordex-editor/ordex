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

/// Verify `K` shows hover information and the popup dismisses on the next keypress.
#[test]
fn test_hover_opens_popup_and_dismisses_on_next_key() {
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

    session.send_text("K").expect("request hover");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("Hover") && screen.contains("fn helper_value() -> i32")
        })
        .expect("hover popup should show the helper_value signature");

    session.send_text("j").expect("dismiss hover and move down");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(5, "    let _ = local_value();")
                && screen.status_line_contains("5:13")
        })
        .expect("next keypress should dismiss hover before moving the cursor");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
