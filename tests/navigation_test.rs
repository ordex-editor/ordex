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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("initial cursor");

    session.send_text("G").expect("jump to last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("5/5:1"))
        .expect("cursor at last line");

    session
        .send_text("\u{f}")
        .expect("jump backward through history");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("ctrl-o should return to the older jump");

    session
        .send_text("\t")
        .expect("jump forward through history");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("5/5:1"))
        .expect("tab should return to the newer jump");

    session
        .send_text("\u{f}")
        .expect("jump backward before clearing forward history");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("returned to the first jump");

    session.send_text("3G").expect("make a fresh jump");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("3/5:1"))
        .expect("cursor at line 3");

    session
        .send_text("\t")
        .expect("try jump forward after fresh jump");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3/5:1") && s.message_line_contains("Already at newest jump")
        })
        .expect("fresh jump should clear forward history");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `:A` should switch to an existing corresponding header file in the same directory.
fn test_alternate_command_opens_corresponding_header_file() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.c", "int main(void) { return 0; }\n")
        .expect("write source");
    tree.write_file("src/main.h", "#pragma once\n#define VALUE 1\n")
        .expect("write header");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[tree.path().join("src/main.c").to_str().expect("utf8 path")],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "int main(void) { return 0; }")
        })
        .expect("source file should render");

    // Execute the Ex command and wait for header content to become visible.
    session.send_text(":A").expect("enter alternate command");
    session.send_enter().expect("execute alternate command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.tab_line_contains("main.h")
                && s.row_trimmed_ends_with(1, "#pragma once")
                && !s.message_line_contains("No corresponding file")
        })
        .expect("alternate command should switch to header");

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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("initial cursor");

    session.send_text("l").expect("move right");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:2"))
        .expect("cursor at 1:2 after l");

    session.send_text("j").expect("move down");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:2"))
        .expect("cursor at 2:2 after j");

    session.send_text("h").expect("move left");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:1"))
        .expect("cursor at 2:1 after h");

    session.send_text("k").expect("move up");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:1"))
        .expect("initial cursor");

    session.send_text("w").expect("word forward to two_three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:5"))
        .expect("cursor at two_three start");

    session
        .send_text("w")
        .expect("word forward to punctuation word between identifiers");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:14"))
        .expect("cursor at punctuation word start");

    session.send_text("w").expect("word forward to four");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:16"))
        .expect("cursor at four start");

    session
        .send_text("b")
        .expect("word backward to punctuation word");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:14"))
        .expect("cursor returned to punctuation word start");

    session.send_text("b").expect("word backward to two_three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:5"))
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
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport");

    session.send_text("\u{6}").expect("ctrl-f page down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            // rows=8 → height=5, scroll_margin=3 (default), page_size=4.
            // alignment_offsets collapses to middle=2 since scroll_margin(3) > height/2(2).
            // Viewport scrolls from line 0 to line 4 ("line 05" at row 1).
            // Cursor lands at alignment_offsets().top=2 rows from top: 4+2=6 = "line 07".
            s.status_line_contains("7/40:1") && s.row_trimmed_ends_with(1, "line 05")
        })
        .expect("ctrl-f: cursor at top-margin row of new viewport");

    session.send_text("\u{2}").expect("ctrl-b page up");
    session
        .wait_until(Duration::from_secs(2), |s| {
            // Viewport scrolls back from line 4 to line 0 ("line 01" at row 1).
            // alignment_offsets().bottom=2 (collapsed to middle). Cursor: 0+2=2 = "line 03".
            s.status_line_contains("3/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("ctrl-b: cursor at bottom-margin row of new viewport");

    session.send_text("\u{4}").expect("ctrl-d half page down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            // Cursor at line 2 (0-indexed, screen row 2 from viewport top 0). scroll_rows=2.
            // New viewport: line 2. Cursor preserved at screen row 2: 2+2=4 = "line 05".
            // "line 03" is now at content row 1.
            s.status_line_contains("5/40:1") && s.row_trimmed_ends_with(1, "line 03")
        })
        .expect("ctrl-d: cursor stays at same screen row after half-page scroll");

    session.send_text("\u{15}").expect("ctrl-u half page up");
    session
        .wait_until(Duration::from_secs(2), |s| {
            // Cursor at line 4 (0-indexed, screen row 2 from viewport top 2). scroll_rows=2.
            // New viewport: 0. Cursor preserved at screen row 2: 0+2=2 = "line 03".
            // "line 01" is back at content row 1.
            s.status_line_contains("3/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("ctrl-u: cursor stays at same screen row after half-page scroll");

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
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport");

    session.send_text("10j").expect("move to line 11");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11/40:1")
        })
        .expect("cursor reached line 11");

    session.send_text("zt").expect("align line to top");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(2, "line 11")
        })
        .expect("zt should place the cursor line near the top margin");

    session.send_text("zz").expect("align line to center");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(4, "line 11")
        })
        .expect("zz should place the cursor line near the viewport center");

    session.send_text("zb").expect("align line to bottom");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(6, "line 11")
        })
        .expect("zb should place the cursor line near the bottom margin");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Keep `zt` alignment near EOF stable across no-op motions and mode switches.
