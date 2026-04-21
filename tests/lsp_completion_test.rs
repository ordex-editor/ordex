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

/// Verify insert-mode completion shows rust-analyzer items with a visible kind label.
#[test]
fn test_lsp_completion_popup_shows_function_kind() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("jjjo")
        .expect("open line below helper_value");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("5:1")
        })
        .expect("wait for insert mode");

    // Typing a call prefix should keep insert mode responsive while the popup
    // later merges in rust-analyzer results for the active buffer snapshot.
    session
        .send_text("    helper_v")
        .expect("type completion prefix");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("helper_value")
                && screen.contains("function")
                && !screen.contains("assert!")
        })
        .expect("wait for LSP completion popup");

    session.send_escape().expect("leave insert mode");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify trigger-character completion works for module paths like `use std::`.
#[test]
fn test_lsp_completion_popup_shows_module_members_after_trigger_character() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session.send_text("O").expect("open line above");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("1:1")
        })
        .expect("wait for insert mode");

    session
        .send_text("use std::")
        .expect("type use path trigger");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("alloc") && screen.contains("collections") && screen.contains("module")
        })
        .expect("wait for trigger completion popup");

    session.send_escape().expect("leave insert mode");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify previously returned LSP candidates stay locally filterable during fast nested typing.
#[test]
fn test_lsp_completion_popup_keeps_nested_path_matches_while_typing_quickly() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session.send_text("O").expect("open line above");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("1:1")
        })
        .expect("wait for insert mode");

    session
        .send_text("use std::alloc::Glo")
        .expect("type nested path quickly");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("GlobalAlloc")
                && (screen.contains("interface") || screen.contains("trait"))
        })
        .expect("wait for nested-path completion popup");

    session.send_escape().expect("leave insert mode");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
