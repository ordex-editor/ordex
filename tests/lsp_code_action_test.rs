use std::fs;
use std::time::Duration;
use test_utils::{
    PtySessionConfig, StartupAnalysisWaitOptions, TempTree, overlay_footer_hidden,
    spawn_lsp_session_with_config, wait_for_startup_analysis_to_settle,
};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Build one temporary Rust workspace that exposes an unused-`mut` quick fix.
fn code_action_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    // A tiny standalone crate is enough for rust-analyzer to surface the
    // unused-`mut` diagnostic and its associated quick fix.
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"code_action_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n    let mut value = 1;\n    println!(\"{value}\");\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Verify the code-action picker opens for one quick fix and Enter applies it.
#[test]
fn test_lsp_code_action_picker_applies_selected_fix() {
    let workspace = code_action_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session = spawn_lsp_session_with_config(
        ordex_bin(),
        std::slice::from_ref(&main_rs),
        PtySessionConfig {
            cols: 160,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for main.rs");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(2, "●")
                && screen.status_line_contains("● 1")
        })
        .expect("unused mut diagnostic should render");
    wait_for_startup_analysis_to_settle(
        &mut session,
        StartupAnalysisWaitOptions {
            require_clear_diagnostics: false,
            ..Default::default()
        },
    );

    session
        .send_text("/mut value")
        .expect("search for unused mut binding");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("2/4:9")
        })
        .expect("cursor should land on mut");

    session.send_text(" a").expect("request code actions");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.contains("Code Actions")
        })
        .expect("code-action picker should open");

    // Confirm the single quick fix from the picker so the workspace edit is
    // applied through the same path that real multi-action selections use.
    session.send_enter().expect("apply code action");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(2, "let value = 1;")
                && screen.message_line_contains("Applied code action")
        })
        .expect("enter should apply the selected code action");

    session.send_text(":w").expect("save updated buffer");
    session.send_enter().expect("confirm save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written")
                && screen.row_trimmed_ends_with(2, "let value = 1;")
        })
        .expect("save should persist the applied code action");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    assert!(
        fs::read_to_string(&main_rs)
            .expect("read main.rs after quit")
            .contains("let value = 1;")
    );
}
