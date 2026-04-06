mod session_test_support;
mod swap_test_support;

use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig};
use test_utils::{TempFile, TempTree};

/// Wait for at least one unnamed-buffer swap file under `cache_root`.
fn wait_for_unnamed_swap_file(cache_root: &std::path::Path) -> std::path::PathBuf {
    let unnamed_dir = swap_test_support::swap_dir(cache_root).join("unnamed");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Ok(entries) = std::fs::read_dir(&unnamed_dir)
            && let Some(path) = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension().and_then(|extension| extension.to_str()) == Some("swp")
                })
        {
            return path;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "unnamed swap file did not appear under {}",
        unnamed_dir.display()
    );
}

#[test]
fn restores_unsaved_edits_after_crash() {
    let file = TempFile::new().expect("create temp file");
    let cache_root = TempTree::with_prefix("ordex_swap_recovery_cache").expect("temp tree");
    file.write_all(b"base").expect("seed file");

    let mut session = session_test_support::open_session(&file, Some(cache_root.path()));
    session_test_support::wait_normal_mode(&mut session);
    session.send_text("ix").expect("edit file");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "xbase")
        })
        .expect("wait for unsaved edit");
    swap_test_support::wait_for_swap_file(session.cache_root(), file.path());

    session.send_signal(libc::SIGKILL).expect("kill ordex");
    let status = session
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    let mut reopen = session_test_support::open_session(&file, Some(cache_root.path()));
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
}

#[test]
fn restores_unnamed_buffer_edits_after_crash() {
    let cache_root = TempTree::with_prefix("ordex_unnamed_recovery_cache").expect("temp tree");

    let mut session = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn unnamed ordex");
    session_test_support::wait_normal_mode(&mut session);
    session.send_text("iunnamed").expect("edit unnamed buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "unnamed")
        })
        .expect("wait for unnamed edit");
    let unnamed_swap_path = wait_for_unnamed_swap_file(session.cache_root());

    session
        .send_signal(libc::SIGKILL)
        .expect("kill unnamed ordex");
    let status = session
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    let mut reopen = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("respawn unnamed ordex");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery data exists")
        })
        .expect("wait for unnamed recovery prompt");
    reopen.send_text("r").expect("restore unnamed recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_contains(1, "unnamed")
                && screen.message_line_contains("Recovered unsaved work")
        })
        .expect("wait for unnamed restored buffer");
    reopen.send_text(":q!").expect("quit unnamed recovery");
    reopen.send_enter().expect("execute unnamed quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit unnamed cleanly");
    assert!(
        !unnamed_swap_path.exists(),
        "restored unnamed recovery should delete the stale swap on forced quit"
    );
}
