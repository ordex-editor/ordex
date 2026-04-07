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

/// Verify multiple definition results open the definition picker before any jump.
#[test]
fn test_goto_definition_opens_picker_for_multiple_targets() {
    let workspace_root = fixture_path("tests/fixtures/lsp/workspace_picker");
    let main_rs = workspace_root.join("src/main.rs");
    let mut session = spawn_lsp_session(&main_rs, workspace_root);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main()")
        })
        .expect("wait for main.rs");

    session.send_text("wwgd").expect("request definition");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.contains("Definitions")
                && screen.contains("defs_a.rs")
                && screen.contains("defs_b.rs")
        })
        .expect("definition picker should list both targets");

    session.send_enter().expect("confirm first definition");
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.tab_line_contains("defs_a.rs")
                && screen.row_contains(1, "pub fn chooser() -> usize")
                && screen.status_line_contains("1:8")
        })
        .expect("confirming the picker should open defs_a.rs");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
