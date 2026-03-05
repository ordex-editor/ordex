#[path = "config_test_support.rs"]
mod config_test_support;

use std::fs;
use std::time::Duration;
use test_utils::TempFile;

#[test]
fn test_valid_config_applies_keymap_with_comments() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello\n").expect("seed file");

    let config = config_test_support::temp_config_path("valid_keymap");
    config_test_support::write_config(
        &config,
        r#"
# top-level comment
[editor]
scroll_margin = 2 # comment at end of line

[keymap.normal]
z = "MoveRight"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("z").expect("use configured keymap");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("config keymap should be active");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let _ = fs::remove_file(config);
}

#[test]
fn test_unknown_keys_are_ignored_and_startup_succeeds() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello\n").expect("seed file");

    let config = config_test_support::temp_config_path("unknown_keys");
    config_test_support::write_config(
        &config,
        r#"
[editor]
scroll_margin = 1
future_setting = 42

[unknown_section]
foo = "bar"

[keymap.normal]
z = "MoveRight"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("z").expect("use configured keymap");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("startup should continue with unknown keys");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let _ = fs::remove_file(config);
}

#[test]
fn test_multi_key_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"a\nb\n").expect("seed file");

    let config = config_test_support::temp_config_path("multi_key_binding");
    config_test_support::write_config(
        &config,
        r#"
[keymap.normal]
zu = "MoveDown"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("zu").expect("use multi-key mapping");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:1"))
        .expect("multi-key mapping should move down");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let _ = fs::remove_file(config);
}

#[test]
fn test_unicode_key_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let config = config_test_support::temp_config_path("unicode_key_binding");
    config_test_support::write_config(
        &config,
        r#"
[keymap.normal]
é = "MoveRight"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("é").expect("use unicode mapping");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("unicode key mapping should move right");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");

    let _ = fs::remove_file(config);
}
