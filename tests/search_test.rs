use std::time::Duration;
use test_utils::{PtySession, TempFile};

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
                && s.row_contains(1, "one")
                && s.row_contains(2, "target line")
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
                && s.status_line_contains("2:1")
                && s.row_contains(2, "target line")
        })
        .expect("cursor moved to found line");

    assert!(snapshot.row_contains(2, "target line"));

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
                && s.row_contains(1, "alpha")
                && s.row_contains(2, "beta")
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
                && s.row_contains(1, "target one")
                && s.row_contains(3, "target two")
        })
        .expect("initial content");

    session.send_text("/target").expect("enter search");
    session.send_enter().expect("execute search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
        })
        .expect("first match selected");

    session.send_text("n").expect("search next");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("3:1")
        })
        .expect("next match selected");

    session.send_text("N").expect("search previous");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
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
                && s.row_contains(1, "abc")
                && s.row_contains(2, "axc")
        })
        .expect("initial content");

    session.send_text("/a.c").expect("enter regex search");
    session.send_enter().expect("execute search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
        })
        .expect("first regex match selected");

    session.send_text("n").expect("search next");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("2:1")
        })
        .expect("second regex match selected");

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
                && s.row_contains(1, "alpha")
                && s.row_contains(2, "beta")
        })
        .expect("wait for ready");

    session
        .send_text("/(?=beta)")
        .expect("search invalid regex");
    session.send_enter().expect("execute search");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("look-around")
        })
        .expect("invalid-regex message");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
