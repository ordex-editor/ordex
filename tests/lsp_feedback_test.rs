use std::time::Duration;
use test_utils::{
    PtySession, PtySessionConfig, ScreenSnapshot, TempFile, overlay_footer_hidden,
    overlay_footer_visible,
};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one repository fixture path for PTY-backed LSP tests.
fn fixture_path(relative: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Return the visible overlay footer line, if the footer is currently on screen.
fn overlay_footer_text(screen: &ScreenSnapshot) -> Option<String> {
    (24..=27)
        .find_map(|row| {
            screen
                .row(row)
                .filter(|line| line.contains("rust-analyzer"))
        })
        .map(str::to_string)
}

/// Spawn Ordex for one unsupported-file lookup test rooted at `current_dir`.
fn spawn_lsp_session_in_dir(
    file_path: &std::path::Path,
    current_dir: std::path::PathBuf,
) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file_path.to_str().expect("utf8 fixture path")],
        PtySessionConfig {
            current_dir: Some(current_dir),
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Verify unsupported files stay in place and report a clear error.
#[test]
fn test_goto_definition_reports_unsupported_file() {
    let file = TempFile::with_suffix(".txt").expect("create temp file");
    file.write_all(b"plain text\n").expect("seed file");
    let current_dir = std::env::current_dir().expect("read current directory");
    let mut session = spawn_lsp_session_in_dir(file.path(), current_dir);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "plain text")
        })
        .expect("wait for txt file");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("is not a supported file for built-in LSP")
                && screen.row_trimmed_ends_with(1, "plain text")
        })
        .expect("unsupported-file message should be visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify Rust files outside a supported workspace report a clear error.
#[test]
fn test_goto_definition_reports_unsupported_project() {
    let file = TempFile::with_suffix(".rs").expect("create temp file");
    file.write_all(b"fn main() {}\n").expect("seed file");
    let current_dir = std::env::current_dir().expect("read current directory");
    let mut session = spawn_lsp_session_in_dir(file.path(), current_dir);

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "fn main() {}")
        })
        .expect("wait for rust file");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("is not inside a supported Rust project root")
                && screen.row_trimmed_ends_with(1, "fn main() {}")
        })
        .expect("unsupported-project message should be visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify launch-time LSP work renders and clears the overlay without user actions.
#[test]
fn test_startup_shows_and_clears_lsp_progress_overlay() {
    let main_rs = fixture_path("tests/fixtures/lsp/workspace_one/src/main.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_rs.to_str().expect("utf8 fixture path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_visible(screen)
        })
        .expect("startup LSP progress overlay should become visible");
    let first_footer =
        overlay_footer_text(&session.snapshot()).expect("capture startup overlay footer");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_text(screen).is_some_and(|footer| footer != first_footer)
        })
        .expect("startup LSP progress overlay should update without user actions");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_hidden(screen)
        })
        .expect("startup LSP progress overlay should clear after progress stops");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify live LSP progress renders and clears during a real definition lookup.
#[test]
fn test_goto_definition_shows_and_clears_lsp_progress_overlay() {
    let main_rs = fixture_path("tests/fixtures/lsp/workspace_one/src/main.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_rs.to_str().expect("utf8 fixture path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .send_text("/helper_value\\(\\)")
        .expect("search for unopened-file symbol");
    session.send_enter().expect("confirm search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("4:13")
        })
        .expect("cursor should land on the helper_value call");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_visible(screen)
        })
        .expect("LSP progress overlay should become visible");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.tab_line_contains("lib.rs")
                && screen.row_contains(1, "pub fn helper_value() -> i32")
                && screen.status_line_contains("1:8")
        })
        .expect("definition jump should open lib.rs");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_hidden(screen)
        })
        .expect("LSP progress overlay should clear after definition progress stops");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify launch-time progress updates do not block ordinary cursor motion.
#[test]
fn test_startup_progress_overlay_does_not_block_cursor_motion() {
    let main_rs = fixture_path("tests/fixtures/lsp/workspace_one/src/main.rs");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[main_rs.to_str().expect("utf8 fixture path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "use workspace_one")
        })
        .expect("wait for main.rs");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_visible(screen)
        })
        .expect("startup LSP progress overlay should become visible");

    // Step through several lines so repeated progress updates cannot starve motion.
    for expected_position in ["2:1", "3:1", "4:1", "5:1", "6:1"] {
        session
            .send_text("j")
            .expect("move cursor while overlay is visible");
        session
            .wait_until(Duration::from_secs(2), |screen| {
                overlay_footer_visible(screen) && screen.status_line_contains(expected_position)
            })
            .expect("cursor motion should keep advancing during startup progress");
    }

    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_hidden(screen)
        })
        .expect("startup LSP progress overlay should clear after progress stops");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
