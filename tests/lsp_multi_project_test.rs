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

/// Wait until `query` selects one buffer-switch entry and confirm it.
fn switch_to_buffer(session: &mut PtySession, query: &str, expected_row: &str) {
    // Drive the shared picker path because this test cares about navigation
    // behavior while multiple buffers remain open in one editor session.
    session
        .send_text(&format!(" b{query}"))
        .expect("open buffer switcher");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.contains(query) && !screen.row_contains(1, expected_row)
        })
        .expect("wait for matching buffer-switch entry");
    session.send_enter().expect("confirm buffer switch");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.row_contains(1, expected_row)
        })
        .expect("target buffer should become active");
}

/// Verify one editor session resolves definitions correctly across two workspaces.
#[test]
fn test_goto_definition_uses_the_active_workspace() {
    let workspace_one_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let workspace_two_root = fixture_path("tests/fixtures/lsp/workspace_two");
    let workspace_one_main = workspace_one_root.join("src/main.rs");
    let workspace_two_main = workspace_two_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&[workspace_one_main.clone(), workspace_two_main.clone()]);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for first startup buffer");

    session
        .send_text("/helper_value()")
        .expect("search for workspace-one symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the workspace-one call");

    session
        .send_text("gd")
        .expect("lookup definition in workspace one");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.tab_line_contains("lib.rs") && screen.row_contains(1, "pub fn helper_value()")
        })
        .expect("open first workspace definition");

    switch_to_buffer(
        &mut session,
        "workspace_two/src/main.rs",
        "use workspace_two::helper_name;",
    );
    session
        .send_text("/helper_name()")
        .expect("search for workspace-two symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the workspace-two call");

    session
        .send_text("gd")
        .expect("lookup definition in workspace two");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(1, "pub fn helper_name() -> &'static str")
                && screen.tab_line_contains("lib.rs")
        })
        .expect("open second workspace definition");

    switch_to_buffer(
        &mut session,
        "workspace_one/src/main.rs",
        "use workspace_one::helper_value;",
    );
    session
        .send_text("/helper_value()")
        .expect("search for workspace-one symbol again");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the workspace-one call again");

    session
        .send_text("gd")
        .expect("lookup definition in workspace one again");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(1, "pub fn helper_value() -> i32")
        })
        .expect("switching back should still resolve within workspace one");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
