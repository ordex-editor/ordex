use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_search_found_moves_cursor() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"one\ntarget line\nthree\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "one")
                && s.row_trimmed_ends_with(2, "target line")
        })
        .expect("initial content");

    session.send_text("/target").expect("enter search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.message_line_contains("/target")
        })
        .expect("search prompt should be visible");
    session.send_enter().expect("execute search");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("2/3:1")
                && s.row_trimmed_ends_with(2, "target line")
        })
        .expect("cursor moved to found line");

    assert!(snapshot.row_trimmed_ends_with(2, "target line"));

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_preview_keeps_cursor_in_place_until_enter() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"target one\nmiddle\ntarget two\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("1/3:1")
                && s.row_trimmed_ends_with(1, "target one")
        })
        .expect("initial content");

    session.send_text("/target").expect("type search preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.status_line_contains("1/3:1")
                && s.message_line_contains("/target")
        })
        .expect("search preview should keep the original cursor position");

    session.send_escape().expect("leave search mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/3:1")
        })
        .expect("return to normal mode");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_not_found_shows_message() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.row_trimmed_ends_with(2, "beta")
        })
        .expect("wait for ready");

    session.send_text("/zzz").expect("search missing pattern");
    session.send_enter().expect("execute search");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("Pattern not found")
        })
        .expect("pattern-not-found message");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_next_previous_occurrence() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"target one\nmiddle\ntarget two\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "target one")
                && s.row_trimmed_ends_with(3, "target two")
        })
        .expect("initial content");

    session.send_text("/target").expect("enter search");
    session.send_enter().expect("execute search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/3:1")
        })
        .expect("first match selected");

    session.send_text("n").expect("search next");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("3/3:1")
        })
        .expect("next match selected");

    session.send_text("N").expect("search previous");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/3:1")
        })
        .expect("previous match selected");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Regex search should match non-literal patterns in the UI flow.
fn test_search_regex_pattern_matches() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\naxc\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "abc")
                && s.row_trimmed_ends_with(2, "axc")
        })
        .expect("initial content");

    session.send_text("/a.c").expect("enter regex search");
    session.send_enter().expect("execute search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("first regex match selected");

    session.send_text("n").expect("search next");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("2/2:1")
        })
        .expect("second regex match selected");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Search should accept `\n` to match across line breaks from the `/` prompt.
fn test_search_newline_escape_matches_across_lines() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"one\nalpha\nbeta\nthree\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(2, "alpha")
                && s.row_trimmed_ends_with(3, "beta")
        })
        .expect("initial content");

    session
        .send_text("/alpha\\nbeta")
        .expect("enter cross-line search");
    session.send_enter().expect("execute search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("2/4:1")
                && s.row_trimmed_ends_with(2, "alpha")
                && s.row_trimmed_ends_with(3, "beta")
        })
        .expect("cross-line search should land on the first line of the match");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Invalid regex input should be surfaced to the user.
fn test_search_invalid_regex_shows_message() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.row_trimmed_ends_with(2, "beta")
        })
        .expect("wait for ready");

    session
        .send_text("/(?=beta)")
        .expect("search invalid regex");
    session.send_enter().expect("execute search");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "Invalid regex:")
        })
        .expect("invalid-regex message");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Current-line substitute should replace every match on the active line only.
fn test_substitute_current_line_replaces_all_matches() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo foo\nfoo\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "foo foo")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("initial content");

    session.send_text(":s/foo/bar/").expect("enter substitute");
    session.send_enter().expect("execute substitute");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "bar bar")
                && s.row_trimmed_ends_with(2, "foo")
                && s.message_line_contains("2 substitutions")
        })
        .expect("substitute result");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Whole-file substitute should support alternate delimiters and capture expansions.
fn test_substitute_whole_file_supports_capture_expansion() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha-12\nbeta-7\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha-12")
                && s.row_trimmed_ends_with(2, "beta-7")
        })
        .expect("initial content");

    session
        .send_text(r":%s#([a-z]+)-(\d+)#$2:$1#")
        .expect("enter substitute");
    session.send_enter().expect("execute substitute");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "12:alpha")
                && s.row_trimmed_ends_with(2, "7:beta")
                && s.message_line_contains("2 substitutions")
        })
        .expect("capture substitute result");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Successful substitute should refresh the last-search pattern used by `n`.
fn test_substitute_updates_last_search_pattern() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo\nfoo\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "foo")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("initial content");

    session.send_text(":s/foo/bar/").expect("enter substitute");
    session.send_enter().expect("execute substitute");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "bar")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("current-line substitute");

    session.send_text("n").expect("repeat search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("2/2:1")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("substitute should refresh last search");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Invalid substitute regex should surface the same regex error overlay as search.
fn test_substitute_invalid_regex_shows_message() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.row_trimmed_ends_with(2, "beta")
        })
        .expect("wait for ready");

    session
        .send_text(":%s/(?=beta)/x/")
        .expect("substitute invalid regex");
    session.send_enter().expect("execute substitute");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "Invalid regex:")
        })
        .expect("invalid substitute regex");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Substitute should still execute when the final delimiter is omitted.
