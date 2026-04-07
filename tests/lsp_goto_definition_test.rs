use std::path::PathBuf;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Return the repository root used for relative-path startup coverage.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Spawn Ordex for one or more LSP fixture files.
fn spawn_lsp_session(file_paths: &[PathBuf]) -> PtySession {
    let args = file_paths
        .iter()
        .map(|path| path.to_str().expect("utf8 fixture path"))
        .collect::<Vec<_>>();
    PtySession::spawn(ordex_bin(), &args, Default::default()).expect("spawn ordex")
}

/// Spawn Ordex for one or more LSP fixture files with an explicit PTY config.
fn spawn_lsp_session_with_config(file_paths: &[&str], config: PtySessionConfig) -> PtySession {
    PtySession::spawn(ordex_bin(), file_paths, config).expect("spawn ordex")
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

/// Verify relative startup paths still produce file URIs that rust-analyzer accepts.
#[test]
fn test_goto_definition_opens_unopened_file_target_from_relative_path() {
    let mut session = spawn_lsp_session_with_config(
        &["tests/fixtures/lsp/workspace_one/src/main.rs"],
        PtySessionConfig {
            current_dir: Some(repo_root()),
            ..Default::default()
        },
    );

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for relative startup path");

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
        .expect("definition jump should open lib.rs from a relative startup path");

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

/// Verify unsaved edits still keep the LSP document synchronized before `gd`.
#[test]
fn test_goto_definition_after_unsaved_edit_uses_latest_buffer_state() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&[main_rs]);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("O// note")
        .expect("insert comment above import");
    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "// note")
                && screen.row_contains(2, "use workspace_one")
        })
        .expect("unsaved edit should remain visible");

    session
        .send_text("/helper_value()")
        .expect("search for shifted symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("5:13")
        })
        .expect("cursor should land on the shifted helper_value call");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1:8")
        })
        .expect("definition jump should use the edited buffer state");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify same-file go-to-definition still lands on the shifted definition after multiline edits.
#[test]
fn test_goto_definition_same_file_after_multiline_unsaved_edit_uses_shifted_target() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&[main_rs]);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("O// note a\n// note b\n// note c")
        .expect("insert multiline comment above import");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "// note a")
                && screen.row_contains(2, "// note b")
                && screen.row_contains(3, "// note c")
        })
        .expect("multiline edit should remain visible");

    session
        .send_text("/local_value")
        .expect("search for same-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("8:13")
        })
        .expect("cursor should land on the shifted local_value call");

    session
        .send_text("gd")
        .expect("request same-file definition");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(11, "fn local_value() -> i32")
                && screen.status_line_contains("11:4")
        })
        .expect("definition jump should use the shifted same-file target");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify same-file go-to-definition still lands on the function line after body-only edits.
#[test]
fn test_goto_definition_same_file_after_multiline_body_edit_stays_on_definition_line() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&[main_rs]);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session.send_text("/11").expect("search for function body");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("9:5")
        })
        .expect("cursor should land on the function body");

    session
        .send_text("Olet inserted_a = 1;\nlet inserted_b = 2;\nlet inserted_c = 3;")
        .expect("insert multiline body text");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(9, "let inserted_a = 1;")
                && screen.row_contains(10, "let inserted_b = 2;")
                && screen.row_contains(11, "let inserted_c = 3;")
        })
        .expect("body edit should remain visible");

    session
        .send_text("gg/local_value")
        .expect("search for same-file call");
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
        .expect("definition jump should stay on the function line");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
