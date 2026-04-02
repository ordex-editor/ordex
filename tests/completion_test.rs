use std::time::Duration;
use test_utils::{PtySession, TempFile};

/// Return the compiled Ordex binary path for PTY tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Open a new empty line below the first line and wait for Insert mode.
fn open_insert_line(session: &mut PtySession) {
    session.send_text("o").expect("open line below");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("INSERT ") && snapshot.status_line_contains("2:1")
        })
        .expect("wait for insert mode on new line");
}

#[test]
/// Confirm completion previews a selected candidate and restores the raw prefix on deselection.
fn test_completion_preview_and_no_selection_restore() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alphabet\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    open_insert_line(&mut session);
    session.send_text("a").expect("type completion prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("INSERT ") && snapshot.contains("Completion")
        })
        .expect("wait for completion popup");

    session
        .send_text("\u{e}")
        .expect("move completion selection down");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "alphabet")
        })
        .expect("wait for previewed completion");

    session
        .send_text("\u{10}")
        .expect("move selection back to none");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "a") && !snapshot.row_contains(2, "alphabet")
        })
        .expect("wait for original prefix restoration");

    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for normal mode");
}

#[test]
/// Confirm lowercase typing still matches buffer words while preserving source casing.
fn test_completion_matches_case_insensitively_and_preserves_casing() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"Message\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    open_insert_line(&mut session);
    session.send_text("me").expect("type lowercase prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.contains("Completion")
        })
        .expect("wait for completion popup");

    session
        .send_text("\u{e}")
        .expect("select matching completion");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "Message")
        })
        .expect("wait for mixed-case preview");
}

#[test]
/// Confirm deleting the active prefix dismisses the completion popup immediately.
fn test_completion_dismisses_after_invalidating_backspace() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alphabet\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    open_insert_line(&mut session);
    session.send_text("a").expect("type prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.contains("Completion")
        })
        .expect("wait for popup");

    session.send_text("\u{7f}").expect("delete typed prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("2:1") && !snapshot.row_contains(2, "a")
        })
        .expect("wait for invalidating backspace result");
}

#[test]
/// Confirm moving the cursor while the popup is open dismisses stale suggestions.
fn test_completion_dismisses_when_cursor_moves() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alphabet\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    open_insert_line(&mut session);
    session.send_text("a").expect("type prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.contains("Completion")
        })
        .expect("wait for popup");

    session.send_text("\u{1b}[D").expect("move cursor left");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("2:1")
                && !(1..=27).any(|row| snapshot.row_contains(row, "Completion"))
        })
        .expect("wait for popup dismissal after cursor move");
}
