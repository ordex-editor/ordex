use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one readable system file that the current user still cannot write.
fn readable_unwritable_system_file() -> Option<PathBuf> {
    ["/etc/pacman.conf", "/etc/passwd"]
        .into_iter()
        .map(Path::new)
        .find(|path| {
            path.exists()
                && fs::File::open(path).is_ok()
                && OpenOptions::new().write(true).open(path).is_err()
        })
        .map(Path::to_path_buf)
}

#[test]
fn test_status_bar_mode_transitions() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains(file.path().file_name().unwrap().to_str().unwrap())
                && s.row_contains(1, "status")
        })
        .expect("initial normal mode");

    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .expect("insert mode visible");

    session.send_escape().expect("back to normal");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored");

    session.send_text(":").expect("enter command mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ") && s.message_line_contains(":")
        })
        .expect("command mode visible");

    session.send_escape().expect("cancel command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after command cancel");
    session.send_text("/").expect("enter search mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.message_line_contains("/")
        })
        .expect("search mode visible");

    session.send_escape().expect("cancel search");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after search cancel");

    session.send_text("v").expect("enter visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("visual mode visible");

    session.send_escape().expect("cancel visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after visual cancel");

    session.send_text("V").expect("enter visual line mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-LINE ")
        })
        .expect("visual line mode visible");

    session.send_escape().expect("cancel visual line mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after visual line cancel");

    session
        .send_text("\u{16}")
        .expect("enter visual block mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("V-BLOCK ")
        })
        .expect("visual block mode visible");

    session.send_escape().expect("cancel visual block mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("normal mode restored after visual block cancel");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_tab_strip_remains_visible_with_single_buffer() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains(file.path().file_name().unwrap().to_str().unwrap())
                && s.row_contains(1, "status")
        })
        .expect("single-buffer tab strip visible");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_pending_g_indicator_on_message_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && !s.message_line_contains("g")
        })
        .expect("initial normal mode");

    session.send_text("g").expect("start sequence prefix");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("g"))
        .expect("pending marker visible");

    session.send_text("i").expect("mismatch consumes both");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && !s.message_line_contains("g")
        })
        .expect("marker cleared after mismatch");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_pending_find_indicator_on_message_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && !s.message_line_contains("f")
        })
        .expect("initial normal mode");

    session.send_text("f").expect("start find prefix");
    session
        .wait_until(Duration::from_secs(2), |s| s.message_line_contains("f"))
        .expect("pending find marker visible");

    session.send_escape().expect("cancel pending find");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && !s.message_line_contains("f")
        })
        .expect("pending marker cleared after escape");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_goto_definition_unsupported_project_message_updates_status_bar() {
    let file = TempFile::with_suffix(".rs").expect("create temp file");
    file.write_all(b"fn main() {}\n").expect("seed file");
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {}")
        })
        .expect("wait for rust file");

    session.send_text("gd").expect("request definition");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.message_line_contains("is not inside a supported Rust project root")
                && screen.status_line_contains("NORMAL ")
        })
        .expect("unsupported-project message should update the message line");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_status_bar_shows_read_only_indicator_for_read_only_file() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"status\n").expect("seed file");
    let mut permissions = fs::metadata(file.path())
        .expect("stat temp file")
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(file.path(), permissions).expect("make file read-only");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains(&format!(
                    "{} 🔒",
                    file.path().file_name().unwrap().to_str().unwrap()
                ))
        })
        .expect("read-only indicator visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify that system files opened read-only for the current user show the indicator.
#[test]
fn test_status_bar_shows_read_only_indicator_for_user_unwritable_system_file() {
    let Some(path) = readable_unwritable_system_file() else {
        return;
    };

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[path.to_str().expect("utf8 system path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains(&format!(
                    "{} 🔒",
                    path.file_name().unwrap().to_str().unwrap()
                ))
        })
        .expect("system read-only indicator visible");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
