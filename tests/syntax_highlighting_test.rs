use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the absolute path to one syntax fixture file.
fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("syntax")
        .join(name)
}

/// Open one fixture file and wait for the initial normal-mode frame.
fn open_fixture(name: &str) -> PtySession {
    let path = fixture_path(name);
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[path.to_str().expect("fixture path utf8")],
        Default::default(),
    )
    .expect("spawn ordex");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");
    session
}

/// Create one temporary Rust file for syntax-highlighting integration tests.
fn temp_rust_file() -> TempFile {
    TempFile::with_suffix(".rs").expect("create temp rust file")
}

/// Return the stable escape sequence for keyword styling.
fn keyword_escape() -> &'static str {
    "\u{1b}[38;5;179m\u{1b}[1m"
}

/// Return the stable escape sequence for ordinary comment styling.
fn comment_escape() -> &'static str {
    "\u{1b}[38;5;249m"
}

/// Return the last synchronized terminal frame captured in one PTY snapshot.
fn last_sync_frame(snapshot: &test_utils::ScreenSnapshot) -> &str {
    let begin = "\u{1b}[?2026h";
    let end = "\u{1b}[?2026l";
    let raw = snapshot.raw();
    let Some(frame_end) = raw.rfind(end) else {
        return raw;
    };
    let upto_end = &raw[..frame_end];
    let Some(frame_start) = upto_end.rfind(begin) else {
        return raw;
    };
    &raw[frame_start..frame_end]
}

/// Return the stable escape sequence for documentation comment styling.
fn doc_comment_escape() -> &'static str {
    "\u{1b}[38;5;113m"
}

/// Return the stable escape sequence for string styling.
fn string_escape() -> &'static str {
    "\u{1b}[38;5;79m"
}

/// Return the stable escape sequence for number styling.
fn number_escape() -> &'static str {
    "\u{1b}[38;5;74m"
}

/// Return the stable escape sequence for Markdown heading styling.
fn heading_escape() -> &'static str {
    "\u{1b}[38;5;74m\u{1b}[1m"
}

/// Return the stable escape sequence for inline-code styling.
fn inline_code_escape() -> &'static str {
    "\u{1b}[38;5;179m"
}

/// Return the stable escape sequence for link styling.
fn link_escape() -> &'static str {
    "\u{1b}[38;5;179m\u{1b}[4m"
}

