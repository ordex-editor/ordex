use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

const BOGSTER_SELECTION_BG_ESCAPE: &str = "\u{1b}[48;5;237m";

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_render_does_not_toggle_cursor_visibility_during_frame_draw() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:2"))
        .expect("cursor moved");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[?25l"),
        "ordinary redraw output should not hide the cursor"
    );
    assert!(
        !snapshot.contains("\u{1b}[?25h"),
        "redraw output should not re-show the cursor anymore"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that insert-mode entry requests a beam cursor and `Esc` restores block mode.
#[test]
fn test_insert_mode_switches_to_beam_and_escape_restores_block_cursor() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .expect("insert mode rendered");

    session.read_available().expect("collect insert transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[6 q"),
        "insert mode should request a beam cursor"
    );

    session.clear_transcript();
    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored");

    session.read_available().expect("collect normal transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[2 q"),
        "leaving insert mode should restore a block cursor"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that command and search prompts keep the terminal in beam-cursor mode.
#[test]
fn test_command_and_search_modes_request_beam_cursor() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text(":").expect("enter command mode");
    // Wait until both the COMMAND status label and the beam-cursor escape are
    // present in the raw transcript.  The beam sequence is emitted after the
    // status line in the same render batch, so a partial PTY read that contains
    // "COMMAND " but not yet ESC[6 q would cause a spurious failure if the
    // condition only checked the status label.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.contains("\u{1b}[6 q")
        })
        .expect("command mode rendered with beam cursor");

    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[6 q"),
        "command mode should request a beam cursor"
    );

    session.send_escape().expect("leave command mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored");

    session.clear_transcript();
    session.send_text("/").expect("enter search mode");
    // Wait until both the SEARCH status label and the beam-cursor escape are
    // present in the raw transcript, for the same reason as the command-mode
    // wait above.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.contains("\u{1b}[6 q")
        })
        .expect("search mode rendered with beam cursor");

    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[6 q"),
        "search mode should request a beam cursor"
    );

    session.send_escape().expect("leave search mode");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that terminal cleanup restores a block cursor after exiting from a beam mode.
#[test]
fn test_shutdown_restores_block_cursor_after_beam_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session
        .send_text(":q")
        .expect("enter command mode with quit command");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[6 q"),
        "command mode should request a beam cursor before shutdown"
    );
    assert!(
        snapshot.contains("\u{1b}[2 q"),
        "terminal shutdown should restore a block cursor"
    );
}

