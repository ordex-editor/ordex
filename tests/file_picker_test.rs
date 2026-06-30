mod swap_test_support;

use std::process::Command;
use std::time::{Duration, Instant};
use test_utils::{PtySession, PtySessionConfig, TempTree};

// Render time on macOS debug builds can reach ~100 ms per frame, so the
// latency budget must be large enough to accommodate one full render cycle
// plus a background-poll interval on top of the actual processing time.
#[cfg(target_os = "macos")]
const ROOT_SCAN_QUERY_LATENCY: Duration = Duration::from_millis(1000);
#[cfg(not(target_os = "macos"))]
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

/// Initialize one Git repository at `path` for picker integration tests.
fn init_git_repository(path: &std::path::Path) {
    let status = Command::new("git")
        .current_dir(path)
        .args(["init", "-q"])
        .status()
        .expect("run git init");
    assert!(status.success());
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

    init_git_repository(tree.path());

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
            s.row_trimmed_ends_with(1, "fn main() {}")
        })
        .expect("open selected file");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify bracketed paste filters the file picker as one flattened query.
#[test]
fn test_file_picker_bracketed_paste_flattens_query_lines() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "fn main() {}\n")
        .expect("write target file");
    tree.write_file("src/lib.rs", "pub fn lib() {}\n")
        .expect("write sibling file");

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
            s.status_line_contains("NORMAL ") && s.contains("src/main.rs")
        })
        .expect("wait for async file-picker results");

    session
        .send_raw_bytes(b"\x1b[200~src\nmain\x1b[201~")
        .expect("send bracketed paste");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("Open: src main")
                && s.any_row_contains("src/main.rs")
        })
        .expect("file-picker paste should flatten lines and filter matches");

    session.send_escape().expect("close picker");
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

    init_git_repository(tree.path());

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

    #[cfg(target_os = "macos")]
    const QUERY_LATENCY: u64 = 1000;
    #[cfg(not(target_os = "macos"))]
    const QUERY_LATENCY: u64 = 200;

    session
        .send_text(" fneedle")
        .expect("open file picker and type filter");
    session
        .wait_until(Duration::from_millis(QUERY_LATENCY), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open: needle")
        })
        .expect("query should render before the full scan finishes");
    session
        .wait_until(Duration::from_secs(15), |s| {
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
                && !s.any_row_contains("Open: alpha omega")
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

    init_git_repository(tree.path());
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

/// Verify that nested repository files are shown by the file picker.
#[test]
fn test_file_picker_shows_files_from_nested_repository_directory() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("reproducer-memchr/src/main.rs", "fn main() {}\n")
        .expect("write nested source file");
    tree.write_file("test-backend/lib.rs", "pub fn backend() {}\n")
        .expect("write sibling source file");

    init_git_repository(tree.path());
    init_git_repository(&tree.path().join("reproducer-memchr"));

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

    // The picker should include files from nested repositories and regular directories.
    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("reproducer-memchr/src/main.rs")
                && s.contains("test-backend/lib.rs")
        })
        .expect("wait for file picker results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that untracked directory files are shown by the file picker.
#[test]
fn test_file_picker_shows_files_from_untracked_directory() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("unstaged/src/main.rs", "fn main() {}\n")
        .expect("write unstaged source file");
    tree.write_file("visible.txt", "visible\n")
        .expect("write visible file");

    init_git_repository(tree.path());

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

    // Untracked directories should surface their files unless excluded by rules.
    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("unstaged/src/main.rs")
                && s.contains("visible.txt")
        })
        .expect("wait for file picker results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that `.gitignore` `target` exclusions are preserved inside `.ignore` reinclusions.
#[test]
fn test_file_picker_preserves_parent_gitignore_target_exclusion_in_reincluded_directory() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(".gitignore", "ignored-by-gitignore/\ntarget\n")
        .expect("write gitignore file");
    tree.write_file(
        ".ignore",
        "!/ignored-by-gitignore/\n!/ignored-by-gitignore/reincluded/\n",
    )
    .expect("write ignore file");
    tree.write_file(
        "ignored-by-gitignore/reincluded/src/main.rs",
        "fn main() {}\n",
    )
    .expect("write reincluded source file");
    tree.write_file(
        "ignored-by-gitignore/reincluded/target/CACHEDIR.TAG",
        "signature\n",
    )
    .expect("write target cache marker");

    init_git_repository(tree.path());

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

    // Reincluded source files stay visible while inherited `.gitignore` `target` exclusions hide artifacts.
    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("src/main.rs")
                && !s.contains("CACHEDIR.TAG")
        })
        .expect("wait for file picker results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that `target/` exclusions are preserved inside `.ignore` reinclusions.
#[test]
fn test_file_picker_preserves_parent_target_exclusion_in_reincluded_directory() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file(".gitignore", "ignored-by-gitignore/\n")
        .expect("write gitignore file");
    tree.write_file(
        ".ignore",
        "!/ignored-by-gitignore/\n!/ignored-by-gitignore/reincluded/\ntarget/\n",
    )
    .expect("write ignore file");
    tree.write_file(
        "ignored-by-gitignore/reincluded/src/main.rs",
        "fn main() {}\n",
    )
    .expect("write reincluded source file");
    tree.write_file(
        "ignored-by-gitignore/reincluded/target/output.o",
        "object\n",
    )
    .expect("write target output file");

    init_git_repository(tree.path());

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

    // Reincluded source files stay visible while inherited `target/` exclusions still hide artifacts.
    session.send_text(" f").expect("open file picker");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("src/main.rs")
                && !s.contains("output.o")
        })
        .expect("wait for file picker results");

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

