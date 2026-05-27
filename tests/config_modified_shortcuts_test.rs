mod config_test_support;

use std::time::Duration;
use test_utils::TempFile;

#[test]
fn config_distinguishes_ctrl_home_from_home() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcde\n").expect("seed file");

    let config = config_test_support::write_config(
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
}

#[test]
fn config_repeated_custom_change_operator_key_changes_current_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
l = "begin-change-operator"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session
        .send_text("llZ")
        .expect("change current line with ll");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.row(1).is_some_and(|line| line.trim_end().ends_with("Z"))
                && s.row(2)
                    .is_some_and(|line| line.trim_end().ends_with("beta"))
        })
        .expect("ll should change the current line and enter insert mode");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute save and quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");
}

#[test]
fn config_repeated_custom_delete_operator_key_deletes_current_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
h = "begin-delete-operator"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session
        .send_text("hh")
        .expect("delete current line with hh");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1)
                    .is_some_and(|line| line.trim_end().ends_with("beta"))
        })
        .expect("hh should delete the current line");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute save and quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");
}

#[test]
fn config_repeated_custom_yank_operator_key_yanks_current_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
u = "begin-yank-operator"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session
        .send_text("uup")
        .expect("yank current line with uu and paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1)
                    .is_some_and(|line| line.trim_end().ends_with("alpha"))
                && s.row(2)
                    .is_some_and(|line| line.trim_end().ends_with("alpha"))
                && s.row(3)
                    .is_some_and(|line| line.trim_end().ends_with("beta"))
        })
        .expect("uu should yank the current line linewise");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute save and quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");
}

#[test]
fn config_replay_binding_uses_remapped_change_operator() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
l = "begin-change-operator"
z = "@liw"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session
        .send_text("z")
        .expect("replay remapped change operator");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_contains(1, " beta")
        })
        .expect("replay binding should use the remapped operator binding");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
