mod lsp_test_support;

use std::time::Duration;
use test_utils::{PtySession, overlay_footer_hidden, overlay_footer_visible};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Verify live LSP progress renders and clears during a real definition lookup.
#[test]
fn test_goto_definition_shows_and_clears_lsp_progress_overlay() {
    let workspace =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace.path().join("src/main.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_rs.to_str().expect("utf8 fixture path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    session
        .send_text("/helper_value\\(\\)")
        .expect("search for unopened-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/15:13")
        })
        .expect("cursor should land on the helper_value call");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(12), |screen| {
            overlay_footer_visible(screen)
        })
        .expect("LSP progress overlay should become visible");
    session
        .wait_until(Duration::from_secs(40), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1/8:8")
        })
        .expect("definition jump should open lib.rs");
    session
        .wait_until(Duration::from_secs(12), |screen| {
            overlay_footer_hidden(screen)
        })
        .expect("LSP progress overlay should clear after definition progress stops");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