fn test_substitute_accepts_missing_final_delimiter() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo foo\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foo foo")
        })
        .expect("initial content");

    session.send_text(":s/foo/bar").expect("enter substitute");
    session.send_enter().expect("execute substitute");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "bar bar")
                && s.message_line_contains("2 substitutions")
        })
        .expect("substitute without final delimiter");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Typing a valid substitute should preview the replacement before Enter commits it.
fn test_substitute_preview_updates_buffer_before_enter() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo foo\nfoo\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Wait for the starting buffer so the later preview assertion only checks
    // substitute-driven redraws instead of startup paint.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "foo foo")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("initial content");

    session
        .send_text(":s/foo/bar")
        .expect("type substitute preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.message_line_contains(":s/foo/bar")
                && s.row_trimmed_ends_with(1, "bar bar")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("preview should update buffer view");

    session.send_escape().expect("cancel preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "foo foo")
                && s.row_trimmed_ends_with(2, "foo")
        })
        .expect("cancel should restore original view");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Preview cancellation should restore the original viewport after recentring on the first match.
fn test_substitute_preview_escape_restores_original_viewport() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"top one\ntop two\nmid one\nmid two\nfoo target\nbottom\n")
        .expect("seed file");
    let config = PtySessionConfig {
        rows: 6,
        ..Default::default()
    };

    let mut session = PtySession::spawn(ordex_bin(), &[file.path().to_str().unwrap()], config)
        .expect("spawn ordex");

    // Keep the opening viewport near the top so preview recentering is visible.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "top one")
                && s.row_trimmed_ends_with(2, "top two")
        })
        .expect("initial viewport");

    session
        .send_text(":%s/foo/bar")
        .expect("type whole-file substitute preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.row_trimmed_ends_with(1, "mid two")
                && s.row_trimmed_ends_with(2, "bar target")
                && !s.row_contains(1, "top one")
        })
        .expect("preview should recenter on first match");

    session.send_escape().expect("cancel preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "top one")
                && s.row_trimmed_ends_with(2, "top two")
                && !s.row_contains(3, "bar target")
        })
        .expect("cancel should restore original viewport");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Committing a previewed substitute should keep the preview-centered viewport in place.
fn test_substitute_preview_enter_keeps_recentered_viewport() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"top one\ntop two\nmid one\nmid two\nfoo target\nbottom\n")
        .expect("seed file");
    let config = PtySessionConfig {
        rows: 6,
        ..Default::default()
    };

    let mut session = PtySession::spawn(ordex_bin(), &[file.path().to_str().unwrap()], config)
        .expect("spawn ordex");

    // The preview should move the viewport away from the top and Enter should
    // leave that same viewport visible after the real edit is committed.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "top one")
                && s.row_trimmed_ends_with(2, "top two")
        })
        .expect("initial viewport");

    session.send_text(":%s/foo/bar").expect("type preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.row_trimmed_ends_with(1, "mid two")
                && s.row_trimmed_ends_with(2, "bar target")
                && !s.row_contains(1, "top one")
        })
        .expect("preview should recenter");
    session.send_enter().expect("commit preview");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "mid two")
                && s.row_trimmed_ends_with(2, "bar target")
                && !s.row_contains(1, "top one")
                && s.message_line_contains("1 substitution")
        })
        .expect("commit should keep centered viewport");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Substitute replacement should turn `\r` into a rendered line break.
fn test_substitute_replacement_newline_escape_splits_lines() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foo")
        })
        .expect("initial content");

    session
        .send_text(":%s/foo/bar\\rbaz/")
        .expect("enter multiline substitute");
    session.send_enter().expect("execute substitute");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "bar")
                && s.row_trimmed_ends_with(2, "baz")
                && s.message_line_contains("1 substitution")
        })
        .expect("replacement should split the line");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Substitute preview should render `\r` replacements before Enter commits them.
fn test_substitute_preview_renders_multiline_replacement_escape() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo tail\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foo tail")
        })
        .expect("initial content");

    session
        .send_text(":s/foo/bar\\rbaz")
        .expect("type multiline preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
                && s.message_line_contains(":s/foo/bar\\rbaz")
                && s.row_trimmed_ends_with(1, "bar")
                && s.row_trimmed_ends_with(2, "baz tail")
        })
        .expect("preview should render the split replacement");

    session.send_escape().expect("cancel preview");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foo tail")
        })
        .expect("cancel should restore the original view");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Escaped backslashes should keep `\\r` literal in substitute replacements.
fn test_substitute_replacement_preserves_literal_backslash_r() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foo")
        })
        .expect("initial content");

    session
        .send_text(r":%s/foo/bar\\rbaz/")
        .expect("enter literal escape substitute");
    session.send_enter().expect("execute substitute");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, r"bar\rbaz")
                && s.message_line_contains("1 substitution")
        })
        .expect("literal escape should stay on one line");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Search should add to jump history so Ctrl-O can return to the original location.
fn test_search_adds_to_jump_history_ctrl_o_returns() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"start here\nmiddle\ntarget line\nend\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/4:1")
        })
        .expect("initial content at line 1");

    session.send_text("/target").expect("enter search");
    session.send_enter().expect("execute search");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("3/4:1")
        })
        .expect("cursor moved to target line");

    session
        .send_text("\u{f}")
        .expect("send Ctrl-O to go back in jump history");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/4:1")
        })
        .expect("Ctrl-O returned to original line");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
