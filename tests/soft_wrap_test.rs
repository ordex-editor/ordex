use std::fs;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_soft_wrap_is_enabled_by_default() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnop").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 12,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "   1 abcdefg") && s.row_trimmed_ends_with(2, "    hijklmn")
        })
        .expect("wrapped rows should render by default");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_j_and_k_move_by_wrapped_rows() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrst\nzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n")
        .expect("seed file");

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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("initial cursor");

    session.send_text("j").expect("move to next wrapped row");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:36"))
        .expect("j should move within the wrapped line first");

    session
        .send_text("k")
        .expect("move back to previous wrapped row");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("k should move back within the wrapped line");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_can_be_disabled_via_config() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnopqrstuvwxyz")
        .expect("seed file");

    let config = TempFile::new().expect("create temp config");
    config
        .write_all(
            br#"
[editor]
soft_wrap = false
 "#,
        )
        .expect("write config");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        PtySessionConfig {
            cols: 100,
            rows: 30,
            ..Default::default()
        },
    )
    .expect("spawn ordex with config");
    session.resize(20, 8).expect("set terminal size");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "   1 abcdefghijklmno") && s.row_trimmed_ends_with(2, "   ~")
        })
        .expect("unwrapped long line should stay on one row");

    session
        .send_text("llllllllllllllllllll")
        .expect("move right repeatedly");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "lmnopqrstuvwxyz")
        })
        .expect("horizontal scrolling should remain available");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_handles_unicode_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all("éééééééééééééééééééééééééééééééééééééé".as_bytes())
        .expect("seed file");

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
            s.row_contains(1, "  1 éééééééééééééééé") && s.row_contains(2, "    éé")
        })
        .expect("unicode text should wrap cleanly");

    session.send_text("j").expect("move to wrapped unicode row");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:36"))
        .expect("wrapped unicode row should keep character indexing");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_wraps_while_in_insert_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"").expect("seed file");
    let inserted = "abcdefghijklmnopqrstuvwxyzabcdefghijkl";

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
        .send_text(&format!("i{inserted}"))
        .expect("insert wrapped text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "   1 abcdefghijklmnopqrstuvwxyzabcdefghi")
                && s.row_trimmed_ends_with(2, "    jkl")
                && s.status_line_contains("INSERT ")
                && s.status_line_contains("1/1:39")
        })
        .expect("insert mode should wrap text and keep the cursor state coherent");

    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("return to normal mode");
    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify wrapped rendering never spills into the reserved status row.
