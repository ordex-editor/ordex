use std::time::Duration;

use termion::cursor;
use termion::screen;
use termion::style;
use test_utils::{PtySession, PtySessionConfig, TempFile};

/// Return the path to the built ordex binary for PTY tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Verify SIGTERM restores the terminal before the process exits.
#[test]
fn test_sigterm_restores_terminal_state_before_exit() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 1\nline 2\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 path")],
        PtySessionConfig { cols: 80, rows: 12 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ") && snapshot.row_contains(1, "line 1")
        })
        .expect("wait for initial render");

    session.clear_transcript();
    session.send_signal(libc::SIGTERM).expect("send SIGTERM");

    let status = session
        .wait_for_exit(Duration::from_secs(2))
        .expect("wait for exit after SIGTERM");

    // The current bug leaves TUI artifacts behind because termination bypasses the
    // normal cleanup path. Once fixed, the byte stream should include the standard
    // screen, cursor, and style restoration sequences before the signal-driven exit.
    let snapshot = session.snapshot();
    let to_main_screen = format!("{}", screen::ToMainScreen);
    let show_cursor = format!("{}", cursor::Show);
    let reset_style = format!("{}", style::Reset);

    assert!(
        status.success(),
        "SIGTERM cleanup path should exit cleanly after restoring the terminal: {status}"
    );
    assert!(
        snapshot.contains(&to_main_screen),
        "terminal cleanup should leave the alternate screen on SIGTERM"
    );
    assert!(
        snapshot.contains(&show_cursor),
        "terminal cleanup should show the cursor on SIGTERM"
    );
    assert!(
        snapshot.contains(&reset_style),
        "terminal cleanup should reset terminal style on SIGTERM"
    );
}
