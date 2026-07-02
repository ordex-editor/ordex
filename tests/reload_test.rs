use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

/// Return the test-built Ordex binary path.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Verify `:edit` without arguments reloads a clean buffer from disk.
#[test]
fn test_edit_without_arguments_reloads_clean_buffer() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"original content\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    // Modify the file on disk without saving in the editor.
    file.write_all(b"new content from disk\n")
        .expect("overwrite file");

    // Reload the buffer using `:edit` without arguments.
    session.send_text(":e").expect("type :e");
    session.send_enter().expect("execute :e");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "new content from disk")
        })
        .expect("buffer should be reloaded");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify `:edit` without arguments shows "No file name" error on unnamed buffer.
#[test]
fn test_edit_without_arguments_shows_error_on_unnamed_buffer() {
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    // Try to reload an unnamed buffer.
    session.send_text(":e").expect("type :e");
    session.send_enter().expect("execute :e");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("No file name")
        })
        .expect("should show no file name error");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify `:edit` without arguments prompts to save when buffer is modified.
#[test]
fn test_edit_without_arguments_prompts_on_modified_buffer() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"original\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    // Modify the buffer without saving.
    session.send_text("i").expect("enter insert mode");
    session.send_text("edit ").expect("type edit text");
    session.exit_to_normal_mode(Duration::from_secs(2));

    // Try to reload - should prompt because buffer is modified.
    session.send_text(":e").expect("type :e");
    session.send_enter().expect("execute :e");

    // The prompt should appear.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("reload")
        })
        .expect("should show reload prompt");

    // Send 'n' to reload without saving
    session.send_text("n").expect("discard changes and reload");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "original")
        })
        .expect("buffer should be reloaded to original content");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify `:edit` without arguments shows reloaded status message.
#[test]
fn test_edit_without_arguments_shows_status_message() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"original\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    // Modify the file on disk.
    file.write_all(b"modified\n").expect("overwrite file");

    // Reload the buffer.
    session.send_text(":e").expect("type :e");
    session.send_enter().expect("execute :e");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("reloaded")
        })
        .expect("should show reloaded status message");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify `:edit` without arguments undoes the reload.
#[test]
fn test_edit_without_arguments_can_be_undone() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"original\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    // Modify the file on disk.
    file.write_all(b"modified\n").expect("overwrite file");

    // Reload the buffer.
    session.send_text(":e").expect("type :e");
    session.send_enter().expect("execute :e");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "modified")
        })
        .expect("buffer should be reloaded");

    // Undo should restore the original content.
    session.send_text("u").expect("undo reload");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "original")
        })
        .expect("buffer should be restored after undo");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify `:edit!` force-reloads a dirty buffer without showing save prompt.
#[test]
fn test_edit_force_reloads_modified_buffer_without_prompt() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"original\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    // Keep a different on-disk payload so a reload can be observed.
    file.write_all(b"from disk\n").expect("overwrite file");

    // Make the in-memory buffer dirty before forcing reload.
    session.send_text("i").expect("enter insert mode");
    session.send_text("dirty ").expect("modify buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));

    // Force reload should discard unsaved edits without prompting.
    session.send_text(":e!").expect("type :e!");
    session.send_enter().expect("execute :e!");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "from disk") && !s.message_line_contains("Save changes to")
        })
        .expect("force reload should discard edits without prompt");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
