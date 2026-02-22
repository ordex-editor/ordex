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
            s.status_line().is_some_and(|line| line.contains("NORMAL"))
        })
        .expect("initial normal mode");

    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("INSERT"))
        })
        .expect("insert mode visible");

    session.send_escape().expect("back to normal");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("NORMAL"))
        })
        .expect("normal mode restored");

    session.send_text(":").expect("enter command mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("COMMAND"))
        })
        .expect("command mode visible");

    session.send_escape().expect("cancel command");
    session.send_text("/").expect("enter search mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("SEARCH"))
        })
        .expect("search mode visible");

    session.send_escape().expect("cancel search");
    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
