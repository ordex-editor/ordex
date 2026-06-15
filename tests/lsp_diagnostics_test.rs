mod lsp_test_support;

use std::time::Duration;
use test_utils::{
    ScreenSnapshot, TempTree, hello_world_workspace, overlay_footer_hidden, spawn_lsp_session,
    wait_for_startup_analysis_to_settle,
};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return whether one line shows an active diagnostic with the expected message.
///
/// Returns `true` when the gutter marker, status-line summary, and a matching
/// diagnostic message are all visible after the progress footer clears, and
/// `false` otherwise.
fn diagnostic_visible(screen: &ScreenSnapshot, line: usize, message_fragment: &str) -> bool {
    overlay_footer_hidden(screen)
        && screen.row_contains(line, "●")
        && screen.status_line_contains("● ")
        && screen.contains(message_fragment)
}

/// Return whether one line no longer shows any active diagnostics.
///
/// Returns `true` when the gutter marker and status-line summary are both gone
/// after the progress footer clears, and `false` otherwise.
fn diagnostic_cleared(screen: &ScreenSnapshot, line: usize) -> bool {
    overlay_footer_hidden(screen)
        && !screen.row_contains(line, "●")
        && !screen.status_line_contains("● ")
}

/// Build one temporary Cargo workspace with two startup diagnostics.
fn diagnostic_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"diag_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n    let _ = missing_one;\n    let _ = missing_two;\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Build one temporary Cargo workspace without startup diagnostics.
fn clean_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"clean_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n    let used = 1;\n    let _ = used;\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Verify startup diagnostics render, list in the picker, and support navigation.
#[test]
fn test_lsp_diagnostics_render_list_and_navigate() {
    let workspace = diagnostic_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for main.rs");

    // rust-analyzer may deliver diagnostics in separate notifications, so require
    // both gutter markers to be present and stable before proceeding.
    lsp_test_support::wait_until_stable(
        &mut session,
        Duration::from_secs(20),
        Duration::from_millis(300),
        |screen| {
            screen.row_contains(2, "●")
                && screen.row_contains(3, "●")
                && screen.contains("missing_one")
        },
    )
    .expect("startup diagnostics should render");

    session.send_text("]d").expect("jump to first diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("2/4:13") && screen.contains("missing_one")
        })
        .expect("next diagnostic should jump to missing_one");

    session.send_text("]d").expect("jump to second diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("3/4:13") && screen.contains("missing_two")
        })
        .expect("next diagnostic should jump to missing_two");

    session
        .send_text("[d")
        .expect("jump back to first diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("2/4:13") && screen.contains("missing_one")
        })
        .expect("previous diagnostic should jump back to missing_one");

    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker command");
    session.send_enter().expect("confirm diagnostics command");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.contains("Diagnostics")
                && screen.contains("missing_one")
                && screen.contains("missing_two")
        })
        .expect("diagnostics picker should list both startup diagnostics");

    session.exit_to_normal_mode(Duration::from_secs(6));
}

/// Verify live `didChange` updates remove diagnostics after in-memory edits.
#[test]
fn test_lsp_diagnostics_refresh_after_edit() {
    let workspace = clean_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session, Default::default());

    session
        .send_text("GOlet broken = ;")
        .expect("insert one parse error before closing brace");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session.send_text(":w").expect("save broken file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(20), |screen| {
            diagnostic_visible(screen, 4, "expected expression")
        })
        .expect("save should surface diagnostics before the local fix");

    session.send_text("dd").expect("delete invalid line");
    session
        .wait_until(Duration::from_secs(12), |screen| {
            diagnostic_cleared(screen, 4)
        })
        .expect("live diagnostics should disappear after the local edit");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify one saved trailing expression still surfaces diagnostics.
#[test]
fn test_lsp_diagnostics_appear_after_saved_trailing_expression_edit() {
    let workspace = hello_world_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session, Default::default());

    // Insert an incomplete trailing expression inside `main`, then save it.
    // A parser error is stable here, while the unresolved-name variant depends
    // on slower semantic analysis that does not publish reliably in CI.
    session
        .send_text("ggjA\n1 +")
        .expect("insert one incomplete trailing expression");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session.send_text(":w").expect("save edited file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(30), |screen| {
            screen.row_contains(3, "1 +")
                && screen.status_line_contains("● ")
                && overlay_footer_hidden(screen)
        })
        .expect("saved diagnostics should appear for the trailing expression");
    // Restart from the first column so diagnostic navigation reaches whichever
    // line rust-analyzer reports for the saved parser error.
    session
        .send_text("gg0]d")
        .expect("jump to the saved trailing-expression diagnostic");
    session
        .wait_until(Duration::from_secs(30), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(3, "1 +")
                && screen.status_line_contains("● ")
                && screen.contains("expected expression")
                && (screen.row_contains(3, "●") || screen.row_contains(4, "●"))
        })
        .expect("diagnostic navigation should surface the trailing-expression error");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify save-triggered diagnostics disappear after a saved fix.
#[test]
fn test_lsp_diagnostics_refresh_after_save_fix() {
    let workspace = clean_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session, Default::default());

    session
        .send_text("GOlet broken = ;")
        .expect("insert one parse error before closing brace");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session.send_text(":w").expect("save fix");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(4, "●") && overlay_footer_hidden(screen)
        })
        .expect("save-triggered diagnostics should appear");

    session.send_text("dd").expect("delete invalid line");
    session.send_text(":w").expect("save repaired file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for second write confirmation");

    session
        .wait_until(Duration::from_secs(20), |screen| {
            overlay_footer_hidden(screen) && !screen.row_contains(4, "●")
        })
        .expect("save-triggered diagnostics should clear after the fix");

    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker command");
    session.send_enter().expect("confirm diagnostics command");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("No diagnostics in active buffer")
        })
        .expect("diagnostics picker should report an empty buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify save-triggered diagnostics appear and remain visible after progress clears.
#[test]
fn test_lsp_diagnostics_appear_after_save_and_persist_after_analysis() {
    let workspace = clean_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session, Default::default());

    session
        .send_text("GOlet broken = ;")
        .expect("insert one parse error before closing brace");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session.send_text(":w").expect("save new warning");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(30), |screen| {
            diagnostic_visible(screen, 4, "expected expression")
        })
        .expect("save-triggered diagnostics should remain visible after analysis");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
