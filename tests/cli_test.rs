use std::time::Duration;
use test_utils::{
    CurrentDirectoryGuard, PtySession, PtySessionConfig, TempFile, TempTree,
    lock_process_environment,
};

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

/// Quit should still succeed when Ordex inherits a process cwd that no longer exists.
#[test]
fn test_quit_when_started_from_deleted_working_directory() {
    let _env_lock = lock_process_environment();
    let cwd_tree = TempTree::with_prefix("ordex_deleted_cwd_start").expect("create temp tree");
    let deleted_cwd = cwd_tree.path().join("deleted-cwd");
    std::fs::create_dir_all(&deleted_cwd).expect("create startup cwd");
    let _cwd_guard = CurrentDirectoryGuard::change_to(&deleted_cwd);
    std::fs::remove_dir(&deleted_cwd).expect("delete startup cwd");

    let mut session = PtySession::spawn(ordex_bin(), &[], Default::default()).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");
    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let transcript = session.snapshot().raw().to_string();
    assert!(!transcript.contains("Warning: skipped autosaving session"));
}

/// Quit should still succeed and emit one warning when cwd is deleted after startup.
#[test]
fn test_quit_when_working_directory_is_deleted_after_startup() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line\n").expect("seed file");
    let cwd_tree = TempTree::with_prefix("ordex_deleted_cwd_runtime").expect("create temp tree");
    let cwd = cwd_tree.path().join("runtime-cwd");
    std::fs::create_dir_all(&cwd).expect("create runtime cwd");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        PtySessionConfig {
            current_dir: Some(cwd.clone()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "line")
        })
        .expect("wait for initial render");
    session
        .send_text(":save-session loaded")
        .expect("save current session");
    session.send_enter().expect("execute save session");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Session \"loaded\" saved")
        })
        .expect("wait for session save message");

    std::fs::remove_dir(&cwd).expect("delete working directory while ordex is running");
    session.send_text(":q").expect("quit after deleting cwd");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let transcript = session.snapshot().raw().to_string();
    assert!(transcript.contains("Warning: skipped autosaving session \"loaded\" on quit"));
}

/// Normal quit with an existing working directory should not emit the autosave-skip warning.
#[test]
fn test_quit_without_deleted_working_directory_does_not_warn() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "line")
        })
        .expect("wait for initial render");
    session
        .send_text(":save-session loaded")
        .expect("save current session");
    session.send_enter().expect("execute save session");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Session \"loaded\" saved")
        })
        .expect("wait for session save message");

    session.send_text(":q").expect("quit with existing cwd");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let transcript = session.snapshot().raw().to_string();
    assert!(!transcript.contains("Warning: skipped autosaving session"));
}
