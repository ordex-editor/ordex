mod swap_test_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use test_utils::{PtySession, TempFile, TempTree};

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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session.send_text(":w").expect("save");
    session.send_enter().expect("execute save");

    let after_save = session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("written") && s.status_line_contains("NORMAL ")
        })
        .expect("wait for written message");

    assert!(after_save.message_line_contains("written"));

    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "base")
        })
        .expect("wait for initial render");

    session.send_text("i!").expect("insert one char");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "!base")
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "base")
        })
        .expect("wait for initial render");

    session.send_text("i!").expect("insert one char");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "!base")
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
            s.message_line_contains("Write cancelled") && s.status_line_contains("NORMAL ")
        })
        .expect("wait for cancellation message");

    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "xabc")
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

#[test]
fn test_q_on_modified_file_prompts_and_n_discards_changes() {
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session.send_text(":q").expect("request quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains("[y]es/[n]o/[c]ancel")
        })
        .expect("wait for quit confirmation prompt");
    session.send_text("n").expect("discard changes");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("discard and quit should exit");

    let saved = fs::read_to_string(file.path()).expect("read file after quit");
    assert_eq!(saved, "abc");
}

#[test]
fn test_open_session_on_modified_buffer_prompts_before_replacing_buffers() {
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session
        .send_text(":open-session demo")
        .expect("request session open");
    session.send_enter().expect("execute command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("before opening session \"demo\"")
                && s.message_line_contains("[y]es/[n]o/[c]ancel")
        })
        .expect("wait for session-open confirmation prompt");

    session.send_text("c").expect("cancel session open");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Session open cancelled") && s.status_line_contains("NORMAL ")
        })
        .expect("wait for cancellation message");

    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_q_bang_on_modified_file_exits_without_saving() {
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("force quit should exit");

    let saved = fs::read_to_string(file.path()).expect("read file after quit");
    assert_eq!(saved, "abc");
}

#[test]
fn test_q_on_modified_file_prompt_y_saves_and_quits() {
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "xabc")
        })
        .expect("back to normal mode");
    session.send_text(":q").expect("request quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains("[y]es/[n]o/[c]ancel")
        })
        .expect("wait for quit confirmation prompt");
    session.send_text("y").expect("save and quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit should exit");

    let saved = fs::read_to_string(file.path()).expect("read file after quit");
    assert_eq!(saved, "xabc");
}

#[test]
fn test_q_on_modified_file_prompt_c_cancels_quit() {
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q").expect("request quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains("[y]es/[n]o/[c]ancel")
        })
        .expect("wait for quit confirmation prompt");
    session.send_text("c").expect("cancel quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Quit cancelled") && s.status_line_contains("NORMAL ")
        })
        .expect("wait for cancel message");

    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read file after quit");
    assert_eq!(saved, "abc");
}

#[test]
fn test_q_on_unnamed_modified_buffer_y_stays_open_with_error() {
    let mut session = PtySession::spawn(ordex_bin(), &[], Default::default()).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q").expect("request quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to \"[No Name]\"? [y]es/[n]o/[c]ancel")
        })
        .expect("wait for quit confirmation prompt");
    session.send_text("y").expect("attempt save and quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("No file name") && s.status_line_contains("NORMAL ")
        })
        .expect("wait for no file name error");

    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute force quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_successful_save_removes_swap_file_after_write() {
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
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    swap_test_support::wait_for_swap_file(session.cache_root(), file.path());

    session.send_text(":w").expect("save");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("written")
        })
        .expect("wait for written message");
    assert!(
        !swap_test_support::swap_path_for_path(session.cache_root(), file.path()).exists(),
        "successful durable save should remove the swap file"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_failed_save_keeps_swap_file_available() {
    let tree = TempTree::new().expect("create temp tree");
    let file_path = tree.path().join("blocked.txt");
    fs::write(&file_path, "abc").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file_path.to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abc")
        })
        .expect("wait for initial render");

    session.send_text("ix").expect("enter insert and type");
    session.exit_to_normal_mode(Duration::from_secs(2));
    swap_test_support::wait_for_swap_file(session.cache_root(), &file_path);

    let original_permissions = fs::metadata(tree.path())
        .expect("read dir metadata")
        .permissions();
    let mut readonly_permissions = original_permissions.clone();
    readonly_permissions.set_mode(0o555);
    fs::set_permissions(tree.path(), readonly_permissions).expect("make directory read-only");

    session.send_text(":w").expect("save");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("Error"))
        .expect("wait for save error");
    assert!(
        swap_test_support::swap_path_for_path(session.cache_root(), &file_path).exists(),
        "failed save should keep the swap file"
    );

    fs::set_permissions(tree.path(), original_permissions).expect("restore directory permissions");
    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
