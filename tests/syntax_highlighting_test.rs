use std::path::{Path, PathBuf};
use std::time::Duration;
use test_utils::PtySession;

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
            snapshot.status_line_contains("NORMAL |")
        })
        .expect("wait for initial render");
    session
}

/// Return the stable escape sequence for keyword styling.
fn keyword_escape() -> &'static str {
    "\u{1b}[38;5;4m\u{1b}[1m"
}

/// Return the stable escape sequence for ordinary comment styling.
fn comment_escape() -> &'static str {
    "\u{1b}[38;5;2m"
}

/// Return the stable escape sequence for documentation comment styling.
fn doc_comment_escape() -> &'static str {
    "\u{1b}[38;5;10m"
}

/// Return the stable escape sequence for string styling.
fn string_escape() -> &'static str {
    "\u{1b}[38;5;3m"
}

/// Return the stable escape sequence for number styling.
fn number_escape() -> &'static str {
    "\u{1b}[38;5;5m"
}

/// Return the stable escape sequence for Markdown heading styling.
fn heading_escape() -> &'static str {
    "\u{1b}[38;5;12m\u{1b}[1m"
}

/// Return the stable escape sequence for inline-code styling.
fn inline_code_escape() -> &'static str {
    "\u{1b}[38;5;11m"
}

/// Return the stable escape sequence for link styling.
fn link_escape() -> &'static str {
    "\u{1b}[38;5;13m\u{1b}[4m"
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
        !snapshot.contains("\u{1b}[38;5;"),
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
        !snapshot.contains("\u{1b}[38;5;12m"),
        "unsupported Markdown constructs should stay plain"
    );
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
