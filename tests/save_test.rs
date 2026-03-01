use std::fs;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_w_writes_file_without_overwrite_confirmation() {
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
    session.exit_to_normal_mode(Duration::from_secs(2));
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
fn test_wq_writes_and_exits_without_overwrite_confirmation() {
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
    session.exit_to_normal_mode(Duration::from_secs(2));
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

#[test]
fn test_w_save_as_cancelled_overwrite_keeps_target_unchanged() {
    let source_file = TempFile::new().expect("create source temp file");
    source_file.write_all(b"base").expect("seed source file");
    let target_file = TempFile::new().expect("create target temp file");
    target_file.write_all(b"target").expect("seed target file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[source_file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "base")
        })
        .expect("wait for initial render");

    session.send_text("i!").expect("insert one char");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "!base")
        })
        .expect("back to normal mode");
    session
        .send_text(&format!(":w {}", target_file.path().to_str().unwrap()))
        .expect("write to target path");
    session.send_enter().expect("execute command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Overwrite") && s.message_line_contains("[y/N]")
        })
        .expect("wait for overwrite prompt");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Write cancelled") && s.status_line_contains("NORMAL |")
        })
        .expect("wait for cancellation message");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let target = fs::read_to_string(target_file.path()).expect("read target file");
    assert_eq!(target, "target");
    let source = fs::read_to_string(source_file.path()).expect("read source file");
    assert_eq!(source, "base");
}

#[test]
fn test_w_bang_bypasses_overwrite_confirmation() {
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
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session.send_text(":w!").expect("force save");
    session.send_enter().expect("execute force save");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("written")
        })
        .expect("wait for written message");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read file after save");
    assert_eq!(saved, "xabc");
}
