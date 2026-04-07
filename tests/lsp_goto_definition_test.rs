use std::path::PathBuf;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the compiled fake rust-analyzer binary path for PTY-backed LSP tests.
fn fake_rust_analyzer_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fake_rust_analyzer")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Spawn Ordex with the fake rust-analyzer override configured.
fn spawn_lsp_session(file: &std::path::Path, current_dir: PathBuf) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file.to_str().expect("utf8 fixture path")],
        PtySessionConfig {
            current_dir: Some(current_dir),
            env: vec![(
                "ORDEX_RUST_ANALYZER".to_string(),
                fake_rust_analyzer_bin().to_string(),
            )],
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Verify `g d` opens the single definition target returned by rust-analyzer.
#[test]
fn test_goto_definition_opens_single_rust_target() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&main_rs, workspace_root);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session.send_text("wwgd").expect("request definition");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1:8")
        })
        .expect("definition jump should open lib.rs");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
