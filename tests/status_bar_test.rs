use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_status_bar_mode_transitions() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && s.row_contains(1, "status")
        })
        .expect("initial normal mode");

    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT |")
        })
        .expect("insert mode visible");

    session.send_escape().expect("back to normal");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
        })
        .expect("normal mode restored");

    session.send_text(":").expect("enter command mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND |") && s.message_line_contains(":")
        })
        .expect("command mode visible");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
        })
        .expect("normal mode restored after command cancel");
    session.send_text("/").expect("enter search mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH |") && s.message_line_contains("/")
        })
        .expect("search mode visible");

    session.send_escape().expect("cancel search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
        })
        .expect("normal mode restored after search cancel");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_pending_g_indicator_on_message_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && !s.message_line_contains("g")
        })
        .expect("initial normal mode");

    session.send_text("g").expect("start sequence prefix");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("g"))
        .expect("pending marker visible");

    session.send_text("i").expect("mismatch consumes both");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |") && !s.message_line_contains("g")
        })
        .expect("marker cleared after mismatch");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