#[test]
fn test_full_redraw_clears_rows_without_full_width_space_fills() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 40,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("v").expect("enter visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("visual mode rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[K"),
        "full redraws should clear rows with ANSI line clears"
    );
    assert!(
        !snapshot.contains("\u{1b}[1;1H                                        "),
        "renderer should not emit full-width space fills for content rows"
    );

    session.send_escape().expect("leave visual mode");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_same_line_cursor_move_does_not_restart_full_redraw_from_top_left() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 40,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:2"))
        .expect("cursor moved");

    session
        .read_available()
        .expect("collect cursor-move transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[1;1H"),
        "same-line cursor movement should not restart a full redraw from the top-left corner"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that ordinary vertical motion avoids content rewrites while staying atomic.
#[test]
fn test_vertical_cursor_move_does_not_restart_full_redraw_from_top_left() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 40,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("j").expect("move down");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:1"))
        .expect("cursor moved");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    let raw = snapshot.raw();

    // Vertical motion should stay a small redraw: only the old/new active rows
    // plus the status line change, and the terminal should not restart from the
    // top-left with a full-frame repaint.
    //
    // Extract only the first synchronized-update frame so that assertions are
    // not sensitive to additional render frames that may follow (e.g. when the
    // editor re-renders under load before the test reads the PTY buffer).
    let frame_start = raw
        .find("\u{1b}[?2026h")
        .expect("vertical cursor movement should begin a synchronized update frame");
    let frame_end_offset = raw[frame_start..]
        .find("\u{1b}[?2026l")
        .expect("vertical cursor movement should end the synchronized update frame");
    // Include the closing BSU escape in the frame slice so assertions about
    // both boundaries can be checked on a single substring.
    let frame = &raw[frame_start..frame_start + frame_end_offset + "\u{1b}[?2026l".len()];

    assert!(
        frame.contains("\u{1b}[?25l"),
        "vertical cursor movement should hide the cursor during the multi-row gutter update"
    );
    assert!(
        frame.contains("\u{1b}[?25h"),
        "vertical cursor movement should restore the cursor after the gutter update"
    );
    assert_eq!(
        frame.matches("\u{1b}[K").count(),
        3,
        "vertical cursor movement should only clear the old row, new row, and status line"
    );
    assert!(
        frame.contains("abc") && frame.contains("def"),
        "vertical cursor movement should repaint only the affected content rows"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_mode_entry_uses_selection_background_without_inline_cursor_styling() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"XYZ\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "XYZ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("v").expect("enter visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("visual mode entry rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "visual mode entry should tint the selected cursor cell background"
    );
    assert!(
        !snapshot.contains("\u{1b}[4m"),
        "visual mode entry should no longer underline the active cursor cell"
    );
    assert!(
        !snapshot.contains("\u{1b}[7m"),
        "visual mode entry should no longer rely on reverse-video selection"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_selection_uses_real_cursor_in_render_output() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"XYZ\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "XYZ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("vl").expect("select XY in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("visual mode rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "selection render should include the paler selection background escape"
    );
    assert!(
        !snapshot.contains("\u{1b}[7m"),
        "visual selections should no longer use reverse-video styling"
    );
    assert!(
        !snapshot.contains("\u{1b}[4m"),
        "visual mode should no longer underline the active cursor cell"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_block_selection_uses_selection_background_in_render_output() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd\na\nabc\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session
        .send_text("l\u{16}jjl")
        .expect("create block selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-BLOCK ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("visual block mode rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "block selection render should include the selection background escape"
    );
    assert!(
        !snapshot.contains("\u{1b}[7m"),
        "block selections should not use reverse-video styling"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_motion_keeps_selection_background_without_cursor_underline() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("vll").expect("move in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("visual movement rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "visual movement should keep the selection background on the active range"
    );
    assert!(
        !snapshot.contains("\u{1b}[4m"),
        "visual movement should no longer underline the active cursor cell"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_normal_mode_uses_terminal_cursor_on_empty_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\ntext\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial frame rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("1 \u{1b}[7m "),
        "normal mode should no longer rely on an inline highlighted placeholder cursor"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_sequence_popup_hides_cursor_when_overlay_covers_it() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 12,
            rows: 7,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("g").expect("start g sequence");
    session
        .wait_until(Duration::from_secs(2), |s| s.contains("\u{1b}[?25l"))
        .expect("popup should hide cursor when it covers the cursor cell");

    session.send_escape().expect("cancel sequence");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_full_redraw_hides_and_restores_cursor_within_one_frame() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("v").expect("enter visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("visual mode rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[?2026h"),
        "full redraws should begin a synchronized update frame"
    );
    assert!(
        snapshot.contains("\u{1b}[?2026l"),
        "full redraws should end the synchronized update frame"
    );
    assert!(
        snapshot.contains("\u{1b}[?25l"),
        "full redraws should hide the cursor before repainting"
    );
    assert!(
        snapshot.contains("\u{1b}[?25h"),
        "full redraws should restore the cursor after repainting"
    );

    session.send_escape().expect("leave visual mode");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_vi_quote_shows_selection_immediately_when_cursor_does_not_move() {
    // Regression: when the cursor is already on the last character before the
    // closing quote, `vi"` must display the full inner selection immediately
    // without requiring any subsequent cursor movement.
    //
    // Buffer: `"hello"`.  Move cursor to `o` (the last char before `"`), then
    // type `vi"`.  The cursor stays on `o`, but the anchor jumps to `h`, so
    // the entire `hello` span must be highlighted.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"hello\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"hello\"")
        })
        .expect("initial frame rendered");

    // Move to `o` (4 steps right, index 4 — last char before closing `"`).
    session
        .send_text("llll")
        .expect("position cursor on last inner char");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:5"))
        .expect("cursor at column 5");

    session.clear_transcript();
    session.send_text("vi\"").expect("select inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("vi\" must show selection highlight immediately, even without cursor movement");

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_vi_single_quote_shows_selection_immediately_when_cursor_does_not_move() {
    // Same regression as the double-quote variant: `vi'` on the last inner
    // char must show the selection right away.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"'hello'").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "'hello'")
        })
        .expect("initial frame rendered");

    session
        .send_text("llll")
        .expect("position cursor on last inner char");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:5"))
        .expect("cursor at column 5");

    session.clear_transcript();
    session.send_text("vi'").expect("select inner single-quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("vi' must show selection highlight immediately, even without cursor movement");

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_vi_quote_shows_selection_when_cursor_on_first_inner_char() {
    // When the cursor is already on the first inner char after `"`, `vi"` must
    // move the cursor to the last inner char and show the full span.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"hello\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"hello\"")
        })
        .expect("initial frame rendered");

    // Move to `h` (index 1 — first inner char).
    session
        .send_text("l")
        .expect("position cursor on first inner char");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:2"))
        .expect("cursor at column 2");

    session.clear_transcript();
    session.send_text("vi\"").expect("select inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("vi\" must show full inner selection when cursor starts on first inner char");

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_vi_quote_shows_selection_for_single_char_string_cursor_on_closing_quote() {
    // Edge case: `"x"` with cursor on the closing `"`.  `vi"` must select
    // just `x` and show the highlight immediately.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"x\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"x\"")
        })
        .expect("initial frame rendered");

    // Move to closing `"` (index 2).
    session
        .send_text("ll")
        .expect("position cursor on closing quote");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:3"))
        .expect("cursor at column 3");

    session.clear_transcript();
    session
        .send_text("vi\"")
        .expect("select inner quote of single-char string");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("vi\" on closing quote of single-char string must show the selection");

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_va_quote_shows_selection_immediately_when_cursor_does_not_move() {
    // `va"` on the closing quote itself keeps the cursor on the closing `"` but
    // sets the anchor to the opening `"` — selection must appear immediately.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"hello\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"hello\"")
        })
        .expect("initial frame rendered");

    // Move to closing `"` (index 6).
    session
        .send_text("llllll")
        .expect("position cursor on closing quote");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:7"))
        .expect("cursor at column 7");

    session.clear_transcript();
    session.send_text("va\"").expect("select around quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("va\" on closing quote must show full around-quote selection immediately");

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_line_visual_mode_highlights_empty_interior_line() {
    // Selecting an empty line between two non-empty lines with linewise visual
    // must emit the selection background for that line.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abc")
        })
        .expect("initial frame rendered");

    // Navigate to the empty second line, then enter linewise visual on it alone.
    session.send_text("j").expect("move to empty line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/3:1"))
        .expect("cursor on empty interior line");

    session.clear_transcript();
    session
        .send_text("V")
        .expect("linewise visual on empty line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-LINE ")
        })
        .expect("linewise visual mode entered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "linewise visual on an empty interior line should emit the selection background"
    );
    assert!(
        !snapshot.contains("\u{1b}[7m"),
        "linewise selection should not use reverse-video styling"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_line_visual_mode_highlights_empty_last_line() {
    // Selecting the empty last line of a file with linewise visual must emit the
    // selection background for that line.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abc")
        })
        .expect("initial frame rendered");

    // Navigate to the empty second line, then enter linewise visual on it alone.
    session.send_text("j").expect("move to empty last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:1"))
        .expect("cursor on empty last line");

    session.clear_transcript();
    session
        .send_text("V")
        .expect("linewise visual on empty last line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-LINE ")
        })
        .expect("linewise visual mode entered on empty last line");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "linewise visual on an empty trailing line should emit the selection background"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_char_visual_mode_highlights_empty_interior_line() {
    // Entering characterwise visual on an empty line must emit the selection
    // background for that line.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abc")
        })
        .expect("initial frame rendered");

    // Navigate to the empty second line, then enter characterwise visual on it.
    session.send_text("j").expect("move to empty line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/3:1"))
        .expect("cursor on empty interior line");

    session.clear_transcript();
    session
        .send_text("v")
        .expect("characterwise visual on empty line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("characterwise visual mode entered on empty line");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "characterwise visual on an empty interior line should emit the selection background"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_char_visual_mode_empty_line_only() {
    // A characterwise selection anchored on an empty line must highlight that line.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("initial frame rendered");

    // Enter characterwise visual on the single empty line.
    session
        .send_text("v")
        .expect("enter characterwise visual on empty line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("characterwise visual mode entered on empty line");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "characterwise visual on a single empty line should emit the selection background"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_block_visual_mode_does_not_highlight_empty_line() {
    // Block-mode selections intentionally skip empty lines because there are no
    // real buffer columns to select. Confirming that the selection background
    // appears on non-empty lines ensures the block path is unaffected by the
    // empty-line fix.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abc")
        })
        .expect("initial frame rendered");

    // Create a block selection on column 0 spanning all three lines.
    session
        .send_text("\u{16}jj")
        .expect("block select column 0 of three lines");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-BLOCK ") && s.contains(BOGSTER_SELECTION_BG_ESCAPE)
        })
        .expect("block visual mode rendered with selection on non-empty lines");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    // The non-empty lines must show the selection background.
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "block selection on non-empty lines should emit the selection background"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_line_visual_multiple_consecutive_empty_lines_are_highlighted() {
    // Selecting only the consecutive empty lines in a linewise visual must emit
    // the selection background for each empty line.
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"a\n\n\n\nb\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "a")
        })
        .expect("initial frame rendered");

    // Navigate to the first empty line, then select the three consecutive empty lines.
    session.send_text("j").expect("move to first empty line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/5:1"))
        .expect("cursor on first empty line");

    session.clear_transcript();
    session
        .send_text("V2j")
        .expect("linewise select three consecutive empty lines");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-LINE ")
        })
        .expect("linewise visual over consecutive empty lines entered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains(BOGSTER_SELECTION_BG_ESCAPE),
        "linewise selection covering consecutive empty lines should emit the selection background"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
