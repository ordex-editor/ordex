mod session_test_support;
mod swap_test_support;

use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig};
use test_utils::{TempFile, TempTree};

/// Build one unique session name for tests that save and reopen project sessions.
fn unique_session_name(prefix: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{prefix}_{stamp}")
}

/// Wait for at least one unnamed-buffer swap file, draining the PTY so ordex can write renders.
fn wait_for_unnamed_swap_file(session: &mut PtySession) -> std::path::PathBuf {
    let swap_dir = swap_test_support::swap_dir(session.cache_root());
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let _ = session.read_available();
        if let Ok(entries) = std::fs::read_dir(&swap_dir)
            && let Some(path) = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| is_unnamed_swap_file(path))
        {
            return path;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "unnamed swap file did not appear under {}",
        swap_dir.display()
    );
}

/// Return whether `path` is an unnamed-buffer swap file by checking the marker prefix.
fn is_unnamed_swap_file(path: &std::path::Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("swp") {
        return false;
    }
    // Swap file names are the URL-encoded full identity path (e.g.
    // `%2Fpath%2F__ordex_unnamed_buffer__.12345`), so look for the marker string
    // anywhere in the file name.
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    stem.contains("__ordex_unnamed_buffer__")
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
            screen.row_trimmed_ends_with(1, "xbase")
        })
        .expect("wait for unsaved edit");
    swap_test_support::wait_for_swap_file(&mut session, file.path());
    swap_test_support::wait_for_swap_body(&mut session, file.path(), "xbase");

    session.send_signal(libc::SIGKILL).expect("kill ordex");
    let status = session
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    let mut reopen = session_test_support::open_session(&file, Some(cache_root.path()));
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for recovery prompt");
    reopen.send_text("r").expect("restore recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "xbase")
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
    let working_dir = TempTree::with_prefix("ordex_unnamed_recovery_cwd").expect("temp tree");

    let mut session = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(working_dir.path().to_path_buf()),
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
            screen.row_trimmed_ends_with(1, "unnamed")
        })
        .expect("wait for unnamed edit");
    let unnamed_swap_path = wait_for_unnamed_swap_file(&mut session);

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
            current_dir: Some(working_dir.path().to_path_buf()),
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("respawn unnamed ordex");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for unnamed recovery prompt");
    reopen.send_text("r").expect("restore unnamed recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "unnamed")
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

/// The `[i] ignore` option should leave the unnamed-buffer swap file on disk and
/// start a fresh empty buffer.
#[test]
fn ignore_keeps_unnamed_swap_file_on_disk() {
    let cache_root = TempTree::with_prefix("ordex_unnamed_ignore_cache").expect("temp tree");
    let working_dir = TempTree::with_prefix("ordex_unnamed_ignore_cwd").expect("temp tree");

    let mut session = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(working_dir.path().to_path_buf()),
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn unnamed session");
    session_test_support::wait_normal_mode(&mut session);
    session
        .send_text("iignore-me")
        .expect("edit unnamed buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "ignore-me")
        })
        .expect("wait for unnamed edit");
    let unnamed_swap_path = wait_for_unnamed_swap_file(&mut session);

    session
        .send_signal(libc::SIGKILL)
        .expect("kill unnamed session");
    let status = session
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    let mut reopen = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(working_dir.path().to_path_buf()),
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("respawn unnamed session");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("[i] ignore")
        })
        .expect("wait for unnamed recovery prompt with ignore option");
    reopen.send_text("i").expect("ignore unnamed recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Swap file left on disk")
        })
        .expect("wait for ignore status message");
    reopen.send_text(":q!").expect("quit after ignore");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
    assert!(
        unnamed_swap_path.exists(),
        "ignored unnamed swap file should remain on disk after quit"
    );
}

