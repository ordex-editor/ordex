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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
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
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
        })
        .expect("command mode rendered");

    session
        .read_available()
        .expect("collect command transcript");
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
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
        })
        .expect("search mode rendered");

    session.read_available().expect("collect search transcript");
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("j").expect("move down");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:1"))
        .expect("cursor moved");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    let raw = snapshot.raw();

    // Vertical motion should stay a small redraw: only the old/new active rows
    // plus the status line change, and the terminal should not restart from the
    // top-left with a full-frame repaint.
    assert!(
        raw.contains("\u{1b}[?2026h"),
        "vertical cursor movement should begin a synchronized update frame"
    );
    assert!(
        raw.contains("\u{1b}[?2026l"),
        "vertical cursor movement should end the synchronized update frame"
    );
    assert!(
        raw.contains("\u{1b}[?25l"),
        "vertical cursor movement should hide the cursor during the multi-row gutter update"
    );
    assert!(
        raw.contains("\u{1b}[?25h"),
        "vertical cursor movement should restore the cursor after the gutter update"
    );
    assert_eq!(
        raw.matches("\u{1b}[K").count(),
        3,
        "vertical cursor movement should only clear the old row, new row, and status line"
    );
    assert!(
        raw.contains("abc") && raw.contains("def"),
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "XYZ")
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "XYZ")
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abcd")
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
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
