use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use test_utils::{TempTree, spawn_lsp_session};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Copy one LSP fixture workspace into a temporary writable directory.
fn copy_workspace_fixture(relative: &str) -> TempTree {
    let source_root = fixture_path(relative);
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        &fs::read_to_string(source_root.join("Cargo.toml")).expect("read Cargo.toml"),
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        &fs::read_to_string(source_root.join("src/main.rs")).expect("read main.rs"),
    )
    .expect("write main.rs");
    tree.write_file(
        "src/lib.rs",
        &fs::read_to_string(source_root.join("src/lib.rs")).expect("read lib.rs"),
    )
    .expect("write lib.rs");
    tree
}

/// Verify rename updates multiple buffers without writing unopened targets to disk.
#[test]
fn test_lsp_rename_updates_open_and_unopened_files() {
    let workspace_root = copy_workspace_fixture("tests/fixtures/lsp/workspace_one");
    let lib_rs = workspace_root.path().join("src/lib.rs");
    let main_rs = workspace_root.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&lib_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
        })
        .expect("wait for lib.rs");

    session
        .send_text("/helper_value() -> i32")
        .expect("search for rename target");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("1:8")
        })
        .expect("cursor should land on the helper_value definition");

    session
        .send_text(":rename helper_total")
        .expect("enter rename command");
    session.send_enter().expect("confirm rename");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(1, "pub fn helper_total() -> i32")
        })
        .expect("rename should update the active buffer");

    session.send_text(":bn").expect("switch to next buffer");
    session.send_enter().expect("execute buffer switch");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.tab_line_contains("main.rs")
                && screen.row_contains(1, "use workspace_one::helper_total;")
                && screen.row_contains(4, "    let _ = helper_total();")
        })
        .expect("rename should update the newly opened target buffer");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute force quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    assert!(
        fs::read_to_string(&lib_rs)
            .expect("read lib.rs after quit")
            .contains("pub fn helper_value() -> i32")
    );
    assert!(
        fs::read_to_string(&main_rs)
            .expect("read main.rs after quit")
            .contains("helper_value")
    );
}
