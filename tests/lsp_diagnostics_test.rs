use std::time::Duration;
use test_utils::{TempTree, spawn_lsp_session};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
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

/// Verify startup diagnostics render, list in the picker, and support navigation.
#[test]
fn test_lsp_diagnostics_render_list_and_navigate() {
    let workspace = diagnostic_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

    session
        .wait_until(Duration::from_secs(12), |screen| {
            screen.row_contains(2, "•")
                && screen.row_contains(3, "•")
                && screen.contains("missing_one")
        })
        .expect("startup diagnostics should render");

    session.send_text("]d").expect("jump to first diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("2:13") && screen.contains("missing_one")
        })
        .expect("next diagnostic should jump to missing_one");

    session.send_text("]d").expect("jump to second diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("3:13") && screen.contains("missing_two")
        })
        .expect("next diagnostic should jump to missing_two");

    session
        .send_text("[d")
        .expect("jump back to first diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("2:13") && screen.contains("missing_one")
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

    session.exit_to_normal_mode(Duration::from_secs(2));
}