/// Verify that picker Ctrl-w deletes across hyphens and other punctuation in one keystroke.
#[test]
fn test_file_picker_ctrl_w_deletes_across_punctuation() {
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

    // Type a hyphenated token then a space-separated token.
    session
        .send_text(" ffoo-bar-baz omega")
        .expect("open picker and type two tokens");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open: foo-bar-baz omega")
        })
        .expect("wait for picker query");

    // Ctrl-w is the Unicode control character 0x17.
    session.send_text("\u{17}").expect("send Ctrl-w");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("Open: foo-bar-baz ")
                && !s.any_row_contains("Open: foo-bar-baz omega")
                && s.contains("Files")
        })
        .expect("Ctrl-w should delete only the last whitespace-separated token");

    // A second Ctrl-w should remove the entire punctuated token foo-bar-baz in one press.
    session.send_text("\u{17}").expect("send second Ctrl-w");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("Open: ")
                && !s.any_row_contains("Open: foo")
                && s.contains("Files")
        })
        .expect("second Ctrl-w should delete the hyphenated token in one go");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that picker Alt-Backspace uses punctuation-aware boundaries and stops at hyphens.
#[test]
fn test_file_picker_alt_backspace_stops_at_hyphens() {
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
        .send_text(" ffoo-bar-baz")
        .expect("open picker and type hyphenated token");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.contains("Open: foo-bar-baz")
        })
        .expect("wait for picker query");

    // Alt-Backspace is encoded as ESC (0x1b) followed by DEL (0x7f).
    session
        .send_text("\u{1b}\u{7f}")
        .expect("send Alt-Backspace");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("Open: foo-bar-")
                && !s.any_row_contains("Open: foo-bar-baz")
                && s.contains("Files")
        })
        .expect("Alt-Backspace should stop at the hyphen, deleting only 'baz'");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// The file picker must replace the default unnamed startup buffer, matching `:edit`.
///
/// Opening a file through the picker on a fresh empty session should not leave a
/// `[No Name]` tab behind; the startup buffer must be reused in place.
#[test]
fn test_file_picker_replaces_empty_startup_buffer() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("target.rs", "fn target() {}\n")
        .expect("write target file");
    init_git_repository(tree.path());

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
        .send_text(" ftarget")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("target.rs")
        })
        .expect("wait for picker result");

    // Give the picker a margin past the 100 ms debounce window so
    // `selected_path` returns the highlighted match on Enter.
    std::thread::sleep(Duration::from_millis(500));

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "fn target() {}")
        })
        .expect("wait for picked buffer");

    session
        .wait_until(Duration::from_secs(1), |s| {
            s.status_line_contains("NORMAL ") && !s.tab_line_contains("[No Name]")
        })
        .expect("startup unnamed buffer must be replaced");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// The file picker must create a swap file for a buffer that replaces the startup slot.
