//! Liveness coverage for the editor Ordex presents when it takes over the terminal.
//!
//! Every case asserts the same two properties under an adversarial startup
//! condition: the opening frame reaches the alternate screen, and the editor
//! still answers a keystroke afterwards. A build that decodes startup input in an
//! unbounded read, or that defers its first paint behind such a read, leaves a
//! blank terminal that ignores the keyboard.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree};

/// Longest the editor may take to finish decoding one adversarial input burst.
///
/// Bounded reads inside the input decoder wait at most one second for the rest
/// of a sequence, so bytes typed before that expires belong to the sequence
/// being decoded rather than to the editor.
const DECODE_SETTLE_DELAY: Duration = Duration::from_millis(1_100);

/// Terminal byte bursts that can already be waiting when Ordex takes over stdin.
///
/// Terminal replies to shell probes, an interrupted paste, and type-ahead all
/// land here, and each shape leaves the decoder needing bytes that never arrive.
const ADVERSARIAL_STARTUP_INPUT: &[(&str, &[u8])] = &[
    ("lone escape", b"\x1b"),
    ("truncated CSI introducer", b"\x1b["),
    ("truncated SS3 introducer", b"\x1bO"),
    ("device attributes reply", b"\x1b[?62;c"),
    ("device control string prefix", b"\x1bP"),
    ("paste start without terminator", b"\x1b[200~"),
    ("paste payload without terminator", b"\x1b[200~partial"),
    ("truncated two-byte character", b"\xc3"),
    ("truncated three-byte character", b"\xe2\x82"),
    ("truncated four-byte character", b"\xf0\x9f\x92"),
    ("null byte", b"\x00"),
];

/// Return the path of the Ordex binary built for this test run.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Assert that the opening frame reached the alternate screen.
///
/// `first_line` is the buffer text expected on the first content row, which
/// distinguishes a painted frame from a status line drawn over an empty screen.
#[track_caller]
fn assert_opening_frame_painted(session: &mut PtySession, first_line: &str, context: &str) {
    session
        .wait_until(Duration::from_secs(5), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
                && snapshot.row_trimmed_ends_with(1, first_line)
        })
        .unwrap_or_else(|error| panic!("{context}: opening frame must be painted: {error}"));
}

/// Assert that a Normal-mode keystroke still switches the editor into Insert mode.
#[track_caller]
fn assert_responds_to_keystrokes(session: &mut PtySession, context: &str) {
    session
        .wait_until(Duration::from_secs(5), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .unwrap_or_else(|error| panic!("{context}: editor must settle in Normal mode: {error}"));
    session.send_text("i").expect("send insert key");
    session
        .wait_until(Duration::from_secs(5), |snapshot| {
            snapshot.status_line_contains("INSERT ")
        })
        .unwrap_or_else(|error| panic!("{context}: editor must react to keystrokes: {error}"));
}

#[test]
fn test_editor_comes_up_alive_after_adversarial_startup_input() {
    for (description, stray) in ADVERSARIAL_STARTUP_INPUT {
        let file = TempFile::with_suffix(".txt").expect("create temp file");
        file.write_all(b"first line\nsecond line\n")
            .expect("seed startup file");
        let path = file.path().to_string_lossy().to_string();
        let mut session =
            PtySession::spawn(ordex_bin(), &[path.as_str()], PtySessionConfig::default())
                .expect("spawn ordex");
        // The bytes go out right after the spawn so they reach the editor while
        // it is still claiming the terminal, which is when a stray terminal reply
        // or an interrupted paste actually arrives.
        session.send_raw_bytes(stray).expect("send startup input");

        assert_opening_frame_painted(&mut session, "first line", description);
        // Bytes that arrive while a sequence is still being decoded belong to
        // that sequence, so responsiveness is asserted once decoding has given up.
        std::thread::sleep(DECODE_SETTLE_DELAY);
        assert_responds_to_keystrokes(&mut session, description);
    }
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
    assert_opening_frame_painted(&mut session, "fn main() {}", "slow workspace probe");
}
