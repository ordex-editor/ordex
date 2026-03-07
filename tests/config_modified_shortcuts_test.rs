mod config_test_support;

use std::fs;
use std::time::Duration;
use test_utils::TempFile;

#[test]
fn config_distinguishes_ctrl_home_from_home() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcde\n").expect("seed file");

    let config = config_test_support::temp_config_path("modified_shortcuts");
    config_test_support::write_config(
        &config,
        r#"
[keymap.normal]
home = "move-line-start"
ctrl-home = "move-line-end"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session.send_text("ll").expect("move cursor right twice");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:3"))
        .expect("cursor should be in the middle of the line");

    session
        .send_text("\u{1b}[1;5H")
        .expect("send ctrl-home escape sequence");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:5"))
        .expect("ctrl-home binding should move to line end");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let _ = fs::remove_file(config);
}
