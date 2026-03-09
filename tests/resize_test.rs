use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_resize_height_renders_more_content_rows() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=20 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 path")],
        PtySessionConfig { cols: 80, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(6, "line 06"))
        .expect("wait for initial render");

    session.resize(80, 12).expect("resize terminal");
    session.send_text("j").expect("send key to continue loop");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(10, "line 10"))
        .expect("wait for resized render");

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_resize_width_retruncates_rendered_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"12345678901234567890TAILMARKER_END\n")
        .expect("seed file");

    let config = TempFile::new().expect("create temp config");
    config
        .write_all(
            br#"
[editor]
soft_wrap = false
"#,
        )
        .expect("write config");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("utf8 path"),
        ],
        PtySessionConfig {
            cols: 100,
            rows: 30,
        },
    )
    .expect("spawn ordex");
    session.resize(40, 8).expect("set initial terminal size");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "TAILMARKER_END")
        })
        .expect("wait for wide render");

    session.resize(20, 8).expect("resize terminal");
    session.send_text("l").expect("send key to continue loop");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "1234567890123456") && !s.row_contains(1, "TAILMARKER_END")
        })
        .expect("wait for narrow render");

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_resize_redraws_without_keyboard_input() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=20 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 path")],
        PtySessionConfig { cols: 80, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(6, "line 06"))
        .expect("wait for initial render");

    session.resize(80, 12).expect("resize terminal");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(10, "line 10"))
        .expect("wait for resized render without input");

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_resize_does_not_full_clear_screen() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=20 {
        file.writeln(&format!("line {:02}", i))
            .expect("append line");
    }

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 path")],
        PtySessionConfig { cols: 80, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(6, "line 06"))
        .expect("wait for initial render");

    // Ignore startup output and inspect only resize-triggered redraw bytes.
    session.clear_transcript();

    session.resize(80, 12).expect("resize terminal");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(10, "line 10"))
        .expect("wait for resized render");

    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[2J"),
        "resize redraw should not perform full-screen clear"
    );

    session.send_text(":q").expect("send quit command");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
