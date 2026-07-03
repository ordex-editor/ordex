use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree, wait_for_initial_render};

// NOTE: the tests with jitter delay are flaky in the macOS CI, so decrease the
// delay on macOS to reduce flakyness.
// TODO: check if it is still needed to be a different value on macOS.
#[cfg(target_os = "macos")]
const JITTER_DELAY: u64 = 0;
#[cfg(not(target_os = "macos"))]
const JITTER_DELAY: u64 = 30;

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Verify bracketed paste inserts only the first line into Command mode.
#[test]
fn test_command_mode_bracketed_paste_uses_first_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Start Command mode before sending the bracketed-paste payload.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");
    session.send_text(":").expect("enter command mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
        })
        .expect("command mode active");

    // Command/Search prompts are single-line, so only the first pasted line should appear.
    session
        .send_raw_bytes(b"\x1b[200~write\nquit\x1b[201~")
        .expect("send command paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.message_line_contains(":write")
                && !s.message_line_contains("quit")
        })
        .expect("command prompt should keep only the first line");

    session.send_escape().expect("cancel command");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify bracketed paste inserts only the first line into Search mode.
#[test]
fn test_search_mode_bracketed_paste_uses_first_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Start Search mode before sending the bracketed-paste payload.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");
    session.send_text("/").expect("enter search mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
        })
        .expect("search mode active");

    // Search input stays single-line, so later pasted lines must be discarded.
    session
        .send_raw_bytes(b"\x1b[200~beta\nalpha\x1b[201~")
        .expect("send search paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/beta")
                && !s.message_line_contains("alpha")
        })
        .expect("search prompt should keep only the first line");

    session.send_escape().expect("cancel search");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_ctrl_editing_bindings() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":abc def").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abc def")
        })
        .expect("command input visible");

    session.send_text("\u{1}").expect("ctrl-a");
    session.send_text("X").expect("insert at start");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains(":Xabc def")
        })
        .expect("ctrl-a moved cursor to start");

    session.send_text("\u{5}").expect("ctrl-e");
    session.send_text("Y").expect("insert at end");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains(":Xabc defY")
        })
        .expect("ctrl-e moved cursor to end");

    session.send_text("\u{17}").expect("ctrl-w");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains(":Xabc"))
        .expect("ctrl-w deleted previous word");

    session.send_text("\u{15}").expect("ctrl-u");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains(":"))
        .expect("ctrl-u deleted to start of input");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_mode_ctrl_editing_bindings() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"target\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text("/target").expect("enter search input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.message_line_contains("/target")
        })
        .expect("search input visible");

    session.send_text("\u{1}").expect("ctrl-a");
    session.send_text("\u{4}").expect("ctrl-d at start");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("/arget")
        })
        .expect("ctrl-d deleted char under cursor");

    session.send_text("\u{5}").expect("ctrl-e");
    session.send_text("\u{8}").expect("ctrl-h");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("/arge"))
        .expect("ctrl-h deleted char backward");

    session.send_text("\u{b}").expect("ctrl-k");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("/arge"))
        .expect("ctrl-k at end is no-op");

    session.send_escape().expect("cancel search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_treats_gd_as_plain_input() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_rs.to_str().expect("utf8 fixture path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session.send_text(":gd").expect("type command text");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ")
                && screen.message_line_contains(":gd")
                && screen.row_contains(1, "use workspace_one")
        })
        .expect("command input should keep literal gd text");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("back to normal mode");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_arrow_left_does_not_cancel_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":abc").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abc")
        })
        .expect("command input visible");

    // Left arrow as CSI sequence: ESC [ D
    session.send_text("\u{1b}[D").expect("left arrow");
    session.send_text("X").expect("insert in middle");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abXc")
        })
        .expect("left arrow should move cursor without cancelling command mode");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_mode_alt_b_does_not_cancel_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text("/foo bar").expect("enter search input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.message_line_contains("/foo bar")
        })
        .expect("search input visible");

    // Alt+b as ESC-prefixed character.
    session.send_text("\u{1b}b").expect("alt-b");
    session
        .send_text("X")
        .expect("insert at moved word boundary");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.message_line_contains("/foo Xbar")
        })
        .expect("alt-b should move cursor without cancelling search mode");

    session.send_escape().expect("cancel search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_history_uses_full_and_prefix_traversal() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":alpha").expect("enter alpha");
    session.send_enter().expect("submit alpha");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("Unknown command: alpha")
        })
        .expect("alpha recorded");

    session.send_text(":alpha").expect("enter duplicate alpha");
    session.send_enter().expect("submit duplicate alpha");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("Unknown command: alpha")
        })
        .expect("duplicate alpha handled");

    session.send_text(":beta").expect("enter beta");
    session.send_enter().expect("submit beta");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("Unknown command: beta")
        })
        .expect("beta recorded");

    session.send_text(":").expect("open command prompt");
    session.send_text("\u{10}").expect("ctrl-p");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":beta")
        })
        .expect("ctrl-p should recall latest command");

    session.send_text("\u{10}").expect("ctrl-p again");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains(":alpha")
        })
        .expect("ctrl-p should skip adjacent duplicate");

    session.send_text("\u{e}").expect("ctrl-n");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains(":beta"))
        .expect("ctrl-n should move forward");

    session.send_text("\u{e}").expect("ctrl-n restore");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line().is_some_and(|line| line.trim_end() == ":")
        })
        .expect("ctrl-n should restore original prompt");

    session.send_text("a").expect("type prefix");
    session.send_text("\u{1b}[A").expect("up arrow");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains(":alpha")
        })
        .expect("up arrow should use prefix matching");

    session.send_text("X").expect("edit recalled entry");
    session.send_text("\u{1b}[B").expect("down arrow");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains(":alphaX")
        })
        .expect("editing should reset traversal");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_mode_history_stays_separate_from_commands() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta gamma\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":alpha").expect("record command entry");
    session.send_enter().expect("submit command entry");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("Unknown command: alpha")
        })
        .expect("command recorded");

    session.send_text("/gamma").expect("search gamma");
    session.send_enter().expect("submit search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta gamma")
        })
        .expect("gamma search executed");

    session.send_text("/gamma").expect("repeat gamma");
    session.send_enter().expect("submit repeated search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("repeated search handled");

    session.send_text("/beta").expect("search beta");
    session.send_enter().expect("submit beta search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("beta search executed");

    session.send_text("/").expect("open search prompt");
    session.send_text("\u{10}").expect("ctrl-p");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.message_line_contains("/beta")
        })
        .expect("search history should recall beta");

    session.send_text("\u{10}").expect("ctrl-p again");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("/gamma")
        })
        .expect("search history should skip adjacent duplicate");

    session.send_text("\u{1b}[B").expect("down arrow");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("/"))
        .expect("down arrow should restore original empty search");

    session.send_text("ga").expect("type prefix");
    session.send_text("\u{1b}[A").expect("up arrow");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("/gamma")
        })
        .expect("up arrow should use search prefix");

    session.send_escape().expect("cancel search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":").expect("open command prompt");
    session.send_text("\u{10}").expect("ctrl-p command history");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":alpha")
        })
        .expect("command history should remain separate");

    session.send_escape().expect("cancel command");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_delayed_arrow_sequence_does_not_cancel_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":abc").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abc")
        })
        .expect("command input visible");

    // Send arrow-left bytes with an artificial delay between ESC and the rest
    // to mimic network/PTY jitter. The delay must stay well below the editor's
    // 50 ms escape-sequence timeout so the sequence is always reassembled, even
    // when CI scheduling adds latency on top of the explicit sleep.
    session.send_text("\u{1b}").expect("arrow-left esc prefix");
    std::thread::sleep(Duration::from_millis(JITTER_DELAY));
    session.send_text("[D").expect("arrow-left suffix");
    session.send_text("X").expect("insert in middle");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abXc")
        })
        .expect("delayed arrow sequence should still be interpreted as Left");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_stray_escape_after_left_burst_does_not_cancel() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":abcd").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abcd")
        })
        .expect("command input visible");

    #[cfg(target_os = "macos")]
    {
        // Send the left-arrow burst, stray ESC, and follow-up character as a
        // single raw write so the editor receives all bytes in one read and
        // processes them with no wall-clock gap between the last arrow and the
        // stray ESC. This eliminates the scheduling-induced race that caused the
        // suppression window to expire before the stray ESC was processed on
        // loaded CI runners.
        session
            .send_raw_bytes(b"\x1b[D\x1b[D\x1b[D\x1bX")
            .expect("left burst, stray esc, then type X");
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Burst of left arrows.
        session.send_text("\u{1b}[D").expect("left");
        session.send_text("\u{1b}[D").expect("left");
        session.send_text("\u{1b}[D").expect("left");

        // Stray ESC observed after burst on some terminals/SSH setups.
        session.send_text("\u{1b}").expect("stray esc");
        session.send_text("X").expect("type after stray esc");
    }

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":aXbcd")
        })
        .expect("stray esc after left burst should not cancel command mode");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_second_escape_after_left_burst_cancels() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":abcd").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abcd")
        })
        .expect("command input visible");

    session.send_text("\u{1b}[D").expect("left");
    session.send_text("\u{1b}[D").expect("left");

    // First Esc can be stray from key-release burst.
    session.send_text("\u{1b}").expect("first esc");
    session.send_text("\u{1b}").expect("second esc");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("second esc should cancel command mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_one_stray_escape_after_left_burst_does_not_cancel() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":write").expect("enter command input");
    session.send_text("\u{1b}[D").expect("left");
    session.send_text("\u{1b}[D").expect("left");

    // One stray ESC from key-release/burst behavior.
    session.send_text("\u{1b}").expect("stray esc 1");
    session.send_text("X").expect("type after burst");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":wriXte")
        })
        .expect("one stray escape after left burst should not cancel command mode");

    session.send_escape().expect("cancel command");
    session.send_escape().expect("confirm cancel");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_split_left_sequence_does_not_insert_literal_csi() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":write").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":write")
        })
        .expect("command input visible");

    // First left is normal.
    session.send_text("\u{1b}[D").expect("left");

    // Second left arrives split (ESC first, CSI tail delayed).
    session.send_text("\u{1b}").expect("left esc");
    std::thread::sleep(Duration::from_millis(JITTER_DELAY));
    session.send_text("[D").expect("left csi tail");
    session
        .send_text("X")
        .expect("insert after two left movements");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.message_line_contains(":wriXte")
                && !s.message_line_contains("[D")
        })
        .expect("split left sequence should move cursor, not insert literal [D");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_single_left_moves_before_insert() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":write").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":write")
        })
        .expect("command input visible");

    session.send_text("\u{1b}[D").expect("left");
    session.send_text("X").expect("insert after left");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.message_line_contains(":writXe")
                && !s.message_line_contains("[D")
        })
        .expect("single left should move once before insert");

    session.send_escape().expect("cancel command");
    session.send_escape().expect("confirm cancel");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_mode_csi_u_ctrl_a_does_not_cancel_prompt() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial normal mode");

    session.send_text(":abc").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":abc")
        })
        .expect("command input visible");

    // Ctrl+A using CSI-u encoding (modifyOtherKeys / kitty protocol style).
    session.send_text("\u{1b}[97;5u").expect("ctrl-a as csi-u");
    session.send_text("X").expect("insert at input start");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":Xabc")
        })
        .expect("csi-u ctrl-a should keep command mode and move cursor to start");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_completion_tab_cycles_and_restores_typed_prefix() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);

    session.send_text(":wr").expect("enter partial command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ")
                && screen.message_line_contains(":wr")
                && screen.contains("write")
        })
        .expect("auto command completion should appear");

    session.send_text("\t").expect("cycle completion forward");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ") && screen.message_line_contains(":write")
        })
        .expect("tab should preview the first completion");

    session.send_text("\t").expect("cycle back to none");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ") && screen.message_line_contains(":wr")
        })
        .expect("cycling past the last entry should restore the typed prefix");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_command_completion_completes_edit_path_arguments() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("state/file.txt", "demo\n")
        .expect("write directory");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);

    session.send_text(":e ").expect("enter edit path prompt");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ") && screen.contains("state/")
        })
        .expect("path completion popup should appear");

    session
        .send_text("\t")
        .expect("cycle path completion forward");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ")
                && screen.message_line_contains(":e state")
                && screen.contains("state/")
                && !screen.contains("file.txt")
        })
        .expect("directory completion should preview the directory name without adding a slash");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Verify `:e ~/` command completion resolves HOME-backed candidates.
