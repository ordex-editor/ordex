mod config_test_support;

use std::time::Duration;
use test_utils::TempFile;

#[test]
fn test_missing_include_is_recoverable() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[include]
extra = "does-not-exist.cfg"

[keymap.normal]
z = "move-right"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("z").expect("use keymap");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:2"))
        .expect("startup should continue and apply keymap");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
