mod lsp_test_support;

use std::time::Duration;
use test_utils::spawn_lsp_session;

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Verify fast typing stays responsive while background LSP work is active.
#[test]
fn test_lsp_insert_mode_stays_responsive_during_fast_typing() {
    let workspace =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace.path().join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    lsp_test_support::warm_up_helper_value_hover(&mut session);

    session
        .send_text("gg0i")
        .expect("enter insert mode at file start");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("1/15:1")
        })
        .expect("wait for insert mode");

    let fast_text = "zzzzzzzzzzzzzzzzzzzzzzzzzzzz";
    session
        .send_text(fast_text)
        .expect("type many characters quickly");
    session
        .wait_until(Duration::from_millis(500), |screen| {
            screen.row_contains(1, fast_text)
        })
        .expect("typed text should appear promptly");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