fn test_command_completion_completes_edit_home_tilde_path_arguments() {
    let tree = TempTree::new().expect("create temp tree");
    let home = tree.path().join("home-user");
    std::fs::create_dir_all(home.join("alpha")).expect("create home subtree");

    let mut config = PtySessionConfig {
        current_dir: Some(tree.path().to_path_buf()),
        ..Default::default()
    };
    config
        .env
        .push(("HOME".to_string(), home.to_string_lossy().into_owned()));

    // Run from the fixture root so completion must rely on HOME rather than cwd.
    let mut session = PtySession::spawn(ordex_bin(), &[], config).expect("spawn ordex");

    wait_for_initial_render(&mut session);

    session
        .send_text(":e ~/")
        .expect("enter edit home path prompt");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("COMMAND ") && screen.contains("alpha/")
        })
        .expect("home path completion popup should appear");

    session
        .send_text("\t")
        .expect("cycle home path completion forward");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ")
                && screen.message_line_contains(":e ~/alpha")
                && screen.contains("alpha/")
        })
        .expect("home directory completion should preserve tilde prompt preview");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Verify `:w ~/` command completion resolves HOME-backed candidates.
fn test_command_completion_completes_write_home_tilde_path_arguments() {
    let tree = TempTree::new().expect("create temp tree");
    let home = tree.path().join("home-user");
    std::fs::create_dir_all(home.join("drafts")).expect("create home subtree");

    let mut config = PtySessionConfig {
        current_dir: Some(tree.path().to_path_buf()),
        ..Default::default()
    };
    config
        .env
        .push(("HOME".to_string(), home.to_string_lossy().into_owned()));

    // Run from the fixture root so completion must rely on HOME rather than cwd.
    let mut session = PtySession::spawn(ordex_bin(), &[], config).expect("spawn ordex");

    wait_for_initial_render(&mut session);

    session
        .send_text(":w ~/")
        .expect("enter write home path prompt");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("COMMAND ") && screen.contains("drafts/")
        })
        .expect("home path completion popup should appear");

    session
        .send_text("\t")
        .expect("cycle home path completion forward");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ")
                && screen.message_line_contains(":w ~/drafts")
                && screen.contains("drafts/")
        })
        .expect("write completion should preserve tilde prompt preview");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("back to normal mode");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
