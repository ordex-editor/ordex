//! Integration tests for navigation functionality (User Story 1)
//!
//! Tests vim-style navigation: hjkl, w/b word motions, and Ctrl+F/Ctrl+B/Ctrl+D/Ctrl+U scrolling.

use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_hjkl_character_navigation() {
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
        .expect("initial cursor");

    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("cursor at 1:2 after l");

    session.send_text("j").expect("move down");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("cursor at 2:2 after j");

    session.send_text("h").expect("move left");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:1"))
        .expect("cursor at 2:1 after h");

    session.send_text("k").expect("move up");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("cursor at 1:1 after k");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_word_navigation() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"one two_three, four\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("initial cursor");

    session.send_text("w").expect("word forward to two_three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:5"))
        .expect("cursor at two_three start");

    session.send_text("w").expect("word forward to four");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:16"))
        .expect("cursor at four start");

    session.send_text("b").expect("word backward to two_three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:5"))
        .expect("cursor returned to two_three start");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_page_navigation() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 80, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "line 01")
        })
        .expect("initial viewport");

    session.send_text("\u{6}").expect("ctrl-f page down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("6:1") && s.row_contains(1, "line 04")
        })
        .expect("paged down");

    session.send_text("\u{2}").expect("ctrl-b page up");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "line 01")
        })
        .expect("paged up");

    session.send_text("\u{4}").expect("ctrl-d half page down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("4:1") && s.row_contains(1, "line 02")
        })
        .expect("half paged down");

    session.send_text("\u{15}").expect("ctrl-u half page up");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "line 01")
        })
        .expect("half paged up");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_boundary_conditions() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\ncd").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("initial cursor");

    session.send_text("hk").expect("attempt up/left at origin");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("cursor stays at origin");

    session
        .send_text("G$l")
        .expect("go to last line and try move right past end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("cursor clamped at line end");

    session
        .send_text("j")
        .expect("attempt move down past last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("cursor stays on last line");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_multikey_g_navigation() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line one\nline two\nline three\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("initial cursor");

    // Move to line 2, column 3 then jump to first line while preserving column.
    session.send_text("jl").expect("line 2, col 2");
    session.send_text("l").expect("line 2, col 3");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:3"))
        .expect("cursor at line 2 col 3");

    session.send_text("gg").expect("go to first line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:3"))
        .expect("gg keeps column");

    session.send_text("g$").expect("go to line end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:8"))
        .expect("g$ moved to line end");

    session.send_text("g0").expect("go to line start");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("g0 moved to line start");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_find_till_and_repeat_navigation() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abca\nzaza\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("initial cursor");

    session.send_text("fa").expect("find next a");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:4"))
        .expect("cursor at found a");

    session.send_text(",").expect("repeat opposite direction");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("cursor returned to first a");

    session.send_text(";").expect("repeat original direction");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:4"))
        .expect("semicolon repeats original forward find");

    session
        .send_text(",,")
        .expect("repeat opposite direction twice");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("comma can be repeated in a row");

    session
        .send_text(";;")
        .expect("repeat base direction twice");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:4"))
        .expect("semicolon can be repeated in a row");

    session
        .send_text("0tb")
        .expect("line start then till before b");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("adjacent till keeps cursor in place");

    session
        .send_text("jfa")
        .expect("move down and find a on same line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("find on second line");

    session
        .send_text("k$fa")
        .expect("back up, go to line end, and try find missing a on line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:4"))
        .expect("line-bounded find should not cross to next line");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
