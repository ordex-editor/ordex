use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

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
            s.status_line_contains("NORMAL |") && s.status_line_contains("1:1")
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
        "redraw output should not hide the cursor anymore"
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

#[test]
fn test_cursor_move_does_not_blank_row_before_repaint() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 40, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.status_line_contains("1:1")
        })
        .expect("initial frame rendered");

    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("cursor moved");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[1;1H                                        "),
        "renderer should not emit full-width space fills for content rows"
    );

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_mode_entry_keeps_real_cursor_visible() {
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
            s.status_line_contains("NORMAL |") && s.row_contains(1, "XYZ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("v").expect("enter visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL |")
        })
        .expect("visual mode entry rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[?25l"),
        "visual mode entry should not hide the cursor"
    );
    assert!(
        !snapshot.contains("\u{1b}[?25h"),
        "visual mode entry should not re-show the cursor"
    );
    assert!(
        snapshot.contains("\u{1b}[4m"),
        "visual mode entry should underline the active cursor cell"
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
            s.status_line_contains("NORMAL |") && s.row_contains(1, "XYZ")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("vl").expect("select XY in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL |")
        })
        .expect("visual mode rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[7mX"),
        "selection render should include reverse-video styling for the selected text"
    );
    assert!(
        !snapshot.contains("\u{1b}[?25l"),
        "visual mode redraw should not hide the cursor"
    );
    assert!(
        !snapshot.contains("\u{1b}[?25h"),
        "visual mode redraw should not re-show the cursor"
    );
    assert!(
        snapshot.contains("\u{1b}[4m"),
        "visual mode should underline the active cursor cell"
    );

    session.send_escape().expect("return to normal");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_motion_keeps_terminal_cursor_visible() {
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
            s.status_line_contains("NORMAL |") && s.row_contains(1, "abcd")
        })
        .expect("initial frame rendered");

    session.clear_transcript();
    session.send_text("vll").expect("move in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL |")
        })
        .expect("visual movement rendered");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[?25l"),
        "visual movement should not hide the cursor"
    );
    assert!(
        !snapshot.contains("\u{1b}[?25h"),
        "visual movement should not re-show the cursor"
    );
    assert!(
        snapshot.contains("\u{1b}[4m"),
        "visual movement should keep the active cursor cell underlined"
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
            s.status_line_contains("NORMAL |") && s.status_line_contains("1:1")
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
