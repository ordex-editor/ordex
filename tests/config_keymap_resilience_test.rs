mod config_test_support;

use std::fs;
use std::time::Duration;
use test_utils::TempFile;

#[test]
fn test_keymap_survives_unrelated_invalid_section() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let config = config_test_support::temp_config_path("keymap_resilience");
    config_test_support::write_config(
        &config,
        r#"
[editor]
scroll_margin ??? 9

[keymap.normal]
z = "MoveRight"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session.send_text("z").expect("use remapped key");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("z should move cursor right");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let _ = fs::remove_file(config);
}