fn test_zt_alignment_near_eof_survives_noop_actions() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=12 {
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

    // Align the last line near the top margin so empty space remains below EOF.
    session.send_text("Gzt").expect("jump to eof and align top");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/12:1") && s.row_trimmed_ends_with(2, "line 12")
        })
        .expect("line 12 aligned near top margin");

    // A no-op `j` at EOF must not pull the viewport back down.
    session.send_text("j").expect("attempt moving past eof");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/12:1") && s.row_trimmed_ends_with(2, "line 12")
        })
        .expect("viewport should remain aligned after no-op down motion");

    // Entering insert mode with unchanged cursor should keep the same viewport origin.
    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("12/12:1")
                && s.row_trimmed_ends_with(2, "line 12")
        })
        .expect("viewport should remain aligned when entering insert mode");

    session.send_escape().expect("leave insert mode");
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
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(2, "line 11")
        })
        .expect("line 11 aligned near top margin");

    session.send_text("\u{5}").expect("ctrl-e scroll down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(1, "line 11")
        })
        .expect("ctrl-e should keep the cursor when it remains visible");

    session.send_text("\u{19}").expect("ctrl-y scroll up");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(2, "line 11")
        })
        .expect("ctrl-y should keep the cursor when it remains visible");

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
                && s.status_line_contains("11/40:1")
                && s.row_trimmed_ends_with(2, "line 11")
        })
        .expect("visual mode at aligned line");

    session
        .send_text("\u{5}")
        .expect("ctrl-e scroll down in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
                && s.status_line_contains("11/40:1")
                && s.row_trimmed_ends_with(1, "line 11")
        })
        .expect("ctrl-e should keep the visual cursor when it remains visible");

    session
        .send_text("\u{19}")
        .expect("ctrl-y scroll up in visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
                && s.status_line_contains("11/40:1")
                && s.row_trimmed_ends_with(2, "line 11")
        })
        .expect("ctrl-y should leave the visual cursor unchanged when visible");

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
fn test_ctrl_e_near_eof_keeps_cursor_on_last_line() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=12 {
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

    session.send_text("Gzt").expect("jump to eof and align top");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/12:1") && s.row_trimmed_ends_with(2, "line 12")
        })
        .expect("line 12 aligned near top margin");

    session.send_text("\u{5}").expect("ctrl-e near eof");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/12:1") && s.row_trimmed_ends_with(1, "line 12")
        })
        .expect("ctrl-e should keep eof cursor unchanged");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_counted_ctrl_e_clamps_offscreen_cursor_to_visible_band() {
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
            s.status_line_contains("11/40:1") && s.row_trimmed_ends_with(2, "line 11")
        })
        .expect("line 11 aligned near top margin");

    session.send_text("2\u{5}").expect("counted ctrl-e");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/40:1") && s.row_trimmed_ends_with(1, "line 12")
        })
        .expect("counted ctrl-e should clamp offscreen cursor to top visible line");

    session.send_text("2\u{19}").expect("counted ctrl-y");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/40:1") && s.row_trimmed_ends_with(3, "line 12")
        })
        .expect("counted ctrl-y keeps the cursor once it remains visible");

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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("initial cursor");

    session.send_text("hk").expect("attempt up/left at origin");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("cursor stays at origin");

    session
        .send_text("G$l")
        .expect("go to last line and try move right past end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:2"))
        .expect("cursor clamped at line end");

    session
        .send_text("j")
        .expect("attempt move down past last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:2"))
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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/3:1"))
        .expect("initial cursor");

    // Move to line 2, column 3 then jump to first line while preserving column.
    session.send_text("jl").expect("line 2, col 2");
    session.send_text("l").expect("line 2, col 3");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/3:3"))
        .expect("cursor at line 2 col 3");

    session.send_text("gg").expect("go to first line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/3:3"))
        .expect("gg keeps column");

    session.send_text("g$").expect("go to line end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/3:8"))
        .expect("g$ moved to line end");

    session.send_text("g0").expect("go to line start");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/3:1"))
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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("initial cursor");

    session.send_text("fa").expect("find next a");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:4"))
        .expect("cursor at found a");

    session.send_text(",").expect("repeat opposite direction");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("cursor returned to first a");

    session.send_text(";").expect("repeat original direction");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:4"))
        .expect("semicolon repeats original forward find");

    session
        .send_text(",,")
        .expect("repeat opposite direction twice");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("comma can be repeated in a row");

    session
        .send_text(";;")
        .expect("repeat base direction twice");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:4"))
        .expect("semicolon can be repeated in a row");

    session
        .send_text("0tb")
        .expect("line start then till before b");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("adjacent till keeps cursor in place");

    session
        .send_text("jfa")
        .expect("move down and find a on same line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:2"))
        .expect("find on second line");

    session
        .send_text("k$fa")
        .expect("back up, go to line end, and try find missing a on line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:4"))
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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:1"))
        .expect("initial cursor");

    session
        .send_text("ww")
        .expect("move to punctuation word before three");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:8"))
        .expect("cursor at punctuation word");

    session.send_text("ge").expect("move to previous word end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:7"))
        .expect("ge should land on two");

    session
        .send_text("wgE")
        .expect("move back to three then previous WORD end");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:3"))
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
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "child.txt")
        })
        .expect("main file opened");

    session.send_text("gf").expect("open file under cursor");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/1:1") && s.row_trimmed_ends_with(1, "child buffer")
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
            s.row_trimmed_ends_with(1, "child.txt:2:3")
        })
        .expect("main file opened");

    session
        .send_text("gF")
        .expect("open file target with line and column");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("2/3:3") && s.row_trimmed_ends_with(2, "beta line")
        })
        .expect("gF should open child file at line and column");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `gf` should report an explicit working-directory error when cwd is unavailable.
