mod lsp_test_support;

use std::path::PathBuf;
use std::time::Duration;
use test_utils::{
    PtySessionConfig, missing_server_path_env, spawn_lsp_session, spawn_lsp_session_with_config,
};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Verify `g d` opens one definition in another file after the real server finishes indexing.
#[test]
fn test_goto_definition_opens_unopened_file_target() {
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
        .wait_until(Duration::from_secs(40), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1/8:8")
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
    let workspace =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_one");
    let mut session = spawn_lsp_session_with_config(
        ordex_bin(),
        &[PathBuf::from("src/main.rs")],
        PtySessionConfig {
            current_dir: Some(workspace.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for relative startup path");
    lsp_test_support::warm_up_helper_value_hover(&mut session);

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
        .wait_until(Duration::from_secs(40), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1/8:8")
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
        .send_text("/local_value")
        .expect("search for same-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("5/15:13")
        })
        .expect("cursor should land on the local_value call");

    session
        .send_text("gd")
        .expect("request same-file definition");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(8, "fn local_value() -> i32")
                && screen.status_line_contains("8/15:4")
        })
        .expect("definition jump should stay in main.rs");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify same-line definition jumps clear the resolving message in the terminal UI.
#[test]
fn test_goto_definition_same_line_jump_clears_resolving_message() {
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

    session.send_text("/main").expect("search for main symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(3, "fn main() {") && screen.status_line_contains("3/15:4")
        })
        .expect("cursor should land on the main symbol");

    session
        .send_text("gd")
        .expect("request same-line definition");
    lsp_test_support::wait_until_stable(
        &mut session,
        Duration::from_secs(8),
        Duration::from_millis(600),
        |screen| {
            screen.row_trimmed_ends_with(3, "fn main() {")
                && screen.status_line_contains("3/15:4")
                && !screen.message_line_contains("Resolving definition...")
        },
    )
    .expect("same-line definition jump should clear the resolving message");

    session
        .send_text("l")
        .expect("move right after definition jump");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("3/15:5")
                && !screen.message_line_contains("Resolving definition...")
        })
        .expect("moving right should not reveal a stale resolving message");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify explicit definition lookups report a clear PATH-specific startup error.
#[test]
fn test_goto_definition_reports_missing_server_binary() {
    let workspace =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace.path().join("src/main.rs");
    let (_path_fixture, path_env) = missing_server_path_env();
    let mut session = spawn_lsp_session_with_config(
        ordex_bin(),
        &[main_rs],
        PtySessionConfig {
            env: vec![("PATH".to_string(), path_env)],
            ..Default::default()
        },
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
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("language server \"rust-analyzer\" is not in PATH")
                && screen.message_line_contains("install \"rust-analyzer\"")
        })
        .expect("missing-server message should be visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify unsaved edits still keep the LSP document synchronized before `gd`.
#[test]
fn test_goto_definition_after_unsaved_edit_uses_latest_buffer_state() {
    let workspace =
        lsp_test_support::isolated_fixture_workspace("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace.path().join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/helper_value\\(\\)")
        .expect("search for warmup symbol");
    session.send_enter().expect("confirm warmup search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/15:13")
        })
        .expect("cursor should land on the warmup helper_value call");
    // Warm up rust-analyzer before the edit so the assertion only exercises the
    // unsaved-buffer synchronization path instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);

    session
        .send_text("ggO// note")
        .expect("insert comment above import");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session
        .wait_until(Duration::from_secs(6), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "// note")
                && screen.row_contains(2, "use workspace_one")
        })
        .expect("unsaved edit should remain visible");

    session
        .send_text("/helper_value\\(\\)")
        .expect("search for shifted symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("5/16:13")
        })
        .expect("cursor should land on the shifted helper_value call");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1/8:8")
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
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/helper_value\\(\\)")
        .expect("search for warmup symbol");
    session.send_enter().expect("confirm warmup search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/15:13")
        })
        .expect("cursor should land on the warmup helper_value call");
    // Warm up rust-analyzer before the edit so the assertion only exercises the
    // unsaved-buffer synchronization path instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);

    session
        .send_text("ggO// note a\nnote b\nnote c")
        .expect("insert multiline comment above import");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("3/18:10")
        })
        .expect("multiline insert should finish before escape");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session
        .wait_until(Duration::from_secs(6), |screen| {
            screen.row_trimmed_ends_with(1, "// note a")
                && screen.row_trimmed_ends_with(2, "// note b")
                && screen.row_trimmed_ends_with(3, "// note c")
        })
        .expect("multiline edit should remain visible");

    session
        .send_text("/local_value")
        .expect("search for same-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("8/18:13")
        })
        .expect("cursor should land on the shifted local_value call");

    session
        .send_text("gd")
        .expect("request same-file definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(11, "fn local_value() -> i32")
                && screen.status_line_contains("11/18:4")
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
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/helper_value\\(\\)")
        .expect("search for warmup symbol");
    session.send_enter().expect("confirm warmup search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4/15:13")
        })
        .expect("cursor should land on the warmup helper_value call");
    // Warm up rust-analyzer before the edit so the assertion only exercises the
    // unsaved-buffer synchronization path instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);

    session.send_text("/11").expect("search for function body");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("9/15:5")
        })
        .expect("cursor should land on the function body");

    session
        .send_text("Olet inserted_a = 1;\nlet inserted_b = 2;\nlet inserted_c = 3;")
        .expect("insert multiline body text");
    // The popup can refresh while rust-analyzer is still indexing, so wait for
    // the full multiline insert to settle before forcing the escape sequence.
    session
        .wait_until(Duration::from_secs(15), |screen| {
            screen.row_trimmed_ends_with(9, "let inserted_a = 1;")
                && screen.row_trimmed_ends_with(10, "let inserted_b = 2;")
                && screen.row_trimmed_ends_with(11, "let inserted_c = 3;")
        })
        .expect("inserted body text should appear before leaving insert mode");
    session.exit_to_normal_mode(Duration::from_secs(5));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(9, "let inserted_a = 1;")
                && screen.row_trimmed_ends_with(10, "let inserted_b = 2;")
                && screen.row_trimmed_ends_with(11, "let inserted_c = 3;")
        })
        .expect("body edit should remain visible");

    session
        .send_text("gg/local_value")
        .expect("search for same-file call");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("5/18:13")
        })
        .expect("cursor should land on the local_value call");

    session
        .send_text("gd")
        .expect("request same-file definition");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.row_contains(8, "fn local_value() -> i32")
                && screen.status_line_contains("8/18:4")
        })
        .expect("definition jump should stay on the function line");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
