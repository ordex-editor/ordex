mod config_test_support;

use std::fs;
use std::path::Path;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

#[test]
fn test_reload_config_command_applies_updated_bindings() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abc\ndef\n").expect("seed file");

    let config = config_test_support::write_config(
        r#"
[keymap.normal]
z = "move-right"
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);

    session.send_text("z").expect("use initial binding");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:2"))
        .expect("initial binding should move right");

    fs::write(
        config.path(),
        r#"
[keymap.normal]
z = "move-down"
"#,
    )
    .expect("rewrite config");

    session
        .send_text(":reload-config")
        .expect("enter reload command");
    session.send_enter().expect("execute reload command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("Config reloaded")
        })
        .expect("reload command should succeed");

    session.send_text("z").expect("use reloaded binding");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2:2"))
        .expect("reloaded binding should move down");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_reload_config_command_reports_missing_active_config() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\n").expect("seed file");

    let mut session = PtySession::spawn(
        config_test_support::ordex_bin(),
        &[file.path().to_str().expect("file path utf8")],
        Default::default(),
    )
    .expect("spawn ordex");
    config_test_support::wait_normal_mode(&mut session);

    session
        .send_text(":reload-config")
        .expect("enter reload command");
    session.send_enter().expect("execute reload command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.message_line_contains("No config file to reload")
        })
        .expect("reload command should report missing config path");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_reload_config_command_applies_relative_line_numbers() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\ndelta\n")
        .expect("seed file");

    let config = config_test_support::write_config(
        r#"
[editor]
relative_line_numbers = false
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("jj").expect("move to third line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3:1")
                && s.row_contains(1, "  1 alpha")
                && s.row_contains(3, "  3 gamma")
        })
        .expect("initial absolute line numbers");

    fs::write(
        config.path(),
        r#"
[editor]
relative_line_numbers = true
"#,
    )
    .expect("rewrite config");

    session
        .send_text(":reload-config")
        .expect("enter reload command");
    session.send_enter().expect("execute reload command");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3:1")
                && s.message_line_contains("Config reloaded")
                && s.row_contains(1, "  2 alpha")
                && s.row_contains(2, "  1 beta")
                && s.row_contains(3, "  3 gamma")
        })
        .expect("reload should apply relative line numbers");

    assert!(snapshot.row_contains(1, "  2 alpha"));
    assert!(snapshot.row_contains(2, "  1 beta"));
    assert!(snapshot.row_contains(3, "  3 gamma"));

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_reload_config_command_applies_updated_theme() {
    let config = config_test_support::write_config(
        r#"
[editor]
theme = "nord"
"#,
    );
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("syntax")
        .join("sample.rs");

    let mut session = PtySession::spawn(
        config_test_support::ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            fixture.to_str().expect("fixture path utf8"),
        ],
        Default::default(),
    )
    .expect("spawn ordex with theme config");
    config_test_support::wait_normal_mode(&mut session);
    session.read_available().expect("collect transcript");
    assert!(
        session.snapshot().contains("\u{1b}[38;5;109m\u{1b}[1m"),
        "initial theme should apply nord keyword styling"
    );

    fs::write(
        config.path(),
        r#"
[editor]
theme = "bogster"
"#,
    )
    .expect("rewrite config");

    session
        .send_text(":reload-config")
        .expect("enter reload command");
    session.send_enter().expect("execute reload command");
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Config reloaded") && s.contains("\u{1b}[38;5;179m\u{1b}[1m")
        })
        .expect("reload should apply bogster theme");

    assert!(snapshot.contains("\u{1b}[38;5;179m\u{1b}[1m"));

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_light_theme_applies_explicit_default_text_color() {
    let config = config_test_support::write_config(
        r#"
[editor]
theme = "catppuccin-latte"
"#,
    );
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"plain text\n").expect("seed file");

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.read_available().expect("collect transcript");
    let snapshot = session.snapshot();

    assert!(snapshot.row_contains(1, "plain text"));
    assert!(
        snapshot.contains("\u{1b}[38;5;59m") && snapshot.contains("\u{1b}[48;5;255m"),
        "light themes should paint both the text foreground and the light background"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
