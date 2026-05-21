use std::process::Command;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempTree, filtered_path_with_real_binaries};

/// Return the compiled ordex binary path for PTY-backed integration tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Initialize a git repository inside `tree` so ignored files may be exercised.
fn init_git_repo(tree: &TempTree) {
    let status = Command::new("git")
        .current_dir(tree.path())
        .args(["init", "-q"])
        .status()
        .expect("run git init");
    assert!(status.success());
}

#[test]
/// The async search picker should open immediately, stream visible matches, and jump on Enter.
fn test_search_picker_streams_results_and_opens_match() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "fn helper() {}\ntarget_value();\n")
        .expect("write main file");
    tree.write_file("src/lib.rs", "pub fn target_value() {}\n")
        .expect("write second match file");
    tree.write_file(".hidden/secret.rs", "target_value();\n")
        .expect("write hidden match");
    tree.write_file(".gitignore", "ignored.log\n")
        .expect("write gitignore");
    tree.write_file("ignored.log", "target_value\n")
        .expect("write ignored match");
    init_git_repo(&tree);

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
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(":grep target_value")
        .expect("enter grep command");
    session.send_enter().expect("execute grep command");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.contains("Search Results")
                && screen.contains("Filter:")
        })
        .expect("search picker should open immediately");

    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.contains("src/main.rs:2:1: target_value();")
                && screen.contains("src/lib.rs:1:8: pub fn target_value() {}")
                && !screen.contains("ignored.log")
                && !screen.contains(".hidden/secret.rs")
        })
        .expect("wait for streamed search results");

    session.send_enter().expect("open selected search result");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            let opened_main = screen.tab_line_contains("main.rs")
                && screen.status_line_contains("2:1")
                && screen.row_contains(2, "target_value();");
            let opened_lib = screen.tab_line_contains("lib.rs")
                && screen.status_line_contains("1:8")
                && screen.row_contains(1, "pub fn target_value() {}");
            screen.status_line_contains("NORMAL ") && (opened_main || opened_lib)
        })
        .expect("open selected file match");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Picker-side fuzzy filtering should narrow streamed results without restarting the search.
fn test_search_picker_fuzzy_filters_streamed_results() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "target_value();\n")
        .expect("write main file");
    tree.write_file("tests/helper.rs", "target_value();\n")
        .expect("write helper file");

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
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(":gr target_value")
        .expect("enter grep alias");
    session.send_enter().expect("execute grep alias");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.contains("src/main.rs:1:1: target_value();")
                && screen.contains("tests/helper.rs:1:1: target_value();")
        })
        .expect("wait for initial search results");

    session.send_text("helper").expect("fuzzy-filter results");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.contains("Filter: helper")
                && screen.contains("tests/helper.rs:1:1: target_value();")
        })
        .expect("picker should narrow to helper match");

    session.send_enter().expect("open filtered result");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "target_value();")
                && screen.tab_line_contains("helper.rs")
        })
        .expect("open fuzzy-filtered match");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// The normal-mode `<Space>/` binding should open command mode prefilled with `:grep `.
fn test_space_slash_prompts_prefilled_grep_command() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "target_value();\n")
        .expect("write visible match");

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
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(" /")
        .expect("trigger prefilled grep command");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("COMMAND ") && screen.message_line_contains(":grep")
        })
        .expect("space slash should prefill grep command");

    session
        .send_text("target_value")
        .expect("finish grep command text");
    session.send_enter().expect("execute grep command");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.contains("Search Results")
                && screen.contains("src/main.rs:1:1: target_value();")
        })
        .expect("prefilled command should open search results");

    session.send_escape().expect("close picker");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// The search picker should fall back to grep when ripgrep is absent and still skip hidden or ignored paths.
fn test_search_picker_falls_back_to_grep_without_rg() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "target_value();\n")
        .expect("write visible match");
    tree.write_file(".hidden/secret.rs", "target_value();\n")
        .expect("write hidden match");
    tree.write_file(".gitignore", "ignored.log\n")
        .expect("write gitignore");
    tree.write_file("ignored.log", "target_value();\n")
        .expect("write ignored match");
    init_git_repo(&tree);

    let tool_path = filtered_path_with_real_binaries(&tree, &["grep", "git"]);
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            env: vec![("PATH".to_string(), tool_path)],
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(":grep target_value")
        .expect("enter grep command");
    session.send_enter().expect("execute grep command");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.contains("src/main.rs:1:1: target_value();")
                && !screen.contains(".hidden/secret.rs")
                && !screen.contains("ignored.log")
        })
        .expect("grep fallback should surface only visible non-ignored matches");

    session.send_enter().expect("open grep fallback result");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "target_value();")
                && screen.tab_line_contains("main.rs")
        })
        .expect("open grep fallback match");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// The recursive grep fallback should find visible matches without traversing hidden paths.
fn test_search_picker_recursive_grep_fallback_without_git() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "target_value();\n")
        .expect("write visible match");
    tree.write_file(".hidden/secret.rs", "target_value();\n")
        .expect("write hidden match");

    // Remove both rg and git so the picker exercises the recursive grep fallback directly.
    let tool_path = filtered_path_with_real_binaries(&tree, &["grep"]);
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            env: vec![("PATH".to_string(), tool_path)],
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(":grep target_value")
        .expect("enter grep command");
    session.send_enter().expect("execute grep command");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.contains("src/main.rs:1:1: target_value();")
                && !screen.contains(".hidden/secret.rs")
        })
        .expect("recursive grep fallback should surface only visible matches");

    session.send_enter().expect("open recursive grep fallback result");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "target_value();")
                && screen.tab_line_contains("main.rs")
        })
        .expect("open recursive grep fallback match");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
