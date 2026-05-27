mod config_test_support;

use std::time::Duration;
use test_utils::TempFile;

#[test]
fn test_valid_config_applies_keymap_with_comments() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
# top-level comment
[editor]
scroll_margin = 2 # comment at end of line

[keymap.normal]
z = "move-right"
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
}

#[test]
fn test_unknown_keys_are_ignored_and_startup_succeeds() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[editor]
scroll_margin = 1
future_setting = 42

[unknown_section]
foo = "bar"

[keymap.normal]
z = "move-right"
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
}

#[test]
fn test_multi_key_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"a\nb\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
zu = "move-down"
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
}

#[test]
fn test_multi_action_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\ncd\nef\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
z = ["move-down", "move-right"]
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("z").expect("use multi-action mapping");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("multi-action mapping should move down and right");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_multi_action_sequence_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\ncd\nef\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
zu = ["move-down", "move-right"]
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session
        .send_text("zu")
        .expect("use multi-action sequence mapping");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("multi-action sequence should move down and right");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_replay_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
z = "@diw"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("z").expect("use replay binding");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1")
                && s.row_contains(1, " beta")
                && !s.row_contains(1, "alpha")
        })
        .expect("replay binding should run the keyed operator sequence");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_replay_binding_supports_enter_token() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
z = "@:q!<Enter>"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session
        .send_text("z")
        .expect("use replay binding with enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("enter token should execute the replayed command");
}

#[test]
fn test_unicode_key_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
é = "move-right"
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
}

#[test]
fn test_operator_keymap_binding_is_applied() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.operator]
é = "word-forward"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session
        .send_text("dé")
        .expect("use configured operator binding");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("operator binding should delete through the next word boundary");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "beta") && !s.row_contains(1, "alpha")
        })
        .expect("buffer should delete the first word and keep only the remainder");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
