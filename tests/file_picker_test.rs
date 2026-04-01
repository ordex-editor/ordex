use std::process::Command;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempTree};

/// Return the compiled ordex binary path for PTY-backed integration tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Verify that the async file picker lists files, filters results, and opens a selection.
#[test]
fn test_file_picker_filters_visible_files_and_opens_selection() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "fn main() {}\n")
        .expect("write visible source file");
    tree.write_file("notes.txt", "notes\n")
        .expect("write visible text file");
    tree.write_file(".secret", "hidden\n")
        .expect("write hidden file");
    tree.write_file("src/.cache/ignored.txt", "hidden nested\n")
        .expect("write hidden nested file");
    tree.write_file(".gitignore", "ignored.log\n")
        .expect("write gitignore");
    tree.write_file("ignored.log", "ignored\n")
        .expect("write ignored file");

    Command::new("git")
        .current_dir(tree.path())
        .args(["init", "-q"])
        .status()
        .expect("run git init");

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
        .expect("wait for startup frame");

    session
        .send_text(" fmain")
        .expect("open file picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("src/main.rs")
                && !s.contains("ignored.log")
        })
        .expect("wait for async file-picker results");

    session.send_enter().expect("confirm file picker selection");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.row_contains(1, "fn main() {}")
        })
        .expect("open selected file");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that a large filesystem scan still lets the picker query update immediately.
#[test]
fn test_file_picker_stays_responsive_during_large_filesystem_scan() {
    let tree = TempTree::new().expect("create temp tree");
    for dir_index in 0..80 {
        for file_index in 0..40 {
            tree.write_file(
                &format!("dir_{dir_index:03}/file_{file_index:03}.txt"),
                "bulk fixture\n",
            )
            .expect("write bulk fixture");
        }
    }
    tree.write_file("dir_079/needle_target.txt", "target\n")
        .expect("write target file");

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
        .expect("wait for startup frame");

    session
        .send_text(" fneedle")
        .expect("open file picker and type filter");
    session
        .wait_until(Duration::from_millis(200), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open: needle")
        })
        .expect("query should render before the full scan finishes");
    session
        .wait_until(Duration::from_secs(5), |s| {
            s.status_line_contains("NORMAL ") && s.contains("needle_target.txt")
        })
        .expect("wait for scan results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
