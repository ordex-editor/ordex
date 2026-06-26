mod session_test_support;
mod swap_test_support;

use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree};

/// Multi-file startup should only create a swap file for the active buffer.
///
/// Inactive buffers opened via CLI arguments must not produce swap files until
/// the user switches to them.
#[test]
fn multi_file_startup_defers_swap_for_inactive_buffers() {
    let first = TempFile::with_suffix("_defer_first.txt").expect("create first file");
    first.write_all(b"first").expect("seed first file");
    let second = TempFile::with_suffix("_defer_second.txt").expect("create second file");
    second.write_all(b"second").expect("seed second file");
    let cache_root = TempTree::with_prefix("ordex_swap_defer_startup").expect("temp cache tree");

    let mut session = PtySession::spawn(
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

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");

    swap_test_support::wait_for_swap_file(&mut session, first.path());
    swap_test_support::assert_no_swap_file(&mut session, second.path());

    session.send_text(":bn").expect("switch to second buffer");
    session.send_enter().expect("execute buffer switch");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer");

    swap_test_support::wait_for_swap_file(&mut session, second.path());

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Session restore should only create a swap file for the active buffer.
///
/// Buffers restored from a project session must not produce swap files until
/// the user switches to them.
#[test]
fn session_restore_defers_swap_for_inactive_buffers() {
    let first = TempFile::with_suffix("_defer_sess_first.txt").expect("create first file");
    first.write_all(b"first").expect("seed first file");
    let second = TempFile::with_suffix("_defer_sess_second.txt").expect("create second file");
    second.write_all(b"second").expect("seed second file");
    let cache_root = TempTree::with_prefix("ordex_swap_defer_session").expect("temp cache tree");
    let session_name = format!(
        "defer_swap_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );

    let mut save_session = PtySession::spawn(
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
    .expect("spawn session saver");
    save_session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");
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
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first restored buffer");

    swap_test_support::wait_for_swap_file(&mut reopen, first.path());
    swap_test_support::assert_no_swap_file(&mut reopen, second.path());

    reopen.send_text(":bn").expect("switch to second buffer");
    reopen.send_enter().expect("execute buffer switch");
    reopen
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer");

    swap_test_support::wait_for_swap_file(&mut reopen, second.path());

    reopen.send_text(":q!").expect("quit");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Closing the active buffer should load swap state for the next buffer.
///
/// After `:bd`, the replacement active buffer must have its swap initialized
/// so edits are protected even if the buffer was never visited before.
#[test]
fn close_buffer_loads_swap_for_next_active() {
    let first = TempFile::with_suffix("_defer_close_first.txt").expect("create first file");
    first.write_all(b"first").expect("seed first file");
    let second = TempFile::with_suffix("_defer_close_second.txt").expect("create second file");
    second.write_all(b"second").expect("seed second file");
    let cache_root = TempTree::with_prefix("ordex_swap_defer_close").expect("temp cache tree");

    let mut session = PtySession::spawn(
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
    .expect("spawn session");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "first")
        })
        .expect("wait for first buffer");

    session.send_text(":bd").expect("close first buffer");
    session.send_enter().expect("execute buffer delete");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "second")
        })
        .expect("wait for second buffer after close");

    session.send_text("iX").expect("edit second buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "Xsecond")
        })
        .expect("wait for edit");

    swap_test_support::wait_for_swap_file(&mut session, second.path());

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// The `:edit` command replacing the default unnamed buffer should create a swap file.
///
/// When `:edit` opens a file into the initial unnamed buffer slot, swap state
/// must be loaded for the new file path.
#[test]
fn edit_command_loads_swap_for_replaced_buffer() {
    let target = TempFile::with_suffix("_defer_edit_target.txt").expect("create target file");
    target.write_all(b"target").expect("seed target file");
    let cache_root = TempTree::with_prefix("ordex_swap_defer_edit").expect("temp cache tree");

    let mut session = PtySession::spawn(
        session_test_support::ordex_bin(),
        &[],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn empty session");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for normal mode");

    session
        .send_text(&format!(
            ":e {}",
            target.path().to_str().expect("target path utf8")
        ))
        .expect("edit target file");
    session.send_enter().expect("execute edit");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "target")
        })
        .expect("wait for target buffer");

    swap_test_support::wait_for_swap_file(&mut session, target.path());

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
