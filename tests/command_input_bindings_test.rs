use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":abc def").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abc def")
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
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text("/target").expect("enter search input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH |") && s.message_line_contains("/target")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":abc").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abc")
        })
        .expect("command input visible");

    // Left arrow as CSI sequence: ESC [ D
    session.send_text("\u{1b}[D").expect("left arrow");
    session.send_text("X").expect("insert in middle");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abXc")
        })
        .expect("left arrow should move cursor without cancelling command mode");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text("/foo bar").expect("enter search input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH |") && s.message_line_contains("/foo bar")
        })
        .expect("search input visible");

    // Alt+b as ESC-prefixed character.
    session.send_text("\u{1b}b").expect("alt-b");
    session
        .send_text("X")
        .expect("insert at moved word boundary");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH |") && s.message_line_contains("/foo Xbar")
        })
        .expect("alt-b should move cursor without cancelling search mode");

    session.send_escape().expect("cancel search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
        })
        .expect("back to normal mode");

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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":abc").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abc")
        })
        .expect("command input visible");

    // Send arrow-left bytes with an artificial delay between ESC and the rest
    // to mimic network/PTY jitter.
    session.send_text("\u{1b}").expect("arrow-left esc prefix");
    std::thread::sleep(Duration::from_millis(80));
    session.send_text("[D").expect("arrow-left suffix");
    session.send_text("X").expect("insert in middle");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abXc")
        })
        .expect("delayed arrow sequence should still be interpreted as Left");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":abcd").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abcd")
        })
        .expect("command input visible");

    // Burst of left arrows.
    session.send_text("\u{1b}[D").expect("left");
    session.send_text("\u{1b}[D").expect("left");
    session.send_text("\u{1b}[D").expect("left");

    // Stray ESC observed after burst on some terminals/SSH setups.
    session.send_text("\u{1b}").expect("stray esc");
    session.send_text("X").expect("type after stray esc");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":aXbcd")
        })
        .expect("stray esc after left burst should not cancel command mode");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":abcd").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abcd")
        })
        .expect("command input visible");

    session.send_text("\u{1b}[D").expect("left");
    session.send_text("\u{1b}[D").expect("left");

    // First Esc can be stray from key-release burst.
    session.send_text("\u{1b}").expect("first esc");
    session.send_text("\u{1b}").expect("second esc");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("COMMAND |") && s.message_line_contains(":wriXte")
        })
        .expect("one stray escape after left burst should not cancel command mode");

    session.send_escape().expect("cancel command");
    session.send_escape().expect("confirm cancel");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":write").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":write")
        })
        .expect("command input visible");

    // First left is normal.
    session.send_text("\u{1b}[D").expect("left");

    // Second left arrives split (ESC first, CSI tail delayed).
    session.send_text("\u{1b}").expect("left esc");
    std::thread::sleep(Duration::from_millis(80));
    session.send_text("[D").expect("left csi tail");
    session
        .send_text("X")
        .expect("insert after two left movements");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |")
                && s.message_line_contains(":wriXte")
                && !s.message_line_contains("[D")
        })
        .expect("split left sequence should move cursor, not insert literal [D");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":write").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":write")
        })
        .expect("command input visible");

    session.send_text("\u{1b}[D").expect("left");
    session.send_text("X").expect("insert after left");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |")
                && s.message_line_contains(":writXe")
                && !s.message_line_contains("[D")
        })
        .expect("single left should move once before insert");

    session.send_escape().expect("cancel command");
    session.send_escape().expect("confirm cancel");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
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
            s.status_line_contains("NORMAL |")
        })
        .expect("initial normal mode");

    session.send_text(":abc").expect("enter command input");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":abc")
        })
        .expect("command input visible");

    // Ctrl+A using CSI-u encoding (modifyOtherKeys / kitty protocol style).
    session.send_text("\u{1b}[97;5u").expect("ctrl-a as csi-u");
    session.send_text("X").expect("insert at input start");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":Xabc")
        })
        .expect("csi-u ctrl-a should keep command mode and move cursor to start");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
        })
        .expect("back to normal mode");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
