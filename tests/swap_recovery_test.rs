mod session_test_support;
mod swap_test_support;

use std::time::Duration;
use test_utils::TempFile;

#[test]
fn restores_unsaved_edits_after_crash() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"base").expect("seed file");
    swap_test_support::cleanup_swap_for_path(file.path());

    let mut session = session_test_support::open_session(&file);
    session_test_support::wait_normal_mode(&mut session);
    session.send_text("ix").expect("edit file");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "xbase")
        })
        .expect("wait for unsaved edit");
    swap_test_support::wait_for_swap_file(file.path());

    session.send_signal(libc::SIGKILL).expect("kill ordex");
    let status = session
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    let mut reopen = session_test_support::open_session(&file);
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery data exists")
        })
        .expect("wait for recovery prompt");
    reopen.send_text("r").expect("restore recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "xbase")
                && screen.message_line_contains("Recovered unsaved work")
        })
        .expect("wait for restored buffer");
    reopen.send_text(":q!").expect("quit after recovery");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    swap_test_support::cleanup_swap_for_path(file.path());
}