/// Deferred startup recovery should prompt each affected buffer when it becomes active.
#[test]
fn defers_startup_multi_file_swap_prompts_until_buffer_activation() {
    let first = TempFile::with_suffix("_swap_multi_first.txt").expect("create first file");
    first.write_all(b"first").expect("seed first file");
    let second = TempFile::with_suffix("_swap_multi_second.txt").expect("create second file");
    second.write_all(b"second").expect("seed second file");
    let cache_root = TempTree::with_prefix("ordex_swap_multi_startup").expect("temp cache tree");

    let mut crash = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[
            first.path().to_str().expect("first file path utf8"),
            second.path().to_str().expect("second file path utf8"),
        ],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn multi-file session");

    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");
    crash.send_text("iA").expect("modify first buffer");
    crash.exit_to_normal_mode(Duration::from_secs(2));
    crash.send_text(":bn").expect("switch to second buffer");
    crash.send_enter().expect("execute buffer switch");
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer");
    crash.send_text("iB").expect("modify second buffer");
    crash.exit_to_normal_mode(Duration::from_secs(2));
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "Bsecond")
        })
        .expect("wait for second edit");
    // Ensure both startup buffers have persisted swap state before simulating a crash.
    swap_test_support::wait_for_swap_file(&mut crash, first.path());
    swap_test_support::wait_for_swap_file(&mut crash, second.path());
    swap_test_support::wait_for_swap_body(&mut crash, second.path(), "Bsecond");
    crash
        .send_signal(libc::SIGKILL)
        .expect("kill crash session");
    let status = crash
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    // Reopen the same startup argument list; only the active buffer prompt should be visible.
    let mut reopen = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[
            first.path().to_str().expect("first file path utf8"),
            second.path().to_str().expect("second file path utf8"),
        ],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("reopen multi-file session");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for first deferred prompt");
    reopen.send_text("d").expect("discard first recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery data discarded")
        })
        .expect("wait for discard status");
    reopen
        .send_text(":bn")
        .expect("switch to second reopened buffer");
    reopen.send_enter().expect("execute second buffer switch");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for second deferred prompt");
    reopen.send_text("r").expect("recover second buffer");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "Bsecond")
                && screen.message_line_contains("Recovered unsaved work")
        })
        .expect("wait for second recovery");
    reopen.send_text(":q!").expect("quit after recovery checks");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Inactive deferred swap prompts should not block force-quit flow.
#[test]
fn force_quit_does_not_require_visiting_inactive_deferred_swap_prompts() {
    let first = TempFile::with_suffix("_swap_quit_first.txt").expect("create first file");
    first.write_all(b"first").expect("seed first file");
    let second = TempFile::with_suffix("_swap_quit_second.txt").expect("create second file");
    second.write_all(b"second").expect("seed second file");
    let cache_root = TempTree::with_prefix("ordex_swap_quit_inactive").expect("temp cache tree");

    let mut crash = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[
            first.path().to_str().expect("first file path utf8"),
            second.path().to_str().expect("second file path utf8"),
        ],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn multi-file session");
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");
    crash.send_text("iA").expect("modify first buffer");
    crash.exit_to_normal_mode(Duration::from_secs(2));
    crash.send_text(":bn").expect("switch to second buffer");
    crash.send_enter().expect("execute buffer switch");
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer");
    crash.send_text("iB").expect("modify second buffer");
    crash.exit_to_normal_mode(Duration::from_secs(2));
    swap_test_support::wait_for_swap_file(&mut crash, first.path());
    swap_test_support::wait_for_swap_file(&mut crash, second.path());
    crash
        .send_signal(libc::SIGKILL)
        .expect("kill crash session");
    let status = crash
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    // Leave the second buffer unresolved and confirm a force quit still succeeds.
    let mut reopen = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[
            first.path().to_str().expect("first file path utf8"),
            second.path().to_str().expect("second file path utf8"),
        ],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("reopen multi-file session");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for first deferred prompt");
    reopen.send_text("d").expect("discard first recovery");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery data discarded")
        })
        .expect("wait for first prompt dismissal");

    reopen
        .send_text(":q!")
        .expect("force quit without visiting second");
    reopen.send_enter().expect("execute force quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit should not require second prompt");
}

