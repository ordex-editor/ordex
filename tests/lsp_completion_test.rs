mod lsp_test_support;

use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use test_utils::{
    PTY_BACKSPACE, PtySessionConfig, missing_server_path_env, spawn_lsp_session,
    spawn_lsp_session_with_config,
};

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
    // Warm up rust-analyzer before the completion request so the assertion only
    // exercises popup rendering instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

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
    // Warm up rust-analyzer before the completion request so the assertion only
    // exercises popup rendering instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

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

/// Verify LSP signature help updates the active parameter while typing arguments.
#[test]
fn test_lsp_signature_help_updates_active_parameter_while_typing_arguments() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

    session
        .send_text("jjjo")
        .expect("open line below helper_value");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("5:1")
        })
        .expect("wait for insert mode");

    session
        .send_text("    let _ = helper_sum(")
        .expect("type helper_sum call");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("Signature Help")
                && screen.contains("helper_sum(")
                && screen.contains("Adds two numbers.")
        })
        .expect("wait for first signature-help popup with docs");

    session.send_text("1, ").expect("type first argument");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("Signature Help") && screen.contains("Adds two numbers.")
        })
        .expect("wait for retriggered signature-help popup");

    session.send_text("2)").expect("finish helper_sum call");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            (1..=30).all(|row| !screen.row_contains(row, "Signature Help"))
        })
        .expect("signature-help popup should close after the call ends");

    session.send_escape().expect("leave insert mode");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify missing server binaries stay quiet during automatic signature-help lookups.
#[test]
fn test_lsp_signature_help_stays_quiet_when_server_is_missing_from_path() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
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
        .send_text("jjjo")
        .expect("open line below helper_value");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("5:1")
        })
        .expect("wait for insert mode");

    session
        .send_text("    let _ = helper_sum(")
        .expect("type helper_sum call");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.contains("    let _ = helper_sum(")
        })
        .expect("wait for typed helper_sum call");
    // Pause for the debounced background lookup so the assertion observes the
    // settled post-failure UI instead of the immediate pre-request state.
    thread::sleep(Duration::from_secs(1));
    let screen = session.snapshot();

    assert!(screen.status_line_contains("INSERT "));
    assert!(screen.contains("    let _ = helper_sum("));
    assert!(!screen.contains("Signature Help"));
    assert!(!screen.message_line_contains("language server \"rust-analyzer\" is not in PATH"));
    assert!(!screen.message_line_contains("failed to inspect Cargo project"));

    session.send_escape().expect("leave insert mode");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify rapid retriggers still dismiss signature help promptly after the call closes.
#[test]
fn test_lsp_signature_help_closes_promptly_after_fast_retriggers() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

    session
        .send_text("jjjo")
        .expect("open line below helper_value");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("5:1")
        })
        .expect("wait for insert mode");

    session
        .send_text("    let _ = helper_sum(")
        .expect("type helper_sum call");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("Signature Help") && screen.contains("helper_sum(")
        })
        .expect("wait for signature-help popup");

    // Send multiple argument edits plus the closing delimiter in one burst so a
    // stale request would otherwise have time to keep the old popup visible.
    session.send_text("1, 2)").expect("finish call quickly");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            (1..=30).all(|row| !screen.row_contains(row, "Signature Help"))
        })
        .expect("signature-help popup should close promptly");

    session.exit_to_normal_mode(Duration::from_secs(2));
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
    // Warm up rust-analyzer before the completion request so the assertion only
    // exercises popup rendering instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

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

/// Verify signature help stays visible when the screen cannot fit both popups.
#[test]
fn test_lsp_signature_help_takes_priority_when_popup_space_is_tight() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session_with_config(
        ordex_bin(),
        &[main_rs],
        PtySessionConfig {
            rows: 8,
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
        .send_text("5Go")
        .expect("open line below local_value call");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("6:1")
        })
        .expect("wait for inserted line");

    session
        .send_text("    std::mem::swap(")
        .expect("type swap call");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.contains("Signature Help")
                && screen.contains("fn swap<")
                && !screen.contains("replace")
        })
        .expect("wait for signature help to win tight popup layout");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify signature help stays above completion when both popups are visible.
