mod lsp_test_support;

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

/// Verify `g r` opens the references picker and can jump to a filtered result.
#[test]
fn test_goto_references_opens_unopened_file_reference() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let lib_rs = workspace_root.join("src/lib.rs");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[lib_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
        })
        .expect("wait for lib.rs");

    session
        .send_text("/helper_value\\(\\) -> i32")
        .expect("search for definition symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("1:8")
        })
        .expect("cursor should land on the helper_value definition");

    session.send_text("gr").expect("request references");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.contains("References")
                && screen.contains("workspace_one/src/m")
                && screen.contains("Preview")
                && screen.contains("use workspace_one::helper_value;")
                && !screen.contains(format!("{}:1:20", main_rs.display()).as_str())
                && !screen.contains(format!("{}:4:13", main_rs.display()).as_str())
        })
        .expect("references picker should list both returned targets");

    session
        .send_text("4:13")
        .expect("filter picker to the call site");
    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.tab_line_contains("main.rs")
                && screen.row_contains(4, "    let _ = helper_value();")
                && screen.status_line_contains("4:13")
        })
        .expect("references jump should open the filtered target");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify same-file go-to-references uses the latest unsaved buffer state.
#[test]
fn test_goto_references_same_file_after_unsaved_edit_uses_shifted_target() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    // Warm up rust-analyzer before the unsaved edit so the assertion only
    // exercises the shifted-buffer references path instead of startup timing.
    session
        .send_text("/helper_value\\(\\)")
        .expect("search for warmup symbol");
    session.send_enter().expect("confirm warmup search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the warmup helper_value call");
    lsp_test_support::warm_up_helper_value_hover(&mut session);

    session
        .send_text("ggO// note a\nnote b")
        .expect("insert multiline comment above import");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("2:10")
        })
        .expect("multiline insert should finish before escape");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "// note a") && screen.row_contains(2, "// note b")
        })
        .expect("multiline edit should remain visible");

    session
        .send_text("/local_value\\(\\) -> i32")
        .expect("search for definition symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("10:4")
        })
        .expect("cursor should land on the shifted local_value definition");

    session.send_text("gr").expect("request references");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(7, "    let _ = local_value();")
                && screen.status_line_contains("7:13")
        })
        .expect("references jump should use the shifted same-file target");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
