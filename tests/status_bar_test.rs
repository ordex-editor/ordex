use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::thread;
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
                && s.row_trimmed_ends_with(1, "status")
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
                && s.row_trimmed_ends_with(1, "status")
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

/// Undo status warnings should remain visible across idle background polls.
#[test]
fn test_undo_oldest_change_message_persists_until_next_input() {
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("initial normal mode");

    // Trigger the empty undo warning without modifying the buffer first.
    session.send_text("u").expect("undo at oldest change");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Already at oldest change")
        })
        .expect("undo warning visible");

    // Stay idle long enough for the background poll loop to run multiple times.
    thread::sleep(Duration::from_millis(200));
    session.read_available().expect("read idle redraw output");
    assert!(
        session
            .snapshot()
            .message_line_contains("Already at oldest change")
    );

    // The next input should clear the old warning as part of the user-driven redraw.
    session.send_text("l").expect("move cursor after warning");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/1:2") && !s.message_line_contains("Already at oldest change")
        })
        .expect("next input clears undo warning");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Jump-history boundary warnings should remain visible across idle background polls.
#[test]
fn test_jump_history_boundary_message_persists_until_next_input() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 01\nline 02\nline 03\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/3:1"))
        .expect("initial cursor");

    session.send_text("G").expect("jump to last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("3/3:1"))
        .expect("cursor at last line");

    session
        .send_text("\t")
        .expect("attempt to jump past newest history entry");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3/3:1") && s.message_line_contains("Already at newest jump")
        })
        .expect("jump history warning visible");

    thread::sleep(Duration::from_millis(200));
    session.read_available().expect("read idle redraw output");
    assert!(
        session
            .snapshot()
            .message_line_contains("Already at newest jump")
    );

    session
        .send_text("h")
        .expect("move cursor after jump warning");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3/3:1") && !s.message_line_contains("Already at newest jump")
        })
        .expect("next input clears jump history warning");

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
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "fn main() {}")
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

#[test]
fn test_status_bar_shows_total_line_count_for_multi_line_buffer() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 01\nline 02\nline 03\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/3:1"))
        .expect("initial position shows total line count");

    session.send_text("G").expect("jump to last line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("3/3:1"))
        .expect("last line shows matching total");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_status_bar_total_line_count_updates_after_inserting_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"single\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:1"))
        .expect("single-line buffer");

    session
        .send_text("Go")
        .expect("open line below and enter insert");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("2/2:1")
        })
        .expect("new line increases total count");

    session.send_escape().expect("return to normal mode");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("2/2:1"))
        .expect("total persists in normal mode");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_status_bar_total_line_count_updates_after_deleting_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/2:1"))
        .expect("two-line buffer");

    session.send_text("jdd").expect("delete second line");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1/1:5"))
        .expect("deleted line reduces total count");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
