use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use test_utils::{
    PtySession, PtySessionConfig, ScreenSnapshot, TempTree, spawn_lsp_session_with_config,
};

/// Return the compiled ordex binary path for PTY-backed repro tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Copy the save-warning repro fixture into a writable temporary workspace.
fn repro_workspace() -> TempTree {
    let source_root = fixture_path("tests/fixtures/lsp/save_warning_probe");
    let tree = TempTree::new().expect("temp workspace");
    // Each PTY probe edits its own workspace copy so the save repro stays isolated.
    tree.write_file(
        "Cargo.toml",
        &fs::read_to_string(source_root.join("Cargo.toml")).expect("read fixture Cargo.toml"),
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        &fs::read_to_string(source_root.join("src/main.rs")).expect("read fixture main.rs"),
    )
    .expect("write main.rs");
    tree
}

/// Return the main Rust source path for one temporary repro workspace.
fn repro_main_rs(workspace: &TempTree) -> PathBuf {
    workspace.path().join("src/main.rs")
}

/// Spawn Ordex against one temporary repro workspace with isolated cache and environment.
fn spawn_repro_session(
    workspace: &TempTree,
    cache_root: &TempTree,
    env: Vec<(String, String)>,
) -> PtySession {
    let main_rs = repro_main_rs(workspace);
    // The PTY helper already serializes these sessions, so the probe only needs
    // a workspace-local cwd and cache root to avoid cross-test interference.
    spawn_lsp_session_with_config(
        ordex_bin(),
        std::slice::from_ref(&main_rs),
        PtySessionConfig {
            current_dir: Some(workspace.path().to_path_buf()),
            cache_root: Some(cache_root.path().to_path_buf()),
            env,
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Return whether the LSP progress footer is absent from the current screen.
///
/// Returns `true` when the bottom overlay does not currently mention rust-analyzer,
/// and `false` when at least one progress line is still visible.
fn overlay_footer_hidden(screen: &ScreenSnapshot) -> bool {
    (24..=27).all(|row| !screen.row_contains(row, "rust-analyzer"))
}

/// Return whether the LSP progress footer is visible in the current screen.
///
/// Returns `true` when the bottom overlay currently mentions rust-analyzer, and
/// `false` when no progress line is visible.
fn overlay_footer_visible(screen: &ScreenSnapshot) -> bool {
    (24..=27).any(|row| screen.row_contains(row, "rust-analyzer"))
}

/// Return whether the warning marker for the inserted `unused_mut` binding is visible.
///
/// Returns `true` when the edited line and diagnostic summary are both visible, and
/// `false` when the warning has not appeared yet.
fn warning_visible(screen: &ScreenSnapshot) -> bool {
    screen.row_contains(3, "let mut value = 10;")
        && screen.row_contains(3, "●")
        && screen.status_line_contains("● ")
}

/// Wait until startup analysis settles without leaving active progress or diagnostics.
fn wait_for_startup_analysis_to_settle(session: &mut PtySession) {
    let _ = session.wait_until(Duration::from_secs(8), |screen| {
        overlay_footer_visible(screen)
    });
    // Rust-analyzer can briefly hide progress between startup phases, so wait
    // for a sustained quiet streak before starting the save-latency repro.
    for _ in 0..10 {
        session
            .wait_until(Duration::from_secs(20), |screen| {
                overlay_footer_hidden(screen) && !screen.status_line_contains("● ")
            })
            .expect("startup analysis should settle");
        thread::sleep(Duration::from_millis(300));
    }
}

/// Wait until the repro workspace renders the initial hello-world file.
fn wait_for_main_rs(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");
}

/// Observe how quickly progress and the warning become visible after one save.
fn observe_warning_latency(session: &mut PtySession) -> (Option<Duration>, Option<Duration>) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs(15);
    let mut first_progress = None;
    let mut first_warning = None;
    // Poll the PTY transcript until either the warning appears or the probe times out.
    while Instant::now() < deadline {
        session.read_available().expect("read PTY output");
        let snapshot = session.snapshot();
        if first_progress.is_none() && overlay_footer_visible(&snapshot) {
            first_progress = Some(start.elapsed());
        }
        if warning_visible(&snapshot) {
            first_warning = Some(start.elapsed());
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    (first_progress, first_warning)
}

/// Quit the PTY session with `:q!`.
fn quit_without_saving(session: &mut PtySession) {
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(3))
        .expect("quit cleanly");
}

/// Return the parsed trace timestamps for lines containing `needle`.
fn trace_timestamps(trace: &str, needle: &str) -> Vec<u128> {
    trace
        .lines()
        .filter(|line| line.contains(needle))
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|value| value.parse::<u128>().ok())
        .collect()
}

/// Read one saved LSP trace file into memory.
fn read_trace(trace_path: &Path) -> String {
    fs::read_to_string(trace_path).expect("read LSP trace")
}

/// Reproduce the reported save latency through one fixture-backed Ordex PTY session.
#[test]
fn test_external_reproducer_warning_latency_probe() {
    let workspace = repro_workspace();
    let cache_root = TempTree::new().expect("create cache root");
    let mut session = spawn_repro_session(&workspace, &cache_root, Vec::new());
    wait_for_main_rs(&mut session);

    wait_for_startup_analysis_to_settle(&mut session);
    session.clear_transcript();

    session
        .send_text("GkO    let mut value = 10;")
        .expect("insert warning reproducer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save warning reproducer");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    let (first_progress, first_warning) = observe_warning_latency(&mut session);
    quit_without_saving(&mut session);

    let warning_latency = first_warning.unwrap_or_else(|| {
        panic!(
            "warning never appeared; first_progress={first_progress:?}\n{}",
            session.snapshot().raw()
        )
    });
    assert!(
        warning_latency <= Duration::from_secs(2),
        "warning appeared too slowly: warning={warning_latency:?}, first_progress={first_progress:?}\n{}",
        session.snapshot().raw()
    );
}

/// Probe the reported save path through live typing that allows background `didChange` syncs.
#[test]
fn test_external_reproducer_warning_latency_probe_with_live_typing() {
    let workspace = repro_workspace();
    let cache_root = TempTree::new().expect("create cache root");
    let trace_dir = TempTree::new().expect("create trace dir");
    let trace_path = trace_dir.path().join("lsp-trace.log");
    let mut session = spawn_repro_session(
        &workspace,
        &cache_root,
        vec![("ORDEX_LSP_TRACE".to_string(), trace_path.display().to_string())],
    );
    wait_for_main_rs(&mut session);

    wait_for_startup_analysis_to_settle(&mut session);
    session.clear_transcript();

    session.send_text("GkO").expect("open line above");
    for character in "    let mut value = 10;".chars() {
        session
            .send_text(&character.to_string())
            .expect("type one character");
        thread::sleep(Duration::from_millis(150));
    }
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save warning reproducer");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    let (first_progress, first_warning) = observe_warning_latency(&mut session);
    quit_without_saving(&mut session);

    let trace = read_trace(&trace_path);
    let did_change_times = trace_timestamps(&trace, "textDocument/didChange");
    let did_save_times = trace_timestamps(&trace, "textDocument/didSave");

    assert!(
        !did_change_times.is_empty(),
        "live typing should still reach rust-analyzer as a didChange before or during save\n{trace}"
    );
    assert!(
        !did_save_times.is_empty(),
        "live typing should still send didSave after writing the file\n{trace}"
    );
    assert!(
        did_change_times[0] <= did_save_times[0],
        "didChange should not lag behind didSave\n{trace}"
    );
    let warning_latency = first_warning.unwrap_or_else(|| {
        panic!(
            "warning never appeared; first_progress={first_progress:?}\n{}",
            session.snapshot().raw()
        )
    });
    assert!(
        warning_latency <= Duration::from_secs(2),
        "warning appeared too slowly after live typing: warning={warning_latency:?}, first_progress={first_progress:?}\n{}",
        session.snapshot().raw()
    );
}

/// Probe the reported save path with an immediate save right after leaving Insert mode.
#[test]
fn test_external_reproducer_warning_latency_probe_after_immediate_escape_save() {
    let workspace = repro_workspace();
    let cache_root = TempTree::new().expect("create cache root");
    let mut session = spawn_repro_session(&workspace, &cache_root, Vec::new());
    wait_for_main_rs(&mut session);

    wait_for_startup_analysis_to_settle(&mut session);
    session.clear_transcript();

    session
        .send_text("GkO    let mut value = 10;")
        .expect("insert warning reproducer");
    session.send_escape().expect("leave insert mode");
    session
        .send_text(" w")
        .expect("save through normal binding");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    let (first_progress, first_warning) = observe_warning_latency(&mut session);
    quit_without_saving(&mut session);

    let warning_latency = first_warning.unwrap_or_else(|| {
        panic!(
            "warning never appeared after immediate save; first_progress={first_progress:?}\n{}",
            session.snapshot().raw()
        )
    });
    assert!(
        warning_latency <= Duration::from_secs(2),
        "warning appeared too slowly after immediate save: warning={warning_latency:?}, first_progress={first_progress:?}\n{}",
        session.snapshot().raw()
    );
}
