use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use test_utils::{PtySession, PtySessionConfig, TempFile, command_path};

const CLIPBOARD_POLL_ATTEMPTS: usize = 20;
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Return the test-built Ordex binary path.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Build one session environment for real Wayland clipboard tests.
fn wayland_session_env() -> Vec<(String, String)> {
    command_path("wl-copy").expect("wl-copy must be installed for Wayland clipboard tests");
    command_path("wl-paste").expect("wl-paste must be installed for Wayland clipboard tests");
    let runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR must be set for Wayland tests");
    let display =
        std::env::var("WAYLAND_DISPLAY").expect("WAYLAND_DISPLAY must be set for Wayland tests");
    vec![
        ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        ("XDG_RUNTIME_DIR".to_string(), runtime_dir),
        ("WAYLAND_DISPLAY".to_string(), display),
        ("DISPLAY".to_string(), String::new()),
    ]
}

/// Build one session environment for real X11 clipboard tests.
fn x11_session_env() -> Vec<(String, String)> {
    command_path("xclip").expect("xclip must be installed for X11 clipboard tests");
    let display = std::env::var("DISPLAY").expect("DISPLAY must be set for X11 tests");
    vec![
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
        ("DISPLAY".to_string(), display),
        ("WAYLAND_DISPLAY".to_string(), String::new()),
    ]
}

/// Spawn one Ordex PTY session with the supplied clipboard environment.
fn spawn_clipboard_session(file: &TempFile, env: Vec<(String, String)>) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 file path")],
        PtySessionConfig {
            env,
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Wait until Ordex finishes its initial Normal-mode render.
fn wait_normal_mode(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait normal mode");
}

/// Keep one clipboard owner alive until the surrounding test drops it.
struct ClipboardOwner {
    child: Child,
}

impl Drop for ClipboardOwner {
    /// Stop the clipboard helper process when the test no longer needs it.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Return the shared guard that serializes real clipboard integration tests.
///
/// This still returns a guard after a poisoned lock so later tests can keep
/// exclusive access to the process-global clipboard resources.
fn clipboard_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Write one payload to the real Wayland clipboard and keep its owner alive.
fn seed_wayland_clipboard(env: &[(String, String)], text: &str) -> ClipboardOwner {
    let mut child = Command::new("wl-copy");
    child
        .args(["--foreground"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .envs(env.iter().cloned());
    let mut child = child.spawn().expect("spawn wl-copy");

    // Feed the clipboard payload before returning so Ordex can paste it later.
    child
        .stdin
        .take()
        .expect("wl-copy stdin")
        .write_all(text.as_bytes())
        .expect("write clipboard stdin");
    ClipboardOwner { child }
}

/// Wait until the real Wayland clipboard serves `expected`.
///
/// This panics when the clipboard never becomes readable with the expected
/// text before the short polling window expires.
fn wait_for_wayland_clipboard_text(env: &[(String, String)], expected: &str) {
    for _ in 0..CLIPBOARD_POLL_ATTEMPTS {
        if try_read_wayland_clipboard(env).as_deref() == Some(expected) {
            return;
        }
        std::thread::sleep(CLIPBOARD_POLL_INTERVAL);
    }
    panic!("clipboard text did not become available: {expected}");
}

/// Read one Wayland clipboard payload when a compositor owner is available.
///
/// Returns `Some(text)` when `wl-paste` succeeds and the clipboard contents are
/// valid UTF-8. Returns `None` when the clipboard is empty, the command fails,
/// or the output is not valid UTF-8.
fn try_read_wayland_clipboard(env: &[(String, String)]) -> Option<String> {
    Command::new("wl-paste")
        .args(["--no-newline"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(env.iter().cloned())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
}

/// Read one X11 primary selection payload when an owner is available.
///
/// Returns `Some(text)` when `xclip -o` succeeds and the selection contents are
/// valid UTF-8. Returns `None` when no selection owner exists, the command
/// fails, or the output is not valid UTF-8.
fn try_read_x11_primary_selection(env: &[(String, String)]) -> Option<String> {
    Command::new("xclip")
        .args(["-o", "-selection", "primary"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(env.iter().cloned())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
}

/// Verify `<Space>p` pastes from the Wayland clipboard register.
#[test]
#[ignore = "requires a local Wayland session on this machine"]
fn test_space_p_pastes_wayland_clipboard_after_cursor() {
    let _guard = clipboard_test_lock();
    let env = wayland_session_env();
    let _clipboard = seed_wayland_clipboard(&env, "XYZ");
    wait_for_wayland_clipboard_text(&env, "XYZ");

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("l p")
        .expect("paste clipboard after cursor");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abXYZ")
        })
        .expect("clipboard paste rendered");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("exit cleanly");

    assert_eq!(
        std::fs::read_to_string(file.path()).expect("read saved file"),
        "abXYZ\n"
    );
}

/// Verify `\"+yy` writes the yanked line into the real Wayland clipboard.
#[test]
#[ignore = "requires a local Wayland session on this machine"]
fn test_quote_plus_yy_writes_wayland_clipboard() {
    let _guard = clipboard_test_lock();
    let env = wayland_session_env();

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("\"+yy")
        .expect("yank into clipboard register");
    session
        .wait_until(Duration::from_secs(2), |_| {
            try_read_wayland_clipboard(&env).as_deref() == Some("alpha\n")
        })
        .expect("clipboard write completed");
}

/// Verify `<Space>y` starts a `\"+` yank operator in Normal mode.
#[test]
#[ignore = "requires a local Wayland session on this machine"]
fn test_space_y_then_motion_writes_wayland_clipboard() {
    let _guard = clipboard_test_lock();
    let env = wayland_session_env();

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text(" ye")
        .expect("yank to clipboard with <Space>y");
    session
        .wait_until(Duration::from_secs(2), |_| {
            try_read_wayland_clipboard(&env).as_deref() == Some("alpha")
        })
        .expect("clipboard write completed");
}

/// Verify `<Space>y` in Visual mode writes the active selection to `\"+`.
#[test]
#[ignore = "requires a local Wayland session on this machine"]
fn test_space_y_in_visual_mode_writes_wayland_clipboard() {
    let _guard = clipboard_test_lock();
    let env = wayland_session_env();

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("V y")
        .expect("yank linewise visual selection to clipboard with <Space>y");
    session
        .wait_until(Duration::from_secs(2), |_| {
            try_read_wayland_clipboard(&env).as_deref() == Some("alpha beta\n")
        })
        .expect("clipboard write completed");
}

/// Verify the X11 backend writes the `\"*` register through the real `xclip`.
#[test]
#[ignore = "requires a local X11 session on this machine"]
fn test_quote_star_yy_writes_x11_primary_selection() {
    let _guard = clipboard_test_lock();
    let env = x11_session_env();

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("\"*yy")
        .expect("yank into primary register");
    session
        .wait_until(Duration::from_secs(2), |_| {
            try_read_x11_primary_selection(&env).as_deref() == Some("alpha\n")
        })
        .expect("primary clipboard write completed");
}
