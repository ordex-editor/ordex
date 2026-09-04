//! End-to-end coverage for the opening frame Ordex paints when it takes over the terminal.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree};

/// Return the path of the Ordex binary built for this test run.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Spawn Ordex on a two-line file with `stray` bytes already waiting in the terminal.
///
/// The bytes are written to the PTY master right after the spawn so they reach
/// the editor while it is still setting the terminal up, which is how a terminal
/// reply or an interrupted paste lands in Ordex's input stream at launch.
fn spawn_with_stray_startup_input(file: &TempFile, stray: &[u8]) -> PtySession {
    file.write_all(b"first line\nsecond line\n")
        .expect("seed startup file");
    let path = file.path().to_string_lossy().to_string();
    let mut session = PtySession::spawn(ordex_bin(), &[path.as_str()], PtySessionConfig::default())
        .expect("spawn ordex");
    session
        .send_raw_bytes(stray)
        .expect("send stray startup input");
    session
}

/// Assert that the opening frame reached the alternate screen.
///
/// A build that lets pending startup input defer the first paint leaves the
/// alternate screen empty here for as long as the decode of that input runs.
#[track_caller]
fn assert_opening_frame_painted(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(5), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
                && snapshot.row_trimmed_ends_with(1, "first line")
        })
        .expect("opening frame must be painted");
}

/// Assert that a Normal-mode keystroke still switches the editor into Insert mode.
#[track_caller]
fn assert_responds_to_keystrokes(session: &mut PtySession) {
    session.send_text("i").expect("send insert key");
    session
        .wait_until(Duration::from_secs(5), |snapshot| {
            snapshot.status_line_contains("INSERT ")
        })
        .expect("editor must react to keystrokes");
}

#[test]
fn test_startup_frame_survives_unterminated_bracketed_paste() {
    let file = TempFile::with_suffix(".txt").expect("create temp file");
    // A paste-start marker whose `ESC [ 201 ~` terminator never arrives. Payload
    // collection has to give up on its own; a read that waits for the terminator
    // owns the input stream for the rest of the session.
    let mut session = spawn_with_stray_startup_input(&file, b"\x1b[200~");
    assert_opening_frame_painted(&mut session);

    // Bytes typed while the terminal claims a paste is running belong to that
    // paste, so responsiveness is asserted once the payload read has timed out.
    std::thread::sleep(Duration::from_millis(1_500));
    assert_responds_to_keystrokes(&mut session);
}

#[test]
fn test_startup_frame_survives_truncated_utf8_sequence() {
    let file = TempFile::with_suffix(".txt").expect("create temp file");
    // A two-byte UTF-8 lead byte with no continuation byte behind it.
    let mut session = spawn_with_stray_startup_input(&file, b"\xc3");
    assert_opening_frame_painted(&mut session);
    assert_responds_to_keystrokes(&mut session);
}

#[test]
fn test_startup_frame_precedes_slow_workspace_probe() {
    let tools = TempTree::with_prefix("ordex_startup_probe_bin").expect("create tool tree");
    let cargo_path = tools.path().join("cargo");
    // The stub outlives the assertion window below so the probe is still running
    // when the opening frame is checked.
    std::fs::write(&cargo_path, "#!/bin/sh\nsleep 5\nexit 1\n").expect("write stub cargo");
    std::fs::set_permissions(&cargo_path, std::fs::Permissions::from_mode(0o755))
        .expect("make stub cargo executable");

    let project = TempTree::with_prefix("ordex_startup_probe_project").expect("create project");
    project
        .write_file(
            "Cargo.toml",
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest");
    project
        .write_file("src/main.rs", "fn main() {}\n")
        .expect("write source");

    let source_path = project.path().join("src/main.rs");
    let search_path = format!(
        "{}:{}",
        tools.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[source_path.to_string_lossy().as_ref()],
        PtySessionConfig {
            current_dir: Some(project.path().to_path_buf()),
            env: vec![("PATH".to_string(), search_path)],
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    // Workspace detection shells out to Cargo on the main thread, so the opening
    // frame has to be on screen before that probe runs.
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
                && snapshot.row_trimmed_ends_with(1, "fn main() {}")
        })
        .expect("opening frame must not wait for the workspace probe");
}
