//! Integration tests for navigation functionality (User Story 1)
//!
//! Tests vim-style navigation: hjkl, w/b word motions, and Ctrl+F/Ctrl+B/Ctrl+D/Ctrl+U scrolling.

mod config_test_support;

use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_jump_history_back_and_forward_shortcuts() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 01\nline 02\nline 03\nline 04\nline 05")
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

    session.send_text("G").expect("jump to last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("5:1"))
        .expect("cursor at last line");

    session
        .send_text("\u{f}")
        .expect("jump backward through history");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("ctrl-o should return to the older jump");

    session
        .send_text("\t")
        .expect("jump forward through history");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("5:1"))
        .expect("tab should return to the newer jump");

    session
        .send_text("\u{f}")
        .expect("jump backward before clearing forward history");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("returned to the first jump");

    session.send_text("3G").expect("make a fresh jump");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("3:1"))
        .expect("cursor at line 3");

    session
        .send_text("\t")
        .expect("try jump forward after fresh jump");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3:1") && s.message_line_contains("Already at newest jump")
        })
        .expect("fresh jump should clear forward history");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
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

    session
        .send_text("w")
        .expect("word forward to punctuation word between identifiers");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:14"))
        .expect("cursor at punctuation word start");

    session.send_text("w").expect("word forward to four");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:16"))
        .expect("cursor at four start");

    session
        .send_text("b")
        .expect("word backward to punctuation word");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:14"))
        .expect("cursor returned to punctuation word start");

    session.send_text("b").expect("word backward to two_three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:5"))
        .expect("cursor returned to two_three start");

    session.send_escape().expect("leave visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after visual cancel");
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
        PtySessionConfig {
            cols: 80,
            rows: 8,
            ..Default::default()
        },
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
            s.status_line_contains("5:1") && s.row_contains(1, "line 04")
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
            s.status_line_contains("3:1") && s.row_contains(1, "line 01")
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
fn test_viewport_shortcuts() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let config = config_test_support::write_config(
        r#"
[editor]
scroll_margin = 1
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.resize(80, 10).expect("set terminal size");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "line 01")
        })
        .expect("initial viewport");

    session.send_text("10j").expect("move to line 11");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("11:1"))
        .expect("cursor reached line 11");

    session.send_text("zt").expect("align line to top");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11:1") && s.row_contains(2, "line 11")
        })
        .expect("zt should place the cursor line near the top margin");

    session.send_text("zz").expect("align line to center");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11:1") && s.row_contains(4, "line 11")
        })
        .expect("zz should place the cursor line near the viewport center");

    session.send_text("zb").expect("align line to bottom");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11:1") && s.row_contains(6, "line 11")
        })
        .expect("zb should place the cursor line near the bottom margin");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_viewport_line_scroll_shortcuts() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let config = config_test_support::write_config(
        r#"
[editor]
scroll_margin = 1
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.resize(80, 10).expect("set terminal size");

    session.send_text("10jzt").expect("place line 11 near top");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11:1") && s.row_contains(2, "line 11")
        })
        .expect("line 11 aligned near top margin");

    session.send_text("\u{5}").expect("ctrl-e scroll down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12:1") && s.row_contains(2, "line 12")
        })
        .expect("ctrl-e should nudge the cursor to stay inside the top margin");

    session.send_text("\u{19}").expect("ctrl-y scroll up");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12:1") && s.row_contains(3, "line 12")
        })
        .expect("ctrl-y should keep the cursor steady once it is back inside the margin");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_mode_viewport_line_scroll_shortcuts() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let config = config_test_support::write_config(
        r#"
[editor]
scroll_margin = 1
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.resize(80, 10).expect("set terminal size");

    session
        .send_text("10jztv")
        .expect("enter visual mode at line 11");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
                && s.status_line_contains("11:1")
                && s.row_contains(2, "line 11")
        })
        .expect("visual mode at aligned line");

    session
        .send_text("\u{5}")
        .expect("ctrl-e scroll down in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
                && s.status_line_contains("12:1")
                && s.row_contains(2, "line 12")
        })
        .expect("ctrl-e should keep the visual cursor inside the top margin");

    session
        .send_text("\u{19}")
        .expect("ctrl-y scroll up in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
                && s.status_line_contains("12:1")
                && s.row_contains(3, "line 12")
        })
        .expect("ctrl-y should leave the visual cursor unchanged once it fits the margin");

    session.send_escape().expect("leave visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after visual cancel");
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

#[test]
/// `ge` and `gE` should land on the previous word and WORD ends.
fn test_ge_and_g_e_move_to_previous_word_end() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"one two-three\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("initial cursor");

    session
        .send_text("ww")
        .expect("move to punctuation word before three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:8"))
        .expect("cursor at punctuation word");

    session.send_text("ge").expect("move to previous word end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:7"))
        .expect("ge should land on two");

    session
        .send_text("wgE")
        .expect("move back to three then previous WORD end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:3"))
        .expect("gE should land on one");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `gf` should open a bare filename token relative to the current buffer.
fn test_gf_opens_bare_filename_under_cursor() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("main.txt", "child.txt\n")
        .expect("write main file");
    tree.write_file("child.txt", "child buffer\n")
        .expect("write child file");
    let main_path = tree.path().join("main.txt");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_path.to_str().expect("utf8 main path")],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "child.txt"))
        .expect("main file opened");

    session.send_text("gf").expect("open file under cursor");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "child buffer")
        })
        .expect("gf should open child file");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `gF` should honor `:line:column` suffixes after opening the target file.
fn test_g_f_opens_file_target_at_line_and_column() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("main.txt", "child.txt:2:3\n")
        .expect("write main file");
    tree.write_file("child.txt", "alpha\nbeta line\ngamma\n")
        .expect("write child file");
    let main_path = tree.path().join("main.txt");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_path.to_str().expect("utf8 main path")],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "child.txt:2:3")
        })
        .expect("main file opened");

    session
        .send_text("gF")
        .expect("open file target with line and column");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("2:3") && s.row_contains(2, "beta line")
        })
        .expect("gF should open child file at line and column");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
