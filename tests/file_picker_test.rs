use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use test_utils::{PtySession, PtySessionConfig, TempTree};

const ROOT_SCAN_QUERY_LATENCY: Duration = Duration::from_millis(100);
const ROOT_SCAN_SETTLE_DURATION: Duration = Duration::from_secs(10);

/// Return the compiled ordex binary path for PTY-backed integration tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the disk root used for integration tests that need a large real scan target.
fn disk_root() -> std::path::PathBuf {
    std::path::PathBuf::from("/")
}

/// Initialize a Git repository at `path`.
fn init_git_repo(path: &Path) {
    let status = Command::new("git")
        .current_dir(path)
        .args(["init", "-q"])
        .status()
        .expect("run git init");
    assert!(status.success());
}

/// Write a nested-repository fixture into `tree`.
fn write_nested_git_root_fixture(tree: &TempTree) {
    // The anchored parent rule verifies that nested scans preserve root-relative
    // paths when consulting parent Git ignore files.
    tree.write_file(
        ".gitignore",
        "test-backend/\n/test-backend/mylib/target/CACHEDIR.TAG\n",
    )
    .expect("write parent gitignore");
    tree.write_file(".ignore", "!/test-backend\n")
        .expect("write picker unignore");
    tree.write_file("src/main.rs", "fn main() {}\n")
        .expect("write visible source file");
    tree.write_file("test-backend/mylib/src/lib.rs", "pub fn nested() {}\n")
        .expect("write nested repo file");
    tree.write_file("test-backend/mylib/target/CACHEDIR.TAG", "cache marker\n")
        .expect("write nested target marker");
}

/// Type one picker query character and assert the rendered query catches up quickly.
fn assert_picker_query_char(
    session: &mut PtySession,
    expected_query: &mut String,
    next_char: char,
) {
    session.clear_transcript();
    let started = Instant::now();
    session
        .send_text(&next_char.to_string())
        .expect("type filter character");
    expected_query.push(next_char);
    session
        .wait_until(ROOT_SCAN_QUERY_LATENCY, |s| {
            s.status_line_contains("NORMAL ") && s.contains(&format!("Open: {expected_query}"))
        })
        .expect("query should update within the latency budget");
    assert!(started.elapsed() <= ROOT_SCAN_QUERY_LATENCY);
}

/// Send Alt plus one ASCII key using the PTY's `Esc` prefix encoding.
fn send_alt_key(session: &mut PtySession, key: char) {
    session.clear_transcript();
    session
        .send_text(&format!("\u{1b}{key}"))
        .expect("send Alt-modified key");
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
                && s.contains("Preview")
                && s.contains("fn main() {}")
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

/// Verify that `.ignore` can re-include files hidden by `.gitignore`.
#[test]
fn test_file_picker_ignore_negation_can_reinclude_gitignored_file() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(".gitignore", "ignored.log\n")
        .expect("write gitignore");
    tree.write_file(".ignore", "!ignored.log\n")
        .expect("write ignore");
    tree.write_file("ignored.log", "ignored\n")
        .expect("write gitignored file");
    tree.write_file("visible.txt", "visible\n")
        .expect("write visible file");

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

    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("visible.txt")
                && s.contains("ignored.log")
        })
        .expect("wait for picker results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that `.ignore` rules are honored during non-Git fallback scans.
#[test]
fn test_file_picker_honors_ignore_rules_without_git() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(".ignore", "*.tmp\nnested/build/\n")
        .expect("write ignore");
    tree.write_file("src/main.rs", "fn main() {}\n")
        .expect("write visible file");
    tree.write_file("src/cache.tmp", "tmp\n")
        .expect("write ignored extension file");
    tree.write_file("nested/build/generated.rs", "pub fn generated() {}\n")
        .expect("write ignored directory file");
    tree.write_file("nested/keep.rs", "pub fn keep() {}\n")
        .expect("write visible nested file");

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

    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("src/main.rs")
                && s.contains("nested/keep.rs")
                && !s.contains("src/cache.tmp")
                && !s.contains("nested/build/generated.rs")
        })
        .expect("wait for fallback scan results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that non-Git picker scans load parent `.gitignore` and `.ignore` files.
#[test]
fn test_file_picker_honors_parent_ignore_files_without_git() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(".gitignore", "*.log\n")
        .expect("write parent gitignore");
    tree.write_file(".ignore", "*.tmp\n")
        .expect("write parent ignore");
    tree.write_file("workspace/src/main.rs", "fn main() {}\n")
        .expect("write visible file");
    tree.write_file("workspace/cache.tmp", "tmp\n")
        .expect("write parent-ignore-hidden file");
    tree.write_file("workspace/debug.log", "log\n")
        .expect("write parent-gitignore-hidden file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(tree.path().join("workspace")),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("src/main.rs")
                && !s.contains("cache.tmp")
                && !s.contains("debug.log")
        })
        .expect("wait for parent ignore results");

    session.send_escape().expect("close picker");
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

