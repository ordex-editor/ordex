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
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("NORMAL"))
        })
        .expect("wait for initial render");

    session.send_text("i world").expect("type in insert mode");
    let insert_snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("INSERT"))
        })
        .expect("wait for insert mode render");
    assert!(insert_snapshot.contains(" worldhello"));

    session.send_escape().expect("exit insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line().is_some_and(|line| line.contains("NORMAL"))
        })
        .expect("back to normal mode");

    session.send_text(":wq").expect("send save and quit");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, " worldhello");
}
