use std::path::PathBuf;
use std::time::{Duration, Instant};
use test_utils::{PtySession, PtySessionConfig, TempFile};

/// Return the release-profile ordex binary path for latency-sensitive PTY tests.
///
/// Latency budgets in this module are calibrated for an optimized binary.
/// `CARGO_BIN_EXE_ordex` points to the debug binary when tests are compiled in
/// debug mode, so this function replaces the `debug` profile component with
/// `release`.  Run `cargo build --release` before executing this test to ensure
/// the release binary is present.
fn ordex_bin() -> PathBuf {
    let debug_path = PathBuf::from(env!("CARGO_BIN_EXE_ordex"));
    // Walk the path components and swap the `debug` profile directory for
    // `release`.  CARGO_BIN_EXE_* always places the binary under a profile
    // directory that matches the active Cargo profile name, so replacing the
    // first component named `debug` gives the release binary location.
    let mut components: Vec<_> = debug_path.components().collect();
    for component in &mut components {
        if component.as_os_str() == "debug" {
            *component = std::path::Component::Normal(std::ffi::OsStr::new("release"));
            break;
        }
    }
    let path: PathBuf = components.iter().collect();
    assert!(
        path.is_file(),
        "release binary not found at {path:?}; run `cargo build --release` first"
    );
    path
}

/// Return one two-line fixture that mirrors the flaky CI failure payload shape.
fn flaky_ci_completion_fixture() -> String {
    const TARGET_SECOND_LINE_LEN: usize = 332_549;
    let first_line = "2026-06-12T14:46:22.7126800Z thread 'test_lsp_completion_popup_stays_below_current_line_after_backspacing_prefix' (9165) panicked at tests/lsp_completion_test.rs:618:6:";
    let second_line_prefix = "2026-06-12T14:46:22.8115647Z wait for final popup below current line: Custom { kind: TimedOut, error: \"condition not stable before timeout; snapshot:\\n\\u{1b}[?1049h\\u{1b}[?2004h\\u{1b}[22;2t\\u{1b}[?2026h\\u{1b}[2J\\u{1b}[?2026l\\u{1b}[?2026h\\u{1b}]2;main.rs (~/work/ordex/ordex) - ordex\\u{7}\\u{1b}[?25l\\u{1b}[1;1H\\u{1b}[48;5;235m\\u{1b}[38;5;253m\\u{1b}[K\\u{1b}[m\\u{1b}[1;1H\\u{1b}[48;5;74m\\u{1b}[38;5;234m\\u{1b}[1m t/f/l/w/s/main.rs \\u{1b}[m\\u{1b}[2;1H\\u{1b}[48;5;234";
    let second_line_padding =
        "x".repeat(TARGET_SECOND_LINE_LEN.saturating_sub(second_line_prefix.chars().count()));
    let second_line = format!("{second_line_prefix}{second_line_padding}");
    format!("{first_line}\n{second_line}")
}

/// Verify long wrapped lines keep `j$` and command-mode round-trip latency below budget.
#[test]
fn test_large_second_line_navigation_and_command_mode_stay_responsive() {
    const MOTION_BUDGET: Duration = Duration::from_millis(500);
    const COMMAND_ROUND_TRIP_BUDGET: Duration = Duration::from_millis(500);

    let file = TempFile::new().expect("create temp file");
    file.write_all(flaky_ci_completion_fixture().as_bytes())
        .expect("seed large two-line fixture");

    let mut session = PtySession::spawn(
        ordex_bin()
            .to_str()
            .expect("release binary path is valid UTF-8"),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 240,
            rows: 30,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    // Wait until the initial frame is fully rendered before measuring interaction latency.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.status_line_contains("1/2:1")
        })
        .expect("wait for initial normal mode frame");

    // Measure the exact `j$` motion sequence reported as slow on this payload shape.
    // Sending the keys in separate waits keeps the timing signal tied to editor work.
    let motion_start = Instant::now();
    session.send_text("j").expect("move to second line");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.status_line_contains("NORMAL ") && screen.status_line_contains("2/2:1")
        })
        .expect("wait for cursor at start of second line");
    session.send_text("$").expect("move to end of second line");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.status_line_contains("NORMAL ") && screen.status_line_contains("2/2:332549")
        })
        .expect("wait for cursor at end of second line");
    let motion_elapsed = motion_start.elapsed();
    assert!(
        motion_elapsed <= MOTION_BUDGET,
        "j$ latency {motion_elapsed:?} exceeded budget {MOTION_BUDGET:?}"
    );

    // Measure entering command mode with `:` and leaving with `Esc` at this cursor location.
    let command_round_trip_start = Instant::now();
    session.send_text(":").expect("enter command mode");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.status_line_contains("COMMAND ")
        })
        .expect("wait for command mode");
    session.send_escape().expect("leave command mode");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.status_line_contains("NORMAL ") && screen.status_line_contains("2/2:332549")
        })
        .expect("wait for normal mode after command cancel");
    let command_round_trip_elapsed = command_round_trip_start.elapsed();
    assert!(
        command_round_trip_elapsed <= COMMAND_ROUND_TRIP_BUDGET,
        ": + Esc latency {command_round_trip_elapsed:?} exceeded budget {COMMAND_ROUND_TRIP_BUDGET:?}"
    );

    session.send_text(":q!").expect("quit without save");
    session.send_enter().expect("confirm quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