/// Verify that picker input stays responsive even after the disk-root scan has run for a while.
#[test]
fn test_file_picker_processes_input_within_reasonable_latency_during_root_scan() {
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(disk_root()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session.send_text(" f").expect("open picker");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open:") && s.contains("/")
        })
        .expect("wait for picker query");

    // Let the disk-root scan accumulate a large result set before typing.
    std::thread::sleep(ROOT_SCAN_SETTLE_DURATION);

    let mut expected_query = String::new();
    for next_char in ['c', 'a', 'r'] {
        assert_picker_query_char(&mut session, &mut expected_query, next_char);
    }

    // Pause between bursts so the test covers repeated responsiveness, not one contiguous write.
    std::thread::sleep(Duration::from_millis(80));

    for next_char in ['g', 'o'] {
        assert_picker_query_char(&mut session, &mut expected_query, next_char);
    }

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that Alt-d edits the picker query instead of closing the popup.
#[test]
fn test_file_picker_alt_d_deletes_forward_word_without_closing_popup() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("fixture.txt", "fixture\n")
        .expect("write picker fixture");

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
        .send_text(" falpha omega")
        .expect("open picker and type two words");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open: alpha omega")
        })
        .expect("wait for picker query");

    session
        .send_text("\u{1b}b")
        .expect("move cursor by one word");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open: alpha omega")
        })
        .expect("wait for cursor movement frame");

    send_alt_key(&mut session, 'd');
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("Open: alpha ")
                && !s.contains("Open: alpha omega")
                && s.contains("Files")
        })
        .expect("Alt-d should edit the query without closing the picker");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that the file picker does not render directory-only Git index entries.
#[test]
fn test_file_picker_does_not_show_git_submodule_directory_entries() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "fn main() {}\n")
        .expect("write visible source file");
    std::fs::create_dir_all(tree.path().join("vendor")).expect("create submodule directory");

    Command::new("git")
        .current_dir(tree.path())
        .args(["init", "-q"])
        .status()
        .expect("run git init");
    Command::new("git")
        .current_dir(tree.path())
        .args([
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,0123456789012345678901234567890123456789,vendor",
        ])
        .status()
        .expect("write gitlink entry");

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

    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("src/main.rs") && !s.contains("vendor")
        })
        .expect("wait for file picker results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that `.ignore` unignores nested repos while nested `target/` files remain hidden.
#[test]
fn test_file_picker_shows_unignored_nested_git_roots_without_nested_target_files() {
    let tree = TempTree::new().expect("create temp tree");
    write_nested_git_root_fixture(&tree);
    init_git_repo(tree.path());
    init_git_repo(&tree.path().join("test-backend/mylib"));

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

    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("test-backend/mylib/src/lib.rs")
                && !s.contains("test-backend/mylib/target/CACHEDIR.TAG")
        })
        .expect("wait for nested git picker results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that `.ignore` can restore ordinary Git-ignored directories.
#[test]
fn test_file_picker_shows_file_in_unignored_gitignored_directory() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(".gitignore", "test-backend\n")
        .expect("write parent gitignore");
    tree.write_file(".ignore", "!/test-backend\n")
        .expect("write picker unignore");
    tree.write_file("test-backend/src/main.rs", "fn main() {}\n")
        .expect("write restored directory file");
    init_git_repo(tree.path());

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

    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("test-backend/src/main.rs")
        })
        .expect("wait for restored directory picker result");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that long picker paths keep the filename tail visible with a leading ellipsis.
#[test]
fn test_file_picker_long_paths_trim_from_start_and_keep_filename_visible() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(
        "very/deep/nested/path/with/many/segments/that/force/truncation/in/the/picker/very_long_tail_filename_component.rs",
        "fn main() {}\n",
    )
    .expect("write deep fixture");

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
        .send_text(" fvery_long_tail_filename_component")
        .expect("open file picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("very_long_tail_filename_component.rs")
                && s.contains("…")
                && s.contains("Preview")
                && !s.contains("very/deep/nested/path/with/many/segments")
        })
        .expect("wait for truncated long-path rendering");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