#[test]
fn test_soft_wrap_does_not_overwrite_status_bar() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
    )
    .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 12,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "   1 abcdefg")
                && s.row_trimmed_ends_with(5, "    cdefghi")
                && s.status_line_contains("NORMAL ")
        })
        .expect("wrapped content should stop before the status bar");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_preserves_syntax_highlighting_across_wrapped_rows() {
    let file = std::env::temp_dir().join(format!("ordex_wrap_syntax_{}.rs", std::process::id()));
    fs::write(
        &file,
        b"fn wrap_test() { let message = \"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\"; }\n",
    )
    .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 28,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_trimmed_ends_with(1, "   1 fn wrap_test() { let me")
                && snapshot.row_trimmed_ends_with(2, "    ssage = \"abcdefghijklmn")
        })
        .expect("wrapped syntax fixture should be visible");

    session
        .read_available()
        .expect("collect wrapped transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.row_trimmed_ends_with(1, "   1 fn wrap_test() { let me"));
    assert!(snapshot.row_trimmed_ends_with(2, "    ssage = \"abcdefghijklmn"));
    assert!(
        snapshot.contains("\u{1b}[38;5;179m\u{1b}[1mfn"),
        "wrapped first row should retain keyword highlighting"
    );
    assert!(
        snapshot.contains("\u{1b}[38;5;79m"),
        "wrapped string literal should keep string highlighting across rows"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
    let _ = fs::remove_file(file);
}

/// `<count>j` with soft-wrap enabled must skip `count` logical buffer lines,
/// not `count` visual/wrapped rows.  The first line is long enough to wrap
/// across multiple visual rows; `3j` should land three logical lines below the
/// start position regardless of the wrap geometry.
#[test]
fn test_counted_j_moves_by_logical_lines() {
    let file = TempFile::new().expect("create temp file");
    // Line 1: 72-char line – wraps at width 40 (two visual rows visible).
    // Lines 2–5: short lines.
    file.write_all(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrst\nline2\nline3\nline4\nline5\n",
    )
    .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 40,
            rows: 12,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("initial cursor at line 1");

    // 3j should land on logical line 4 (status shows "4/5").
    session.send_text("3j").expect("counted j");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("4/5:1"))
        .expect("3j must jump 3 logical lines to line 4");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// `<count>k` with soft-wrap enabled must move `count` logical lines upward.
#[test]
fn test_counted_k_moves_by_logical_lines() {
    let file = TempFile::new().expect("create temp file");
    // Same layout as the j test; start at line 4 and move up.
    file.write_all(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrst\nline2\nline3\nline4\nline5\n",
    )
    .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 40,
            rows: 12,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("initial cursor at line 1");

    // Navigate to line 4 with G then count.
    session.send_text("4G").expect("go to line 4");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("4/5:1"))
        .expect("cursor at line 4");

    // 3k must land on logical line 1 (not on a visual row of the long first line).
    session.send_text("3k").expect("counted k");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("3k must jump 3 logical lines up to line 1");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// `1j` (explicit count of one) with soft-wrap enabled must move to the next
/// logical line, not to the next visual row within the same wrapped line.
#[test]
fn test_count_one_j_moves_to_next_logical_line() {
    let file = TempFile::new().expect("create temp file");
    // Line 1 wraps; line 2 is short.
    file.write_all(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrst\nshort\n",
    )
    .expect("seed file");

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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("initial cursor at line 1");

    // 1j should jump to logical line 2, not stay on a wrapped row of line 1.
    session.send_text("1j").expect("1j");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:1"))
        .expect("1j must move to logical line 2");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Plain `j` without a count continues to move by visual wrapped rows even when
/// the cursor is inside a long line that spans multiple screen rows.
#[test]
fn test_plain_j_still_moves_by_wrapped_rows() {
    let file = TempFile::new().expect("create temp file");
    // Line 1 wraps at width 40: first visual row cols 1–35, second 36–70.
    file.write_all(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrst\nshort\n",
    )
    .expect("seed file");

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
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("initial cursor at line 1");

    // Plain j moves one visual row: still logical line 1 but column advances.
    session.send_text("j").expect("plain j");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/2:") && !s.status_line_contains("1/2:1")
        })
        .expect("plain j should stay on logical line 1 (moved within wrapped rows)");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// `<count>j` with soft-wrap disabled continues to use logical-line movement
/// (no regression from the existing behavior).
#[test]
fn test_counted_j_soft_wrap_disabled_unchanged() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line1\nline2\nline3\nline4\nline5\n")
        .expect("seed file");

    let config = TempFile::new().expect("create temp config");
    config
        .write_all(
            br#"
[editor]
soft_wrap = false
"#,
        )
        .expect("write config");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        PtySessionConfig {
            cols: 40,
            rows: 12,
            ..Default::default()
        },
    )
    .expect("spawn ordex with config");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/5:1"))
        .expect("initial cursor at line 1");

    session.send_text("3j").expect("counted j");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("4/5:1"))
        .expect("3j must jump 3 logical lines with soft-wrap disabled");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// In insert mode, when a line exactly fills the content width, the
/// cursor must sit on its own visual row instead of overlapping with
/// the last character.  Verify that the next line's content is not
/// shown on that cursor row.
#[test]
fn test_insert_mode_exact_wrap_cursor_on_new_row() {
    let file = TempFile::new().expect("create temp file");
    // First line is exactly content_width characters so it fills one
    // wrapped row.  Second line has enough text to wrap across two
    // visual rows so we can confirm it is shifted down.
    // cols=20, gutter = marker(1) + digits(3) + separator(1) = 5,
    // so content_width = 15.
    file.write_all(b"abcdefghijklmno\nSECOND_LINE_CONTENT\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 20,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    // Start at line 1, press A to move past the last character and
    // enter insert mode.  The cursor is at buffer column 15
    // (0-indexed past end of "abcdefghijklmno"), which is one past the
    // content width boundary.
    session
        .send_text("A")
        .expect("append at line end in insert mode");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            // First row shows the wrapped first line (no wrapping needed).
            snapshot.row_trimmed_ends_with(1, "   1 abcdefghijklmno")
                // The cursor row must NOT show content from the second line.
                && !snapshot.row_trimmed_ends_with(2, "SECOND")
                // Second line is shifted down and starts at row 3.
                && snapshot.row_contains(3, "SECOND")
                // Status line confirms insert mode and position.
                && snapshot.status_line_contains("INSERT ")
                && snapshot.status_line_contains("1/2:16")
        })
        .expect("cursor should sit on an empty row after the wrapped first line");

    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("return to normal mode");
    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