/// Verify one fixture renders the expected token classes at open time.
fn assert_fixture_renders_expected_tokens(
    name: &str,
    expect_keyword: bool,
    expect_string: bool,
    expect_number: bool,
    expect_comment: bool,
) {
    let mut session = open_fixture(name);
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    if expect_keyword {
        assert!(
            snapshot.contains(keyword_escape()),
            "expected keyword in {name}"
        );
    }
    if expect_string {
        assert!(
            snapshot.contains(string_escape()),
            "expected string in {name}"
        );
    }
    if expect_number {
        assert!(
            snapshot.contains(number_escape()),
            "expected number in {name}"
        );
    }
    if expect_comment {
        assert!(
            snapshot.contains(comment_escape()),
            "expected comment in {name}"
        );
    }
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_open_time_rust_highlighting_renders_distinct_tokens() {
    let mut session = open_fixture("sample.rs");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.contains(doc_comment_escape()));
    assert!(snapshot.contains(keyword_escape()));
    assert!(snapshot.contains(string_escape()));
    assert!(snapshot.contains(number_escape()));
    assert!(snapshot.contains(comment_escape()));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_open_time_toml_highlighting_renders_distinct_tokens() {
    let mut session = open_fixture("sample.toml");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.contains(keyword_escape()));
    assert!(snapshot.contains(string_escape()));
    assert!(snapshot.contains(number_escape()));
    assert!(snapshot.contains(comment_escape()));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_open_time_config_highlighting_renders_distinct_tokens() {
    let mut session = open_fixture("sample.cfg");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.contains(keyword_escape()));
    assert!(snapshot.contains(string_escape()));
    assert!(snapshot.contains(number_escape()));
    assert!(snapshot.contains(comment_escape()));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_open_time_markdown_highlighting_renders_distinct_tokens() {
    let mut session = open_fixture("sample.md");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.contains(heading_escape()));
    assert!(snapshot.contains(inline_code_escape()));
    assert!(snapshot.contains(link_escape()));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_open_time_d_highlighting_renders_distinct_tokens() {
    let mut session = open_fixture("sample.d");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.contains(doc_comment_escape()));
    assert!(snapshot.contains(keyword_escape()));
    assert!(snapshot.contains(string_escape()));
    assert!(snapshot.contains(number_escape()));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_unsupported_files_render_with_plain_fallback_only() {
    let mut session = open_fixture("unsupported.txt");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.row_contains(1, "plain fallback text"));
    assert!(
        !snapshot.contains(keyword_escape())
            && !snapshot.contains(comment_escape())
            && !snapshot.contains(doc_comment_escape())
            && !snapshot.contains(string_escape())
            && !snapshot.contains(number_escape()),
        "plain fallback should not emit syntax color escapes"
    );
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_irregular_markdown_stays_conservative_and_readable() {
    let mut session = open_fixture("irregular.md");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.row_contains(1, "a_b_c * and [brackets] without target"));
    assert!(
        !snapshot.contains("\u{1b}[38;5;74m"),
        "unsupported Markdown constructs should stay plain"
    );
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify code-language fixtures render distinct tokens at open time.
#[test]
fn test_open_time_new_language_highlighting_renders_distinct_tokens() {
    for fixture in [
        "sample.js",
        "sample.ts",
        "sample.py",
        "sample.java",
        "sample.cs",
        "sample.cpp",
        "sample.go",
        "sample.c",
        "sample.php",
    ] {
        assert_fixture_renders_expected_tokens(fixture, true, true, true, true);
    }
}

/// Verify AsciiDoc renders comment and markup-specific highlighting at open time.
#[test]
fn test_open_time_asciidoc_highlighting_renders_markup_constructs() {
    let mut session = open_fixture("sample.adoc");
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.contains(comment_escape()));
    assert!(snapshot.contains(heading_escape()));
    assert!(snapshot.contains(inline_code_escape()));
    assert!(snapshot.contains(link_escape()));
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_scrolling_keeps_visible_syntax_highlighting() {
    let file = temp_rust_file();
    let mut writer = std::io::BufWriter::new(
        std::fs::File::create(file.path()).expect("open temp rust file for writing"),
    );
    writer
        .write_all(b"/// heading\n")
        .expect("write first line");
    for _ in 0..199 {
        writer
            .write_all(b"let value = 1;\n")
            .expect("append highlighted line");
    }
    writer.flush().expect("flush temp rust file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("temp file path utf8")],
        PtySessionConfig {
            cols: 80,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ") && snapshot.contains(keyword_escape())
        })
        .expect("initial syntax-highlighted render");

    session.clear_transcript();
    session.send_text("\u{6}").expect("ctrl-f page down");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("5:1")
                && snapshot.row_contains(1, "let value = 1;")
                && snapshot.contains(keyword_escape())
        })
        .expect("page-down render should keep keyword highlighting");

    for target_line in 6..=84 {
        session.clear_transcript();
        session.send_text("j").expect("scroll down with j");
        session
            .wait_until(Duration::from_secs(2), |snapshot| {
                snapshot.status_line_contains(&format!("{target_line}:1"))
                    && snapshot.row_contains(1, "let value = 1;")
                    && snapshot.contains(keyword_escape())
            })
            .expect("each j redraw should keep keyword highlighting");
    }

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_scrolling_preserves_multiline_comment_highlighting() {
    let file = temp_rust_file();
    let mut writer = std::io::BufWriter::new(
        std::fs::File::create(file.path()).expect("open temp rust file for writing"),
    );
    writer
        .write_all(b"/* open comment\n")
        .expect("write comment opener");
    for _ in 0..199 {
        writer
            .write_all(b"comment body\n")
            .expect("append comment line");
    }
    writer
        .write_all(b"*/\nlet value = 1;\n")
        .expect("write comment closer and code");
    writer.flush().expect("flush temp rust file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("temp file path utf8")],
        PtySessionConfig {
            cols: 80,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ") && snapshot.contains(comment_escape())
        })
        .expect("initial multiline comment render");

    session.clear_transcript();
    session.send_text("\u{6}").expect("ctrl-f page down");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("5:1")
                && snapshot.row_contains(1, "comment body")
                && snapshot.contains(comment_escape())
        })
        .expect("page-down comment render should keep comment styling");

    for target_line in 6..=84 {
        session.clear_transcript();
        session.send_text("j").expect("scroll down with j");
        let snapshot = session
            .wait_until(Duration::from_secs(2), |snapshot| {
                snapshot.status_line_contains(&format!("{target_line}:1"))
                    && snapshot.row_contains(1, "comment body")
            })
            .expect("each j redraw should reach the expected far comment line");
        assert!(
            snapshot.contains(comment_escape()),
            "j redraw at line {target_line} lost comment styling; raw transcript:\n{}",
            snapshot.raw()
        );
    }

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_ctrl_f_on_main_rs_preserves_comment_coloring() {
    let path = fixture_path("main_scroll_fixture.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[path.to_str().expect("fixture path utf8")],
        PtySessionConfig {
            cols: 80,
            rows: 24,
            ..Default::default()
        },
    )
    .expect("spawn ordex on frozen main fixture");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ") && snapshot.contains(comment_escape())
        })
        .expect("initial syntax-highlighted render");

    for _ in 0..4 {
        session.clear_transcript();
        session.send_text("\u{6}").expect("ctrl-f page down");
    }

    let snapshot = session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("81:1") && snapshot.raw().contains("\u{1b}[?2026l")
        })
        .expect("four ctrl-f presses should reach the comment block in the frozen fixture");
    let last_frame = last_sync_frame(&snapshot);
    assert!(
        last_frame.contains("/// Synchronize viewport width"),
        "comment row disappeared after four ctrl-f presses; last frame:\n{}",
        last_frame
    );
    assert!(
        last_frame.contains(&format!(
            "{}/// Synchronize viewport width",
            doc_comment_escape()
        )),
        "doc comment styling disappeared after four ctrl-f presses; last frame:\n{}",
        last_frame
    );

    session.clear_transcript();
    for _ in 0..5 {
        session.send_text("j").expect("move to inline comment");
    }
    let snapshot = session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.contains("Gutter-width changes alter the effective content width")
                && snapshot.contains(comment_escape())
        })
        .expect("inline comment should be visible after stepping down");
    assert!(
        snapshot.contains(comment_escape()),
        "comment styling disappeared after stepping to the inline comment; raw transcript:\n{}",
        snapshot.raw()
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
