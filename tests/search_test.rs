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
        .wait_until(Duration::from_secs(2), |s| s.contains("one"))
        .expect("initial content");

    session.send_text("/target").expect("enter search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.contains("SEARCH |") && s.contains("/target")
        })
        .expect("search prompt should be visible");
    session.send_enter().expect("execute search");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| s.contains("2:1"))
        .expect("cursor moved to found line");

    assert!(snapshot.contains("target line"));

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
        .wait_until(Duration::from_secs(2), |s| s.contains("NORMAL |"))
        .expect("wait for ready");

    session.send_text("/zzz").expect("search missing pattern");
    session.send_enter().expect("execute search");

    session
        .wait_until(Duration::from_secs(2), |s| s.contains("Pattern not found"))
        .expect("pattern-not-found message");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
