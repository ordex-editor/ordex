use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use test_utils::{PtySession, PtySessionConfig, TempFile, command_path};

/// Return the test-built Ordex binary path.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Build one session environment for real Wayland clipboard tests.
fn wayland_session_env() -> Option<Vec<(String, String)>> {
    command_path("wl-copy")?;
    command_path("wl-paste")?;
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let display = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(vec![
        ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        ("XDG_RUNTIME_DIR".to_string(), runtime_dir),
        ("WAYLAND_DISPLAY".to_string(), display),
        ("DISPLAY".to_string(), String::new()),
    ])
}

/// Build one session environment for real X11 clipboard tests.
fn x11_session_env() -> Option<Vec<(String, String)>> {
    command_path("xclip")?;
    let display = std::env::var("DISPLAY").ok()?;
    Some(vec![
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
        ("DISPLAY".to_string(), display),
        ("WAYLAND_DISPLAY".to_string(), String::new()),
    ])
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

/// Run one real clipboard command with `env` and optional stdin payload.
fn run_command_with_optional_input(
    program: &str,
    args: &[&str],
    env: &[(String, String)],
    input: Option<&str>,
) -> String {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn clipboard command");

    // Feed the requested clipboard payload before waiting so the helper can
    // finish owning the clipboard selection in the same subprocess.
    if let Some(input) = input
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin
            .write_all(input.as_bytes())
            .expect("write clipboard stdin");
    }
    let output = child.wait_with_output().expect("wait clipboard command");
    assert!(
        output.status.success(),
        "clipboard command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("clipboard output utf8")
}

/// Write one payload to the real Wayland clipboard.
fn seed_wayland_clipboard(env: &[(String, String)], text: &str) {
    let _ = run_command_with_optional_input("wl-copy", &[], env, Some(text));
}

/// Read one payload from the real Wayland clipboard.
fn read_wayland_clipboard(env: &[(String, String)]) -> String {
    run_command_with_optional_input("wl-paste", &["--no-newline"], env, None)
}

/// Read one payload from the real X11 primary selection.
fn read_x11_primary_selection(env: &[(String, String)]) -> String {
    run_command_with_optional_input("xclip", &["-o", "-selection", "primary"], env, None)
}

/// Verify `<Space>p` pastes from the Wayland clipboard register.
#[test]
fn test_space_p_pastes_wayland_clipboard_after_cursor() {
    let Some(env) = wayland_session_env() else {
        return;
    };
    seed_wayland_clipboard(&env, "XYZ");

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("l p")
        .expect("paste clipboard after cursor");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abXYZ")
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
fn test_quote_plus_yy_writes_wayland_clipboard() {
    let Some(env) = wayland_session_env() else {
        return;
    };

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("\"+yy")
        .expect("yank into clipboard register");
    session
        .wait_until(Duration::from_secs(2), |_| {
            read_wayland_clipboard(&env) == "alpha\n"
        })
        .expect("clipboard write completed");
}

/// Verify the X11 backend writes the `\"*` register through the real `xclip`.
#[test]
fn test_quote_star_yy_writes_x11_primary_selection() {
    let Some(env) = x11_session_env() else {
        return;
    };

    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");
    let mut session = spawn_clipboard_session(&file, env.clone());
    wait_normal_mode(&mut session);

    session
        .send_text("\"*yy")
        .expect("yank into primary register");
    session
        .wait_until(Duration::from_secs(2), |_| {
            read_x11_primary_selection(&env) == "alpha\n"
        })
        .expect("primary clipboard write completed");
}