/// Opening a saved project session should defer swap prompts until each swapped buffer is active.
#[test]
fn open_session_defers_swap_prompt_until_swapped_buffer_is_activated() {
    let first = TempFile::with_suffix("_swap_session_first.txt").expect("create first file");
    first.write_all(b"first").expect("seed first file");
    let second = TempFile::with_suffix("_swap_session_second.txt").expect("create second file");
    second.write_all(b"second").expect("seed second file");
    let third = TempFile::with_suffix("_swap_session_third.txt").expect("create third file");
    third.write_all(b"third").expect("seed third file");
    let cache_root = TempTree::with_prefix("ordex_swap_session_open").expect("temp cache tree");
    let session_name = unique_session_name("swap_session_defer");

    // Save a session whose active buffer is the second file.
    let mut save_session = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[
            first.path().to_str().expect("first file path utf8"),
            second.path().to_str().expect("second file path utf8"),
            third.path().to_str().expect("third file path utf8"),
        ],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn session saver");
    save_session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");
    save_session
        .send_text(":bn")
        .expect("switch to second buffer");
    save_session.send_enter().expect("execute switch");
    save_session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer");
    save_session
        .send_text(&format!(":save-session {session_name}"))
        .expect("save session");
    save_session.send_enter().expect("execute session save");
    save_session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains(&format!("Session \"{session_name}\" saved"))
        })
        .expect("wait for save message");
    save_session.send_text(":q!").expect("quit saver");
    save_session.send_enter().expect("execute quit");
    save_session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit saver cleanly");

    // Create stale swap data only for the third file.
    let mut crash = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[
            first.path().to_str().expect("first file path utf8"),
            second.path().to_str().expect("second file path utf8"),
            third.path().to_str().expect("third file path utf8"),
        ],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn crash session");
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");
    crash.send_text(":bn").expect("switch to second");
    crash.send_enter().expect("execute second switch");
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer");
    crash.send_text(":bn").expect("switch to third");
    crash.send_enter().expect("execute third switch");
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "third")
        })
        .expect("wait for third buffer");
    crash.send_text("iZ").expect("modify third buffer");
    crash.exit_to_normal_mode(Duration::from_secs(2));
    crash
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "Zthird")
        })
        .expect("wait for third edit");
    swap_test_support::wait_for_swap_file(&mut crash, third.path());
    swap_test_support::wait_for_swap_body(&mut crash, third.path(), "Zthird");
    crash
        .send_signal(libc::SIGKILL)
        .expect("kill crash session");
    let status = crash
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for crash exit");
    assert!(!status.success());

    // Start from an empty editor to avoid startup swap prompts interfering with :open-session.
    let mut reopen = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn reopen session");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for reopen baseline");
    reopen
        .send_text(&format!(":open-session {session_name}"))
        .expect("open saved session");
    reopen.send_enter().expect("execute session open");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
                && screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for restored active buffer prompt");
    reopen.send_text("d").expect("discard second prompt");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery data discarded")
        })
        .expect("wait for second discard");
    reopen.send_text(":bp").expect("switch to first");
    reopen.send_enter().expect("execute first switch");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
                && screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for first buffer prompt");
    reopen.send_text("d").expect("discard first prompt");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery data discarded")
        })
        .expect("wait for first discard");
    reopen.send_text(":bn").expect("return to second");
    reopen.send_enter().expect("execute second switch");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
                && !screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for second buffer without prompt");
    reopen.send_text(":bn").expect("switch to third");
    reopen.send_enter().expect("execute third switch");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("Recovery swap found")
        })
        .expect("wait for deferred third prompt");
    reopen.send_text("r").expect("recover third buffer");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "Zthird")
                && screen.message_line_contains("Recovered unsaved work")
        })
        .expect("wait for recovered third buffer");
    reopen.send_text(":q!").expect("quit reopen session");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit reopen cleanly");
}
