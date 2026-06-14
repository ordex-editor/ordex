use std::time::Duration;
use test_utils::{
    StartupAnalysisWaitOptions, TempTree, hello_world_workspace, overlay_footer_hidden,
    spawn_lsp_session, wait_for_startup_analysis_to_settle, wait_for_write_confirmation,
};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Build one temporary Cargo workspace for save-triggered semantic diagnostics.
fn semantic_diagnostics_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    // Keep the fixture minimal so save-triggered semantic diagnostics settle quickly.
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"semantic_diag_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "use std::collections::HashMap;\n\nfn main() {\n    let used = 1;\n    let _ = used;\n    let _map: HashMap<String, String> = HashMap::new();\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Return the stricter startup-settle policy used by saved semantic-warning checks.
///
/// Compared with the default startup wait, these options require visible startup
/// progress, double the idle samples, lengthen the sample gap, and increase the
/// idle timeout so the first saved semantic warning starts after rust-analyzer's
/// slower background analysis has fully gone idle with a clean status line.
fn saved_semantic_warning_wait_options() -> StartupAnalysisWaitOptions {
    StartupAnalysisWaitOptions {
        wait_for_visible_progress: true,
        idle_samples: 10,
        sample_gap: Duration::from_millis(300),
        idle_timeout: Duration::from_secs(20),
        require_clear_diagnostics: true,
    }
}

/// Warm the save-triggered semantic-diagnostics path before timing one warning save.
///
/// This helper creates a temporary unused-variable warning, saves until that
/// warning renders, then removes it and waits for the gutter to clear again.
/// The timed assertion then runs after the same session has already paid the
/// cold-start semantic-check cost that made the original test flaky.
fn warm_up_saved_semantic_warning(session: &mut test_utils::PtySession) {
    // First create one untimed saved warning in the same file so rust-analyzer
    // finishes the slow cold-start semantic-check path before the real assertion.
    session
        .send_text("GO    let warmup = true;")
        .expect("insert warmup warning");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.row_trimmed_ends_with(7, "    let warmup = true;")
                && screen.status_line_contains("INSERT ")
        })
        .expect("wait for warmup warning line");
    session.exit_to_normal_mode(Duration::from_secs(6));
    // Force one full save-triggered semantic warning so the timed assertion
    // exercises the hot path instead of the initial cargo-check startup cost.
    session.send_text(":w").expect("save warmup warning");
    session.send_enter().expect("execute warmup warning save");
    wait_for_write_confirmation(session);
    session
        .wait_until(Duration::from_secs(20), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(7, "●")
                && screen.status_line_contains("● 1")
        })
        .expect("warmup warning should appear");
    // Then remove that temporary warning and wait for the gutter to clear so the
    // timed assertion starts from the same clean state as the original test.
    session.send_text("dd").expect("delete warmup warning line");
    session
        .send_text(":w")
        .expect("save warmup warning removal");
    session
        .send_enter()
        .expect("execute warmup warning removal");
    wait_for_write_confirmation(session);
    session
        .wait_until(Duration::from_secs(12), |screen| {
            overlay_footer_hidden(screen)
                && !screen.row_contains(7, "●")
                && !screen.status_line_contains("● ")
        })
        .expect("warmup warning should clear");
}

/// Verify one saved semantic warning appears quickly for a small Rust file.
#[test]
fn test_lsp_diagnostics_warning_appears_quickly_after_save() {
    let workspace = semantic_diagnostics_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "use std::collections::HashMap;")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session, saved_semantic_warning_wait_options());
    warm_up_saved_semantic_warning(&mut session);

    // Save one semantic warning through a single-line insert so the check-on-save
    // path stays focused on one stable unused-variable diagnostic.
    session
        .send_text("GO    let value = true;")
        .expect("insert one saved warning");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.row_trimmed_ends_with(7, "    let value = true;")
                && screen.status_line_contains("INSERT ")
        })
        .expect("wait for inserted warning line");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session.send_text(":w").expect("save warning");
    session.send_enter().expect("execute save");
    wait_for_write_confirmation(&mut session);

    session
        .wait_until(Duration::from_secs(15), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(7, "●")
                && screen.status_line_contains("● 1")
        })
        .expect("saved warning should appear quickly");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify one saved trailing-expression error appears quickly and clears after removal.
#[test]
fn test_lsp_diagnostics_error_clears_quickly_after_saved_removal() {
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

    // Save one explicit parser error so the regression focuses on gutter clearing.
    session.send_text("ggjA\n1 +").expect("insert error");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.row_trimmed_ends_with(3, "1 +") && screen.status_line_contains("INSERT ")
        })
        .expect("wait for inserted error line");
    session.exit_to_normal_mode(Duration::from_secs(6));
    session
        .send_text(":w")
        .expect("save trailing-expression error");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(30), |screen| {
            screen.row_trimmed_ends_with(3, "1 +")
                && screen.status_line_contains("● ")
                && overlay_footer_hidden(screen)
                && (screen.row_contains(3, "●") || screen.row_contains(4, "●"))
        })
        .expect("saved error should appear quickly");

    // Delete the known trailing-expression line directly because rust-analyzer
    // may place the saved diagnostic marker on either adjacent line.
    session
        .send_text("ggjjdd")
        .expect("delete saved error line");
    session.send_text(":w").expect("save repaired file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for second write confirmation");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_hidden(screen)
                && !screen.row_contains(3, "1 +")
                && !screen.row_contains(3, "●")
                && !screen.row_contains(4, "●")
                && !screen.status_line_contains("● ")
        })
        .expect("saved error should clear quickly after removal");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