///
/// When the picker replaces the unnamed startup buffer with a real file, the swap
/// subsystem must load swap state for the new path — exactly like `:edit`.
#[test]
// FIXME
fn test_file_picker_creates_swap_for_replaced_startup_buffer() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("pkswap.txt", "target body\n")
        .expect("write target file");
    init_git_repository(tree.path());
    let cache_root = TempTree::with_prefix("ordex_picker_swap_replace").expect("temp cache tree");

    let target_path = tree.path().join("pkswap.txt");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            cache_root: Some(cache_root.path().to_path_buf()),
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn empty session");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(" fpkswap")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("pkswap")
        })
        .expect("wait for picker result");

    // Give the picker a margin past the 100 ms debounce window so
    // `selected_path` returns the highlighted match on Enter.
    std::thread::sleep(Duration::from_millis(500));

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "target body")
        })
        .expect("wait for picked buffer");

    swap_test_support::wait_for_swap_file(&mut session, &target_path);

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// The file picker must not replace a modified startup buffer.
///
/// Once the user has typed into the unnamed startup buffer, it is no longer the
/// pristine default; the picker must open the picked file as a new buffer and
/// preserve the modified unnamed buffer as an inactive tab.
#[test]
fn test_file_picker_does_not_replace_modified_startup_buffer() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("other.rs", "fn other() {}\n")
        .expect("write other file");
    init_git_repository(tree.path());

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
        .send_text("istartup content")
        .expect("type into unnamed buffer");
    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.any_row_contains("startup content")
        })
        .expect("wait for typed content");

    session
        .send_text(" fother")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("other.rs")
        })
        .expect("wait for picker result");

    // Give the picker a margin past the 100 ms debounce window so
    // `selected_path` returns the highlighted match on Enter.
    std::thread::sleep(Duration::from_millis(500));

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "fn other() {}")
        })
        .expect("wait for picked buffer");

    session
        .wait_until(Duration::from_secs(1), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains("[No Name]")
                && s.tab_line_contains("other.rs")
        })
        .expect("modified unnamed buffer must be preserved alongside picked file");

    session.send_text(":bp").expect("switch to previous buffer");
    session.send_enter().expect("execute buffer switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.any_row_contains("startup content")
        })
        .expect(":bp must return to the modified unnamed buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// The file picker must open the selection as a new buffer when a non-startup buffer is active.
///
/// When the session was launched with a file argument, the active buffer is a real
/// file rather than the unnamed startup slot, so the picker must add the picked
/// file as a second buffer instead of replacing the current one.
#[test]
fn test_file_picker_adds_buffer_when_launched_with_file() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("first.rs", "fn first() {}\n")
        .expect("write first file");
    tree.write_file("second.rs", "fn second() {}\n")
        .expect("write second file");
    init_git_repository(tree.path());

    let first_path = tree.path().join("first.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[first_path.to_str().expect("first path utf8")],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "fn first() {}")
        })
        .expect("wait for first buffer");

    session
        .send_text(" fsecond")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("second.rs")
        })
        .expect("wait for picker result");

    // Give the picker a margin past the 100 ms debounce window so
    // `selected_path` returns the highlighted match on Enter.
    std::thread::sleep(Duration::from_millis(500));

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "fn second() {}")
        })
        .expect("wait for picked buffer");

    session
        .wait_until(Duration::from_secs(1), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains("first.rs")
                && s.tab_line_contains("second.rs")
        })
        .expect("both buffers must be present after picker confirm");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// The file picker must reactivate an existing inactive buffer instead of duplicating it.
///
/// When the picked path already has a parked inactive buffer (here, one opened via
/// a CLI argument), the picker must switch to it without creating a second entry.
#[test]
fn test_file_picker_reactivates_inactive_buffer_with_same_path() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("alpha.rs", "fn alpha() {}\n")
        .expect("write alpha file");
    tree.write_file("beta.rs", "fn beta() {}\n")
        .expect("write beta file");
    init_git_repository(tree.path());

    let alpha_path = tree.path().join("alpha.rs");
    let beta_path = tree.path().join("beta.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            alpha_path.to_str().expect("alpha path utf8"),
            beta_path.to_str().expect("beta path utf8"),
        ],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "fn alpha() {}")
        })
        .expect("wait for first buffer");

    session
        .send_text(" fbeta")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("beta.rs")
        })
        .expect("wait for picker result");

    // Give the picker a margin past the 100 ms debounce window so
    // `selected_path` returns the highlighted match on Enter.
    std::thread::sleep(Duration::from_millis(500));

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "fn beta() {}")
        })
        .expect("wait for beta buffer");

    session
        .wait_until(Duration::from_secs(1), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_count("alpha.rs") == 1
                && s.tab_line_count("beta.rs") == 1
        })
        .expect("exactly one alpha tab and one beta tab; no duplicate beta buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Confirming the file picker on the currently active file must be a no-op.
///
/// Selecting the file that is already open must not spawn a duplicate buffer or
/// otherwise disturb the active state.
#[test]
// FIXME
fn test_file_picker_confirming_same_file_is_noop() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("only.rs", "fn only() {}\n")
        .expect("write only file");
    init_git_repository(tree.path());

    let only_path = tree.path().join("only.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[only_path.to_str().expect("only path utf8")],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "fn only() {}")
        })
        .expect("wait for only buffer");

    session
        .send_text(" fonly")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ") && s.contains("only.rs")
        })
        .expect("wait for picker result");

    // Give the picker a margin past the 100 ms debounce window so
    // `selected_path` returns the highlighted match on Enter.
    std::thread::sleep(Duration::from_millis(500));

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "fn only() {}")
                && s.status_line_contains("NORMAL ")
                && s.tab_line_count("only.rs") == 1
        })
        .expect("only buffer still active after confirm and no duplicate tab was added");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
