use std::fs;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_w_writes_file_and_shows_status_message() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.send_escape().expect("exit insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session.send_text(":w").expect("save");
    session.send_enter().expect("execute save");

    let after_save = session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("written") && s.status_line_contains("NORMAL |")
        })
        .expect("wait for written message");

    assert!(after_save.message_line_contains("written"));

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read file after save");
    assert_eq!(saved, "xabc");
}

#[test]
fn test_wq_writes_and_exits() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"base").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "base")
        })
        .expect("wait for initial render");

    session.send_text("i!").expect("insert one char");
    session.send_escape().expect("exit insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "!base")
        })
        .expect("back to normal mode");
    session.send_text(":wq").expect("write and quit");
    session.send_enter().expect("execute command");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("write and quit should exit");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "!base");
}
