use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use test_utils::{PtySession, PtySessionConfig, ScreenSnapshot, TempTree};

/// Restore the external repro file when the current test scope ends.
struct ExternalMainRestore {
    original: String,
}

impl Drop for ExternalMainRestore {
    /// Restore the original external repro contents during drop.
    fn drop(&mut self) {
        fs::write(external_main_rs(), &self.original).expect("restore external main.rs");
    }
}

/// Return one process-wide lock so external repro probes do not overlap.
fn external_repro_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Acquire the external-repro lock even if an earlier probe panicked while holding it.
fn lock_external_repro_tests() -> MutexGuard<'static, ()> {
    match external_repro_test_lock().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Return the compiled ordex binary path for PTY-backed repro tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the external repro workspace root under the current user's home directory.
fn external_workspace_root() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME should be set")).join("tests/ordex-test")
}

/// Return the exact `main.rs` path for the external repro workspace.
fn external_main_rs() -> PathBuf {
    external_workspace_root().join("src/main.rs")
}

/// Return the cache root that matches the user's default XDG cache location.
fn default_cache_root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME should be set")).join(".cache")
        })
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
    // Rust-analyzer can briefly hide progress between startup phases, so
    // require a longer quiet run before starting the external repro edits.
    for _ in 0..10 {
        session
            .wait_until(Duration::from_secs(20), |screen| {
                overlay_footer_hidden(screen) && !screen.status_line_contains("● ")
            })
            .expect("startup analysis should settle");
        thread::sleep(Duration::from_millis(300));
    }
}

/// Restore the external repro file to the requested contents.
fn write_external_main(contents: &str) {
    fs::write(external_main_rs(), contents).expect("write external main.rs");
}

/// Replace the external repro file and restore the original contents on drop.
fn replace_external_main(contents: &str) -> ExternalMainRestore {
    let original = fs::read_to_string(external_main_rs()).expect("read external main.rs");
    write_external_main(contents);
    ExternalMainRestore { original }
}

/// Spawn Ordex against the external repro workspace using the user's cache root.
fn spawn_external_repro_session() -> PtySession {
    spawn_external_repro_session_with_cache_root(default_cache_root(), Vec::new())
}

/// Spawn Ordex against the external repro workspace with extra environment overrides.
fn spawn_external_repro_session_with_env(env: Vec<(String, String)>) -> PtySession {
    spawn_external_repro_session_with_cache_root(default_cache_root(), env)
}

/// Spawn Ordex against the external repro workspace with a chosen cache root.
fn spawn_external_repro_session_with_cache_root(
    cache_root: PathBuf,
    env: Vec<(String, String)>,
) -> PtySession {
    let main_rs = external_main_rs();
    let mut session_env = vec![("ORDEX_DISABLE_DEFAULT_CONFIG".to_string(), "0".to_string())];
    session_env.extend(env);
    PtySession::spawn(
        ordex_bin(),
        &[main_rs.to_str().expect("utf8 main.rs path")],
        PtySessionConfig {
            current_dir: Some(external_workspace_root()),
            cache_root: Some(cache_root),
            env: session_env,
            ..Default::default()
        },
    )
    .expect("spawn ordex")
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

/// Reproduce the reported external-workspace save latency through a real Ordex PTY session.
#[test]
fn test_external_reproducer_warning_latency_probe() {
    let _lock = lock_external_repro_tests();
    let _restore = replace_external_main("fn main() {\n    println!(\"Hello, world!\");\n}\n");

    let mut session = spawn_external_repro_session();
    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

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

    let start = Instant::now();
    let deadline = start + Duration::from_secs(15);
    let mut first_progress = None;
    let mut first_warning = None;
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

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(3))
        .expect("quit cleanly");

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

/// Probe the external repro through live typing that allows background `didChange` syncs.
#[test]
fn test_external_reproducer_warning_latency_probe_with_live_typing() {
    let _lock = lock_external_repro_tests();
    let _restore = replace_external_main("fn main() {\n    println!(\"Hello, world!\");\n}\n");
    let trace_dir = TempTree::new().expect("create trace dir");
    let trace_path = trace_dir.path().join("lsp-trace.log");
    let mut session = spawn_external_repro_session_with_env(vec![(
        "ORDEX_LSP_TRACE".to_string(),
        trace_path.display().to_string(),
    )]);

    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

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

    let start = Instant::now();
    let deadline = start + Duration::from_secs(15);
    let mut first_progress = None;
    let mut first_warning = None;
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

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(3))
        .expect("quit cleanly");

    let trace = fs::read_to_string(trace_path).expect("read LSP trace");
    let did_change_times = trace_timestamps(&trace, "textDocument/didChange");
    let did_save_times = trace_timestamps(&trace, "textDocument/didSave");
    let publish_times = trace_timestamps(&trace, "textDocument/publishDiagnostics");
    eprintln!(
        "live-typing trace: didChange={did_change_times:?} didSave={did_save_times:?} publish={publish_times:?} first_progress={first_progress:?} first_warning={first_warning:?}"
    );

    assert!(
        !did_change_times.is_empty(),
        "live typing should still reach rust-analyzer as a didChange before or during save\n{trace}"
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

/// Probe the external repro with an immediate save right after leaving Insert mode.
#[test]
fn test_external_reproducer_warning_latency_probe_after_immediate_escape_save() {
    let _lock = lock_external_repro_tests();
    let _restore = replace_external_main("fn main() {\n    println!(\"Hello, world!\");\n}\n");
    let cache_root = TempTree::new().expect("create cache root");
    let mut session =
        spawn_external_repro_session_with_cache_root(cache_root.path().to_path_buf(), Vec::new());

    session
        .wait_until(Duration::from_secs(3), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

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

    let start = Instant::now();
    let deadline = start + Duration::from_secs(15);
    let mut first_progress = None;
    let mut first_warning = None;
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

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(3))
        .expect("quit cleanly");

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
