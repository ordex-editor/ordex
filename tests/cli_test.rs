use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_open_existing_file_and_quit() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 1\nline 2\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    let initial = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "line 1")
                && s.row_contains(2, "line 2")
        })
        .expect("wait for initial render");

    assert!(initial.status_line_contains(file.path().file_name().unwrap().to_str().unwrap()));

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_nonexistent_file_name_is_shown() {
    let path = format!("/tmp/ordex_e2e_nonexistent_{}.txt", std::process::id());

    let mut session =
        PtySession::spawn(ordex_bin(), &[&path], Default::default()).expect("spawn ordex");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    assert!(snapshot.status_line_contains("ordex_e2e_nonexistent"));

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
