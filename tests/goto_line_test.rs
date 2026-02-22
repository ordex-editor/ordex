use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_goto_line_updates_cursor_position() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line1\nline2\nline3\nline4\nline5\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("1:1"))
        })
        .expect("initial position");

    session.send_text(":4").expect("goto line 4");
    session.send_enter().expect("execute goto");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("4:1"))
        })
        .expect("cursor moved to line 4");

    assert!(snapshot.contains("line4"));

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
