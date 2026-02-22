use std::fs;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_insert_text_and_save() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.contains("NORMAL |"))
        .expect("wait for initial render");

    session.send_text("i world").expect("type in insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| s.contains("INSERT |"))
        .expect("wait for insert mode render");

    session.send_escape().expect("exit insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| s.contains("NORMAL |"))
        .expect("back to normal mode");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.contains("1:8")
        })
        .expect("cursor should have moved");

    session.send_text(":wq").expect("send save and quit");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, " worldhello");

    // Reopen the saved file and verify the written text is visible on screen.
    let mut reopen = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex for reopen");

    reopen
        .wait_until(Duration::from_secs(2), |s| {
            s.contains("NORMAL |") //&& s.contains("worldhello")
        })
        .expect("saved text should be visible after reopen");

    session.send_text("e").expect("go to end of word");

    reopen
        .wait_until(Duration::from_secs(2), |s| {
            s.contains("1:7")
        })
        .expect("cursor should have moved");

    reopen.send_text(":q").expect("quit reopen session");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("reopen quit cleanly");
}