fn test_gf_reports_missing_working_directory_for_relative_buffer_paths() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("main.txt", "child.txt\n")
        .expect("write main file");
    let cwd = tree.path().join("runtime-cwd");
    std::fs::create_dir_all(&cwd).expect("create runtime cwd");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &["../main.txt"],
        PtySessionConfig {
            current_dir: Some(cwd.clone()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "child.txt")
        })
        .expect("main file opened");

    std::fs::remove_dir(&cwd).expect("delete working directory while ordex is running");
    session
        .send_text("gf")
        .expect("open file target with missing cwd");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("working directory is unavailable")
        })
        .expect("gf should surface explicit missing cwd message");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Spawn an ordex session with the given viewport size and no custom config.
///
/// The default scroll_margin (3) is used so no config file is required.
fn spawn_plain_session(file: &TempFile, rows: u16) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("file path")],
        PtySessionConfig {
            rows,
            cols: 80,
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

#[test]
/// ctrl-f places the cursor at the top-margin row of the new viewport (Neovim behavior).
fn test_ctrl_f_cursor_at_top_margin() {
    // Terminal: rows=12 → height=9 content rows. Default scroll_margin=3.
    // ctrl-f page_size=8. From viewport line 0: new viewport at line 8 (display "line 09").
    // Cursor lands at scroll_margin rows from new top: 8+3=11 (0-indexed) = display "line 12".
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = spawn_plain_session(&file, 12);

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport at top");

    session.send_text("\u{6}").expect("ctrl-f page down");

    session
        .wait_until(Duration::from_secs(2), |s| {
            // Cursor at display "line 12" (0-indexed 11 = top of new viewport 8 + scroll_margin 3).
            s.status_line_contains("12/40:1")
                // New viewport top is display "line 09".
                && s.row_trimmed_ends_with(1, "line 09")
        })
        .expect("ctrl-f: cursor at scroll_margin rows from top of new viewport");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// ctrl-b places the cursor at the bottom-margin row of the new viewport (Neovim behavior).
fn test_ctrl_b_cursor_at_bottom_margin() {
    // Terminal: rows=12 → height=9 content rows. Default scroll_margin=3.
    // Start from viewport top=8 (after ctrl-f). ctrl-b page_size=8.
    // New viewport: 8-8=0 (display "line 01" at row 1).
    // Bottom margin row: height-1-scroll_margin = 9-1-3=5. Cursor: 0+5=5 (0-indexed) = display "line 06".
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = spawn_plain_session(&file, 12);

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport at top");

    // Scroll down one full page so ctrl-b has room to scroll back.
    session.send_text("\u{6}").expect("ctrl-f page down");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/40:1")
        })
        .expect("paged down, cursor at top margin");

    session.send_text("\u{2}").expect("ctrl-b page up");

    session
        .wait_until(Duration::from_secs(2), |s| {
            // Cursor at display "line 06" (bottom-margin row: viewport 0 + (9-1-3) = 5).
            s.status_line_contains("6/40:1")
                // Viewport is back at the document top.
                && s.row_trimmed_ends_with(1, "line 01")
                // Cursor line is at content row 6.
                && s.row_trimmed_ends_with(6, "line 06")
        })
        .expect("ctrl-b: cursor at bottom-margin row of new viewport");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// ctrl-d preserves the cursor's screen row when it is inside the margin band, or snaps to the
/// top margin when the cursor is above the margin band.
fn test_ctrl_d_preserves_cursor_screen_row() {
    // Terminal: rows=12 → height=9 content rows. Default scroll_margin=3,
    // alignment_offsets().top=3.
    // Cursor at content row 3 (screen row 2, 0-indexed: line 2 = display "line 03").
    // screen_row=2 is below the top margin (3), so the cursor snaps to the margin:
    // new viewport top=4, cursor = 4+3=7 = display "line 08". Content row 4.
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = spawn_plain_session(&file, 12);

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport");

    // Move cursor to content row 3 (display "line 03", 0-indexed line 2).
    session.send_text("jj").expect("move down 2 lines");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("3/40:1"))
        .expect("cursor at line 3");

    session.send_text("\u{4}").expect("ctrl-d half page down");

    session
        .wait_until(Duration::from_secs(2), |s| {
            // rows=12 → height=9, scroll_margin=3, alignment_offsets().top=3. scroll_rows=4.
            // Cursor was at screen row 2, which is below the top margin (3), so it snaps to the
            // top margin: new viewport top=4, cursor = 4+3=7 = display "line 08". Content row 4.
            s.status_line_contains("8/40:1")
                // Viewport scrolled: display "line 05" is now at content row 1.
                && s.row_trimmed_ends_with(1, "line 05")
                // Cursor is at content row 4 (screen row 3, the top margin).
                && s.row_trimmed_ends_with(4, "line 08")
        })
        .expect("ctrl-d: cursor snaps to top margin when it was below the margin band");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// ctrl-d always scrolls the viewport; when the cursor is below the scroll-margin band it is
/// snapped to the top margin of the new viewport.
fn test_ctrl_d_always_scrolls_from_top_of_screen() {
    // Terminal: rows=12 → height=9. Default scroll_margin=3, alignment_offsets().top=3.
    // Cursor at display line 1 (0-indexed line 0, screen row 0), viewport at line 0.
    // ctrl-d scroll_rows=4. New viewport: line 4. screen_row=0 < top_margin=3, so cursor
    // snaps to 4+3=7 = display "line 08".
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = spawn_plain_session(&file, 12);

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport with cursor at top");

    session.send_text("\u{4}").expect("ctrl-d half page down");

    session
        .wait_until(Duration::from_secs(2), |s| {
            // rows=12 → height=9, scroll_margin=3, alignment_offsets().top=3. scroll_rows=4.
            // Cursor was at screen row 0, which is below the top margin (3), so it snaps to the
            // top margin: new viewport top=4, cursor = 4+3=7 = display "line 08". Content row 4.
            // Viewport scrolled: display "line 05" is now at content row 1.
            s.row_trimmed_ends_with(1, "line 05") && s.status_line_contains("8/40:1")
        })
        .expect("ctrl-d: viewport always scrolls even when cursor starts at the top of the screen");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// ctrl-u preserves the cursor's screen row after scrolling half a page.
fn test_ctrl_u_preserves_cursor_screen_row() {
    // Terminal: rows=12 → height=9. Default scroll_margin=3.
    // After ctrl-f: viewport top at line 8 (display "line 09"), cursor at line 11 (display "line 12").
    // Move cursor up 2: cursor at line 9 (display "line 10"). Because scroll_margin=3 and the cursor
    // is now within the top margin (9 < 8+3=11), ensure_cursor_visible shifts the viewport up so
    // the cursor sits at scroll_margin rows from the top: first_visible = 9-3 = 6 (display "line 07").
    // Screen row of cursor = 9 - 6 = 3.
    // ctrl-u scroll_rows=4. New viewport: 6-4=2 (display "line 03" at row 1).
    // Cursor preserved at screen row 3: 2+3=5 = display "line 06". Content row 4.
    let file = TempFile::new().expect("create temp file");
    for i in 1..=40 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = spawn_plain_session(&file, 12);

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/40:1") && s.row_trimmed_ends_with(1, "line 01")
        })
        .expect("initial viewport");

    // Scroll down one full page to place viewport at line 8, cursor at line 11.
    session
        .send_text("\u{6}")
        .expect("ctrl-f to scroll down one page");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("12/40:1")
        })
        .expect("paged down, cursor at top margin");

    // Move cursor up 2 rows. ensure_cursor_visible adjusts viewport to respect scroll_margin.
    session.send_text("kk").expect("move cursor up 2 lines");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("10/40:1")
        })
        .expect("cursor at line 10");

    session.send_text("\u{15}").expect("ctrl-u half page up");

    session
        .wait_until(Duration::from_secs(2), |s| {
            // New viewport top: display "line 03" at content row 1.
            s.row_trimmed_ends_with(1, "line 03")
                // Cursor preserved at same screen row 3: 2+3=5 = display "line 06".
                && s.status_line_contains("6/40:1")
                // Cursor sits at content row 4 (display "line 03" at row 1, cursor 3 rows below).
                && s.row_trimmed_ends_with(4, "line 06")
        })
        .expect("ctrl-u: cursor stays at same screen row after half-page scroll up");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
