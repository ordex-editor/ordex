mod lsp_test_support;

use std::path::Path;
use std::time::Duration;
use test_utils::{PtySession, spawn_lsp_session};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one short buffer-picker query token derived from a workspace path.
fn workspace_query_token(main_path: &Path) -> String {
    let parent = main_path
        .parent()
        .and_then(Path::parent)
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .expect("workspace directory name");
    let mut parts = parent.rsplit('_');
    let tail = parts.next().expect("workspace index suffix");
    let head = parts.next().expect("workspace id suffix");
    format!("{head}_{tail}")
}

/// Wait until `query` selects one buffer-switch entry and confirm it.
fn switch_to_buffer(session: &mut PtySession, query: &str, expected_row: &str) {
    // Drive the shared picker path because this test cares about navigation
    // behavior while multiple buffers remain open in one editor session.
    session
        .send_text(&format!(" b{query}"))
        .expect("open buffer switcher");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.contains(query)
                && screen.contains("src/main.rs")
                && screen.contains(expected_row)
        })
        .expect("wait for matching buffer-switch entry");
    session.send_enter().expect("confirm buffer switch");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_trimmed_ends_with(1, expected_row)
        })
        .expect("target buffer should become active");
}

/// Verify one editor session resolves definitions correctly across two workspaces.
#[test]
fn test_goto_definition_uses_the_active_workspace() {
    let workspace_one =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_one");
    let workspace_two =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_two");
    let workspace_one_main = workspace_one.path().join("src/main.rs");
    let workspace_two_main = workspace_two.path().join("src/main.rs");
    let workspace_one_query = workspace_query_token(&workspace_one_main);
    let workspace_two_query = workspace_query_token(&workspace_two_main);
    let mut session = spawn_lsp_session(
        ordex_bin(),
        &[workspace_one_main.clone(), workspace_two_main.clone()],
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for first startup buffer");
    session
        .send_text("/helper_value\\(\\)")
        .expect("search for workspace-one symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/15:13")
        })
        .expect("cursor should land on the workspace-one call");

    session
        .send_text("gd")
        .expect("lookup definition in workspace one");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.tab_line_contains("lib.rs") && screen.row_contains(1, "pub fn helper_value()")
        })
        .expect("open first workspace definition");

    switch_to_buffer(
        &mut session,
        &format!("{workspace_two_query}/src/main.rs"),
        "use workspace_two::helper_name;",
    );
    session
        .send_text("/helper_name\\(\\)")
        .expect("search for workspace-two symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/5:13")
        })
        .expect("cursor should land on the workspace-two call");

    session
        .send_text("gd")
        .expect("lookup definition in workspace two");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(1, "pub fn helper_name() -> &'static str")
                && screen.tab_line_contains("lib.rs")
        })
        .expect("open second workspace definition");

    switch_to_buffer(
        &mut session,
        &format!("{workspace_one_query}/src/main.rs"),
        "use workspace_one::helper_value;",
    );
    session
        .send_text("/helper_value\\(\\)")
        .expect("search for workspace-one symbol again");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/15:13")
        })
        .expect("cursor should land on the workspace-one call again");

    session
        .send_text("gd")
        .expect("lookup definition in workspace one again");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(1, "pub fn helper_value() -> i32")
        })
        .expect("switching back should still resolve within workspace one");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
