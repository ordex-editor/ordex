use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use test_utils::{
    PTY_BACKSPACE, PtySession, PtySessionConfig, spawn_lsp_session, spawn_lsp_session_with_config,
};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Wait until rust-analyzer can answer one helper-value hover in `main.rs`.
fn warm_up_helper_value_hover(session: &mut PtySession) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        session
            .send_text("/helper_value()")
            .expect("search for warmup symbol");
        session.send_enter().expect("confirm warmup search");
        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.status_line_contains("4:13")
            })
            .expect("cursor should land on the warmup helper_value call");
        session.send_text("K").expect("request warmup hover");
        if session
            .wait_until(Duration::from_secs(4), |screen| {
                screen.contains("Hover") && screen.contains("fn helper_value() -> i32")
            })
            .is_ok()
        {
            session.send_text("j").expect("dismiss warmup hover");
            session
                .wait_until(Duration::from_secs(2), |screen| {
                    screen.row_contains(5, "    let _ = local_value();")
                        && screen.status_line_contains("5:13")
                })
                .expect("warmup hover should dismiss before moving down");
            return;
        }
        // Retry the hover request until rust-analyzer finishes enough analysis
        // to answer symbol lookups reliably for the test workspace.
        assert!(Instant::now() < deadline, "warmup hover should succeed");
        thread::sleep(Duration::from_millis(100));
    }
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
    warm_up_helper_value_hover(&mut session);
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
        .wait_until(Duration::from_secs(45), |screen| {
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
    warm_up_helper_value_hover(&mut session);
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
        .wait_until(Duration::from_secs(45), |screen| {
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
    // Warm up rust-analyzer before the completion request so the assertion only
    // exercises popup rendering instead of startup analysis timing.
    warm_up_helper_value_hover(&mut session);
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
        .wait_until(Duration::from_secs(45), |screen| {
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
    warm_up_helper_value_hover(&mut session);
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
        .send_text("use std::alloc")
        .expect("type initial std alloc path");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.contains("alloc") && screen.contains("module")
        })
        .expect("wait for initial completion popup");

    // Reproduce the reported edit sequence one step at a time so each backspace
    // settles its own popup refresh before the next character is sent.
    session.send_text(PTY_BACKSPACE).expect("delete c");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.row_contains(3, "use std::allo") && screen.contains("alloc")
        })
        .expect("wait for allo completion popup");
    session.send_text(PTY_BACKSPACE).expect("delete o");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.row_contains(3, "use std::all") && screen.contains("alloc")
        })
        .expect("wait for all completion popup");
    session.send_text("u").expect("type u");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.row_contains(3, "use std::allu") && screen.contains("alloc")
        })
        .expect("wait for allu completion popup");
    session.send_text(PTY_BACKSPACE).expect("delete u");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.row_contains(3, "use std::all") && screen.contains("alloc")
        })
        .expect("wait for restored all completion popup");
    session.send_text(PTY_BACKSPACE).expect("delete final l");
    session
        .wait_until(Duration::from_secs(45), |screen| {
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
