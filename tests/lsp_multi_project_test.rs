use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the compiled fake rust-analyzer binary path for PTY-backed LSP tests.
fn fake_rust_analyzer_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fake_rust_analyzer")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Spawn Ordex with both LSP-related test overrides configured.
fn spawn_lsp_session(file_paths: &[PathBuf], log_file: &TempFile) -> PtySession {
    let args = file_paths
        .iter()
        .map(|path| path.to_str().expect("utf8 fixture path"))
        .collect::<Vec<_>>();
    PtySession::spawn(
        ordex_bin(),
        &args,
        PtySessionConfig {
            env: vec![
                (
                    "ORDEX_RUST_ANALYZER".to_string(),
                    fake_rust_analyzer_bin().to_string(),
                ),
                (
                    "ORDEX_FAKE_RA_LOG".to_string(),
                    log_file.path().display().to_string(),
                ),
            ],
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Wait until `query` selects one buffer-switch entry and confirm it.
fn switch_to_buffer(session: &mut PtySession, query: &str, expected_row: &str) {
    // Drive the shared picker path because this test cares about one process
    // reusing workspace sessions across multiple open buffers.
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

/// Verify one Ordex process reuses rust-analyzer per workspace root.
#[test]
fn test_goto_definition_reuses_one_session_per_workspace() {
    let workspace_one_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let workspace_two_root = fixture_path("tests/fixtures/lsp/workspace_two");
    let workspace_one_main = workspace_one_root.join("src/main.rs");
    let workspace_two_main = workspace_two_root.join("src/main.rs");
    let log_file = TempFile::with_suffix("_fake_ra.log").expect("create fake rust-analyzer log");
    let mut session = spawn_lsp_session(
        &[workspace_one_main.clone(), workspace_two_main.clone()],
        &log_file,
    );

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for first startup buffer");

    session
        .send_text("wwgd")
        .expect("lookup definition in workspace one");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.row_contains(1, "pub fn helper_value() -> i32")
        })
        .expect("open first workspace definition");

    switch_to_buffer(
        &mut session,
        "workspace_one/src/main.rs",
        "use workspace_one::helper_value;",
    );
    session
        .send_text("wwgd")
        .expect("repeat lookup in workspace one");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.row_contains(1, "pub fn helper_value() -> i32")
        })
        .expect("open first workspace definition again");

    switch_to_buffer(
        &mut session,
        "workspace_two/src/main.rs",
        "use workspace_two::helper_name;",
    );
    session
        .send_text("wwgd")
        .expect("lookup definition in workspace two");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.row_contains(1, "pub fn helper_name() -> &'static str")
        })
        .expect("open second workspace definition");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let lines = fs::read_to_string(log_file.path())
        .expect("read fake rust-analyzer log")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        lines,
        vec![
            format!("file://{}", workspace_one_root.display()),
            format!("file://{}", workspace_two_root.display()),
        ]
    );
}
