use std::process::Command;
use std::time::Duration;
use test_utils::{
    CurrentDirectoryGuard, PtySession, PtySessionConfig, TempFile, TempTree,
    lock_process_environment,
};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_version_flag_prints_version_and_exits() {
    let output = Command::new(ordex_bin())
        .arg("--version")
        .output()
        .expect("run ordex --version");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("ordex v{}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn test_unknown_long_flag_exits_without_opening_editor() {
    let output = Command::new(ordex_bin())
        .arg("--does-not-exist")
        .output()
        .expect("run ordex with unknown flag");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown flag: --does-not-exist"));
    assert!(output.stdout.is_empty());
}

#[test]
fn test_unknown_short_flag_exits_without_opening_editor() {
    let output = Command::new(ordex_bin())
        .arg("-z")
        .output()
        .expect("run ordex with unknown short flag");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown flag: -z"));
    assert!(output.stdout.is_empty());
}

#[test]
fn test_dash_prefixed_file_can_open_after_option_marker() {
    let _env_lock = lock_process_environment();
    let tree = TempTree::with_prefix("ordex_dash_file").expect("create temp tree");
    let path = tree.path().join("--notes");
    std::fs::write(&path, "dash file\n").expect("seed dash-prefixed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &["--", "--notes"],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("--notes")
                && s.row_trimmed_ends_with(1, "dash file")
        })
        .expect("wait for dash-prefixed file");

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_pwd_command_shows_current_working_directory() {
    let _env_lock = lock_process_environment();
    let tree = TempTree::with_prefix("ordex_pwd_command").expect("create temp tree");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");
    session.send_text(":pwd").expect("send pwd command");
    session.send_enter().expect("send enter");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains(&tree.path().display().to_string())
        })
        .expect("pwd should show current directory");

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_open_existing_file_and_quit() {
    let _env_lock = lock_process_environment();
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
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "line 2")
        })
        .expect("wait for initial render");

    assert!(initial.status_line_contains(file.path().display().to_string().as_str()));

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_nonexistent_file_name_is_shown() {
    let _env_lock = lock_process_environment();
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
    assert!(!transcript.contains("Swap recovery unavailable:"));
}

/// Missing working directories should warn once while unnamed swap protection degrades.
#[test]
fn test_deleted_working_directory_warns_once_for_unnamed_swap_degradation() {
    let _env_lock = lock_process_environment();
    let cwd_tree =
        TempTree::with_prefix("ordex_deleted_cwd_unnamed_swap").expect("create temp tree");
    let cwd = cwd_tree.path().join("runtime-cwd");
    std::fs::create_dir_all(&cwd).expect("create runtime cwd");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(cwd.clone()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");
    std::fs::remove_dir(&cwd).expect("delete working directory while ordex is running");

    session.send_text("ix").expect("edit unnamed buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("working directory no longer exists")
        })
        .expect("wait for missing cwd swap warning");

    session.send_text("ia").expect("edit unnamed buffer again");
    session.exit_to_normal_mode(Duration::from_secs(2));

    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let transcript = session.snapshot().raw().to_string();
    assert!(
        transcript.contains(
            "Unnamed swap protection is degraded because the working directory no longer exists"
        ),
        "missing-cwd unnamed-swap warning should be present"
    );
}

/// Quit should still succeed and emit one warning when cwd is deleted after startup.
#[test]
fn test_quit_when_working_directory_is_deleted_after_startup() {
    let _env_lock = lock_process_environment();
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "line")
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

/// Quit warning should be printed after leaving the alternate screen.
#[test]
fn test_quit_warning_prints_after_alternate_screen_teardown() {
    let _env_lock = lock_process_environment();
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line\n").expect("seed file");
    let cwd_tree =
        TempTree::with_prefix("ordex_deleted_cwd_warning_order").expect("create temp tree");
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "line")
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

    // Exercise the user-reported flow: session opened before quit.
    session
        .send_text(":open-session loaded")
        .expect("open current session");
    session.send_enter().expect("execute open session");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && !s.message_line_contains("Error opening session \"loaded\"")
        })
        .expect("wait for session open completion");

    session.clear_transcript();
    std::fs::remove_dir(&cwd).expect("delete working directory while ordex is running");
    session.send_text(":q").expect("quit after deleting cwd");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
    session
        .read_available()
        .expect("drain final transcript bytes");

    let transcript = session.snapshot().raw().to_string();
    let warning = "Warning: skipped autosaving session \"loaded\" on quit because the working directory no longer exists";
    let warning_index = transcript
        .find(warning)
        .expect("warning should be present in PTY transcript");
    let teardown_index = transcript
        .rfind("\u{1b}[?1049l")
        .expect("alternate-screen teardown escape should be present");
    assert!(
        warning_index > teardown_index,
        "warning must be printed after alternate-screen teardown"
    );
}

/// Normal quit with an existing working directory should not emit the autosave-skip warning.
#[test]
fn test_quit_without_deleted_working_directory_does_not_warn() {
    let _env_lock = lock_process_environment();
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "line")
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