#[test]
fn test_lsp_signature_help_uses_opposite_side_from_completion_popup() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session_with_config(
        ordex_bin(),
        &[main_rs],
        PtySessionConfig {
            rows: 12,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

    session
        .send_text("5Go")
        .expect("open line below local_value call");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("6:1")
        })
        .expect("wait for inserted line");

    // Keeping the edited line near the middle of a short terminal leaves room
    // for completion below and forces signature help to render on the other side.
    session
        .send_text("    std::mem::swap(")
        .expect("type swap call");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            let signature_above = (1..=5).any(|row| screen.row_contains(row, "Signature Help"));
            let signature_below = (7..=10).any(|row| screen.row_contains(row, "Signature Help"));
            let completion_above = (1..=5).any(|row| {
                screen.row_contains(row, "swap")
                    || screen.row_contains(row, "replace")
                    || screen.row_contains(row, "function")
            });
            let completion_below = (7..=10).any(|row| {
                screen.row_contains(row, "swap")
                    || screen.row_contains(row, "replace")
                    || screen.row_contains(row, "function")
            });
            screen.row_contains(6, "std::mem::swap(")
                && ((signature_above && completion_below) || (completion_above && signature_below))
        })
        .expect("wait for separated completion and signature-help popups");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify fast typing stays responsive while background LSP work is active.
#[test]
fn test_lsp_insert_mode_stays_responsive_during_fast_typing() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(ordex_bin(), &[main_rs]).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    lsp_test_support::warm_up_helper_value_hover(&mut session);

    session
        .send_text("gg0i")
        .expect("enter insert mode at file start");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("1:1")
        })
        .expect("wait for insert mode");

    let fast_text = "zzzzzzzzzzzzzzzzzzzzzzzzzzzz";
    session
        .send_text(fast_text)
        .expect("type many characters quickly");
    session
        .wait_until(Duration::from_millis(500), |screen| {
            screen.row_contains(1, fast_text)
        })
        .expect("typed text should appear promptly");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify deleting back through one visible LSP popup keeps it below the edited line.
#[test]
fn test_lsp_completion_popup_stays_below_current_line_after_backspacing_prefix() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session_with_config(
        ordex_bin(),
        &[main_rs],
        PtySessionConfig {
            rows: 12,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");
    // Warm up rust-analyzer before the completion request so the assertion only
    // exercises popup rendering instead of startup analysis timing.
    lsp_test_support::warm_up_helper_value_hover(&mut session);
    session
        .send_text("gg0")
        .expect("return to file start after warmup");

    session.send_text("o").expect("open line below");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("2:1")
        })
        .expect("wait for first inserted line");
    session.send_enter().expect("insert another line");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.status_line_contains("INSERT ") && screen.status_line_contains("3:1")
        })
        .expect("wait for second inserted line");

    session
        .send_text("use std::")
        .expect("type initial std path trigger");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::")
                && (screen.row_contains(4, "┌")
                    || screen.row_contains(4, "alloc")
                    || screen.row_contains(5, "alloc"))
                && screen.contains("module")
        })
        .expect("wait for initial completion popup");
    // Type through the prefix in stages so the popup keeps one stable trigger
    // state while the test exercises the later backspace refresh behavior.
    session.send_text("allo").expect("narrow popup to allo");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::allo")
                && (screen.row_contains(4, "┌")
                    || screen.row_contains(4, "alloc")
                    || screen.row_contains(5, "alloc"))
        })
        .expect("wait for allo completion popup");
    session.send_text("c").expect("complete prefix to alloc");
    session
        .wait_until(Duration::from_secs(5), |screen| {
            screen.row_contains(3, "use std::alloc")
        })
        .expect("wait for alloc text");

    // Reproduce the reported edit sequence one step at a time so each backspace
    // settles its own popup refresh before the next character is sent.
    session.send_text(PTY_BACKSPACE).expect("delete c");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::allo") && screen.contains("alloc")
        })
        .expect("wait for allo completion popup");
    session.send_text(PTY_BACKSPACE).expect("delete o");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::all") && screen.contains("alloc")
        })
        .expect("wait for all completion popup");
    session.send_text("u").expect("type u");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::allu") && screen.contains("alloc")
        })
        .expect("wait for allu completion popup");
    session.send_text(PTY_BACKSPACE).expect("delete u");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::all") && screen.contains("alloc")
        })
        .expect("wait for restored all completion popup");
    session.send_text(PTY_BACKSPACE).expect("delete final l");
    session
        .wait_until(Duration::from_secs(10), |screen| {
            screen.row_contains(3, "use std::al")
                && (screen.row_contains(4, "┌")
                    || screen.row_contains(4, "alloc")
                    || screen.row_contains(5, "alloc"))
        })
        .expect("wait for final popup below current line");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
