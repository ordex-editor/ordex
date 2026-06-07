mod config_test_support;

use std::time::Duration;
use test_utils::{ScreenSnapshot, TempFile};

/// Return whether any visible row contains `needle`.
fn screen_has_text(snapshot: &ScreenSnapshot, rows: usize, needle: &str) -> bool {
    (1..=rows).any(|row| snapshot.row_contains(row, needle))
}

#[test]
fn test_builtin_sequence_popup_shows_g_continuations() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = test_utils::PtySession::spawn(
        config_test_support::ordex_bin(),
        &[file.path().to_str().expect("file path utf8")],
        Default::default(),
    )
    .expect("spawn ordex");
    config_test_support::wait_normal_mode(&mut session);

    session.send_text("g").expect("start g sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            screen_has_text(snapshot, 30, "Move to first line")
                && screen_has_text(snapshot, 30, "Move line end")
                && screen_has_text(snapshot, 30, "Move line start")
        })
        .expect("popup should show built-in g continuations");

    session.send_escape().expect("cancel sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            !screen_has_text(snapshot, 30, "Move to first line")
                && !screen_has_text(snapshot, 30, "Move line end")
                && !screen_has_text(snapshot, 30, "Move line start")
        })
        .expect("popup should disappear after escape");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_configured_sequence_popup_shows_custom_continuations() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
zu = ["move-down", "move-right"]
zq = "save-current-file"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session.send_text("z").expect("start z sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            screen_has_text(snapshot, 30, "Move down -> Move right")
                && screen_has_text(snapshot, 30, "Save current file")
        })
        .expect("popup should show configured continuations");

    session.send_text("u").expect("complete zu sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("2/2:2")
                && !screen_has_text(snapshot, 30, "Move down -> Move right")
        })
        .expect("configured sequence should execute and hide popup");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_configured_space_sequence_popup_shows_custom_continuations() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
<space>s = "move-right"
<space>t = "move-down"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session.send_text(" ").expect("start space sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            screen_has_text(snapshot, 30, "Move right")
                && screen_has_text(snapshot, 30, "Move down")
        })
        .expect("popup should show configured space continuations");

    session.send_text("s").expect("complete space sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("1/2:2") && !screen_has_text(snapshot, 30, "Move right")
        })
        .expect("space sequence should execute and hide popup");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_sequence_popup_can_be_disabled_via_config() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[editor]
sequence_discovery_popup = false
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session.send_text("g").expect("start g sequence");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.message_line_contains("g")
                && !screen_has_text(snapshot, 30, "Move to first line")
                && !screen_has_text(snapshot, 30, "Move line end")
        })
        .expect("popup should remain hidden when disabled");

    session.send_escape().expect("cancel sequence");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
