use std::fs;
use std::thread;
use std::time::Duration;
use test_utils::{PtySession, ScreenSnapshot, TempTree, spawn_lsp_session};

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

/// Return whether the LSP progress footer is absent from the current screen.
fn overlay_footer_hidden(screen: &ScreenSnapshot) -> bool {
    (24..=27).all(|row| !screen.row_contains(row, "rust-analyzer"))
}

/// Wait until startup analysis has visibly settled for the active LSP session.
fn wait_for_startup_analysis_to_settle(session: &mut PtySession) {
    let _ = session.wait_until(Duration::from_secs(8), |screen| {
        (24..=27).any(|row| screen.row_contains(row, "rust-analyzer"))
    });
    // Rust-analyzer can briefly drop the footer before continuing startup work,
    // so wait for a short streak of idle samples instead of one instant.
    for _ in 0..5 {
        session
            .wait_until(Duration::from_secs(12), |screen| {
                overlay_footer_hidden(screen) && !screen.status_line_contains("● ")
            })
            .expect("startup analysis should settle without diagnostics");
        thread::sleep(Duration::from_millis(200));
    }
}

/// Verify the code-action picker opens for one quick fix and Enter applies it.
#[test]
fn test_lsp_code_action_picker_applies_selected_fix() {
    let workspace = code_action_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");
    wait_for_startup_analysis_to_settle(&mut session);

    // Trigger the save-driven rustc warning so the matching quick fix becomes available.
    session.send_text(":w").expect("save startup buffer");
    session.send_enter().expect("confirm save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(2, "●")
                && screen.status_line_contains("● 1")
        })
        .expect("unused mut diagnostic should render");

    session
        .send_text("/mut value")
        .expect("search for unused mut binding");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("2:9")
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
                && screen.row_contains(2, "let value = 1;")
                && screen.message_line_contains("Applied code action")
        })
        .expect("enter should apply the selected code action");

    session.send_text(":w").expect("save updated buffer");
    session.send_enter().expect("confirm save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.row_contains(2, "let value = 1;")
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
