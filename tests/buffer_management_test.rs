use std::fs;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_multiple_startup_files_support_buffer_switching_commands() {
    let first = TempFile::new().expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::new().expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "first buffer")
        })
        .expect("wait for first startup buffer");

    session.send_text(":bn").expect("switch to next buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "second buffer")
        })
        .expect("wait for second buffer");

    session.send_text(":bp").expect("switch to previous buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "first buffer")
        })
        .expect("wait for first buffer again");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_buffer_delete_prompts_for_dirty_buffer_and_closes_after_discard() {
    let first = TempFile::new().expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::new().expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "first buffer")
        })
        .expect("first buffer visible");

    session.send_text(":bn").expect("switch to second buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "second buffer")
        })
        .expect("second buffer visible");

    session.send_text("ix").expect("modify second buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "xsecond buffer")
        })
        .expect("dirty second buffer visible");

    session.send_text(":bd").expect("delete dirty buffer");
    session.send_enter().expect("execute delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("before closing")
                && s.message_line_contains("[y]es/[n]o/[c]ancel")
        })
        .expect("wait for close prompt");

    session.send_text("n").expect("discard changes and close");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "first buffer")
        })
        .expect("switched back to first buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_quit_walks_each_dirty_buffer_before_exiting() {
    let first = TempFile::new().expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::new().expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "first buffer")
        })
        .expect("first buffer visible");

    session.send_text("ia").expect("modify first buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "afirst buffer")
        })
        .expect("dirty first buffer visible");

    session.send_text(":bn").expect("switch to second buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "second buffer")
        })
        .expect("second buffer visible");

    session.send_text("ib").expect("modify second buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "bsecond buffer")
        })
        .expect("dirty second buffer visible");

    session.send_text(":q").expect("request quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains(second.path().file_name().unwrap().to_str().unwrap())
        })
        .expect("prompt for active dirty buffer");

    session.send_text("n").expect("discard active buffer");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains(first.path().file_name().unwrap().to_str().unwrap())
        })
        .expect("prompt for remaining dirty buffer");

    session.send_text("n").expect("discard final dirty buffer");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit after resolving all dirty buffers");

    assert_eq!(
        fs::read_to_string(first.path()).expect("read first file"),
        "first buffer\n"
    );
    assert_eq!(
        fs::read_to_string(second.path()).expect("read second file"),
        "second buffer\n"
    );
}
