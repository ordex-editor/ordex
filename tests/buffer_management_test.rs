use std::fs;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree};

/// Return the compiled Ordex binary path for integration tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return the stable escape sequence used for the active tab in the default theme.
fn active_tab_escape() -> &'static str {
    "\u{1b}[48;5;74m\u{1b}[38;5;234m\u{1b}[1m"
}

#[test]
fn test_multiple_startup_files_support_buffer_switching_commands() {
    let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains("_first.txt")
                && s.tab_line_contains("_second.txt")
                && s.row_contains(1, "first buffer")
        })
        .expect("wait for first startup buffer");

    session.send_text(":bn").expect("switch to next buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains("_second.txt")
                && s.row_contains(1, "second buffer")
        })
        .expect("wait for second buffer");

    session.send_text(":bp").expect("switch to previous buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.tab_line_contains("_first.txt")
                && s.row_contains(1, "first buffer")
        })
        .expect("wait for first buffer again");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_tab_strip_tracks_active_buffer_switches() {
    let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.tab_line_contains("/t/")
                && s.tab_line_contains("_second.txt")
                && s.row_contains(1, "first buffer")
        })
        .expect("initial tabs visible");
    assert!(
        snapshot.tab_line_contains("/t/"),
        "tab strip should render trimmed paths: {}",
        snapshot.tab_line().unwrap_or_default()
    );

    session.clear_transcript();
    session.send_text(":bn").expect("switch to next buffer");
    session.send_enter().expect("execute switch");
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.tab_line_contains("_first.txt")
                && s.tab_line_contains("_second.txt")
                && s.row_contains(1, "second buffer")
        })
        .expect("tabs should still show both buffers after switch");
    assert!(
        snapshot.raw().matches(active_tab_escape()).count() >= 2,
        "active tab should use accent styling after buffer switch:\n{}",
        snapshot.raw()
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_narrow_tab_strip_drops_modified_markers_before_labels() {
    let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        PtySessionConfig {
            cols: 32,
            rows: 10,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.tab_line_contains("/t/") && s.row_contains(1, "first buffer")
        })
        .expect("initial narrow tabs visible");

    session.send_text("ix").expect("modify first buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.tab_line_contains("/t/") && s.row_contains(1, "xfirst buffer")
        })
        .expect("modified first buffer visible");

    assert!(
        !snapshot.tab_line_contains("+"),
        "narrow tab strip should drop modified markers before labels: {}",
        snapshot.tab_line().unwrap_or_default()
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_wide_tab_strip_keeps_modified_marker_with_many_buffers() {
    let mut files = Vec::new();
    for suffix in [
        "_one.txt",
        "_two.txt",
        "_three.txt",
        "_four.txt",
        "_five.txt",
    ] {
        let file = TempFile::with_suffix(suffix).expect("create temp file");
        file.write_all(b"buffer\n").expect("seed temp file");
        files.push(file);
    }
    let args = files
        .iter()
        .map(|file| file.path().to_str().unwrap())
        .collect::<Vec<_>>();

    let mut session = PtySession::spawn(
        ordex_bin(),
        &args,
        PtySessionConfig {
            cols: 80,
            rows: 10,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.tab_line_contains("_one.txt"))
        .expect("initial wide tabs visible");

    session.send_text("ix").expect("modify first buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.tab_line_contains("+") && s.row_contains(1, "xbuffer")
        })
        .expect("modified marker should stay visible on wide terminals");

    assert!(
        snapshot.tab_line_contains("+"),
        "wide tab strip should keep modified markers even with many buffers: {}",
        snapshot.tab_line().unwrap_or_default()
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_buffer_switch_picker_filters_and_confirms_selection() {
    let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");
    let third = TempFile::with_suffix("_third.txt").expect("create third temp file");
    third.write_all(b"third buffer\n").expect("seed third file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
            third.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "first buffer")
        })
        .expect("wait for first startup buffer");

    session
        .send_text(" bsecond")
        .expect("open picker and type filter");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains(second.path().to_str().unwrap())
                && s.contains(first.path().to_str().unwrap())
                && !s.row_contains(1, "second buffer")
        })
        .expect("picker should show filtered second buffer plus disabled active entry");

    session.send_enter().expect("confirm picker selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "second buffer")
        })
        .expect("second buffer should become active");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_multi_buffer_edits_track_saved_and_unsaved_buffers_independently() {
    let first = TempFile::new().expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::new().expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");
    let third = TempFile::new().expect("create third temp file");
    third.write_all(b"third buffer\n").expect("seed third file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
            third.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "first buffer")
        })
        .expect("wait for first startup buffer");

    session.send_text("iA").expect("edit first buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "Afirst buffer")
        })
        .expect("first buffer edit should be visible");

    session.send_text(":bn").expect("switch to second buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "second buffer")
                && !s.row_contains(1, "Afirst buffer")
        })
        .expect("second buffer should remain isolated");

    session.send_text("iB").expect("edit second buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "Bsecond buffer")
        })
        .expect("second buffer edit should be visible");

    session
        .send_text(":bp")
        .expect("switch back to first buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "Afirst buffer")
                && !s.row_contains(1, "Bsecond buffer")
        })
        .expect("first buffer edit should still be present");

    session.send_text(":w").expect("save first buffer");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "Afirst buffer")
                && s.message_line_contains("written")
        })
        .expect("first buffer should save");

    session
        .send_text(":bn")
        .expect("switch to second buffer again");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "Bsecond buffer")
        })
        .expect("second buffer edit should still be present");

    session.send_text(":bn").expect("switch to third buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "third buffer")
                && !s.row_contains(1, "Afirst buffer")
                && !s.row_contains(1, "Bsecond buffer")
        })
        .expect("third buffer should remain isolated");

    session.send_text("iC").expect("edit third buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "Cthird buffer")
        })
        .expect("third buffer edit should be visible");

    session
        .send_text(":bp")
        .expect("switch back to second buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "Bsecond buffer")
                && !s.row_contains(1, "Cthird buffer")
        })
        .expect("second buffer should still be visible after leaving unsaved third buffer");

    session.send_text(":w").expect("save second buffer");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "Bsecond buffer")
                && s.message_line_contains("written")
        })
        .expect("second buffer should save");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains(third.path().file_name().unwrap().to_str().unwrap())
        })
        .expect("quit should prompt for unsaved third buffer");
    session
        .send_text("n")
        .expect("discard third buffer changes");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit after discarding unsaved third buffer");

    assert_eq!(
        fs::read_to_string(first.path()).expect("read first file"),
        "Afirst buffer\n"
    );
    assert_eq!(
        fs::read_to_string(second.path()).expect("read second file"),
        "Bsecond buffer\n"
    );
    assert_eq!(
        fs::read_to_string(third.path()).expect("read third file"),
        "third buffer\n"
    );
}

#[test]
fn test_buffer_delete_prompts_for_dirty_buffer_and_closes_after_discard() {
    let first = TempFile::new().expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::new().expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "first buffer")
        })
        .expect("first buffer visible");

    session.send_text(":bn").expect("switch to second buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "second buffer")
        })
        .expect("second buffer visible");

    session.send_text("ix").expect("modify second buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "xsecond buffer")
        })
        .expect("dirty second buffer visible");

    session.send_text(":bd").expect("delete dirty buffer");
    session.send_enter().expect("execute delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("before closing")
                && s.message_line_contains("[y]es/[n]o/[c]ancel")
        })
        .expect("wait for close prompt");

    session.send_text("n").expect("discard changes and close");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "first buffer")
        })
        .expect("switched back to first buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_quit_walks_each_dirty_buffer_before_exiting() {
    let first = TempFile::new().expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::new().expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "first buffer")
        })
        .expect("first buffer visible");

    session.send_text("ia").expect("modify first buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "afirst buffer")
        })
        .expect("dirty first buffer visible");

    session.send_text(":bn").expect("switch to second buffer");
    session.send_enter().expect("execute switch");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "second buffer")
        })
        .expect("second buffer visible");

    session.send_text("ib").expect("modify second buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "bsecond buffer")
        })
        .expect("dirty second buffer visible");

    session.send_text(":q").expect("request quit");
    session.send_enter().expect("execute quit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains(second.path().file_name().unwrap().to_str().unwrap())
        })
        .expect("prompt for active dirty buffer");

    session.send_text("n").expect("discard active buffer");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Save changes to")
                && s.message_line_contains(first.path().file_name().unwrap().to_str().unwrap())
        })
        .expect("prompt for remaining dirty buffer");

    session.send_text("n").expect("discard final dirty buffer");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit after resolving all dirty buffers");

    assert_eq!(
        fs::read_to_string(first.path()).expect("read first file"),
        "first buffer\n"
    );
    assert_eq!(
        fs::read_to_string(second.path()).expect("read second file"),
        "second buffer\n"
    );
}

#[test]
/// `ga` should toggle between the two most recently visited named buffers.
fn test_ga_toggles_between_recent_named_buffers() {
    let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
    first.write_all(b"first buffer\n").expect("seed first file");
    let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
    second
        .write_all(b"second buffer\n")
        .expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[first.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "first buffer")
        })
        .expect("first buffer visible");

    session.send_text(":e ").expect("start edit command");
    session
        .send_text(second.path().to_str().unwrap())
        .expect("type second path");
    session.send_enter().expect("execute edit");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "second buffer")
        })
        .expect("second buffer visible");

    session.send_text("ga").expect("jump to alternate file");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "first buffer")
        })
        .expect("ga should return to first buffer");

    session
        .send_text("ga")
        .expect("jump back to most recent alternate");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "second buffer")
        })
        .expect("second ga should return to second buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `g.` should jump back to the latest committed change even from another buffer.
fn test_gdot_jumps_to_most_recent_change_across_buffers() {
    let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
    first.write_all(b"alpha\n").expect("seed first file");
    let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
    second.write_all(b"beta\n").expect("seed second file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[first.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "alpha"))
        .expect("first buffer visible");

    session.send_text("ix").expect("modify first buffer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "xalpha")
        })
        .expect("first buffer change committed");

    session.send_text(":e ").expect("start edit command");
    session
        .send_text(second.path().to_str().unwrap())
        .expect("type second path");
    session.send_enter().expect("execute edit");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "beta"))
        .expect("second buffer visible");

    session.send_text("g.").expect("jump to last modification");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:1") && s.row_contains(1, "xalpha")
        })
        .expect("g. should return to the edited first buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `ga` should skip recent files that are no longer open.
fn test_ga_skips_closed_recent_buffer() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("one.txt", "one\n").expect("write one");
    tree.write_file("two.txt", "two\n").expect("write two");
    tree.write_file("three.txt", "three\n")
        .expect("write three");
    let one = tree.path().join("one.txt");
    let two = tree.path().join("two.txt");
    let three = tree.path().join("three.txt");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[one.to_str().unwrap()],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "one"))
        .expect("one visible");

    session.send_text(":e ").expect("start edit one->two");
    session.send_text(two.to_str().unwrap()).expect("type two");
    session.send_enter().expect("execute edit");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "two"))
        .expect("two visible");

    session.send_text(":e ").expect("start edit two->three");
    session
        .send_text(three.to_str().unwrap())
        .expect("type three");
    session.send_enter().expect("execute edit");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "three"))
        .expect("three visible");

    session.send_text(":bp").expect("switch back to two");
    session.send_enter().expect("execute buffer prev");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "two"))
        .expect("two visible again");

    session.send_text(":bd").expect("close two");
    session.send_enter().expect("execute delete");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "three"))
        .expect("three should remain after closing two");

    session
        .send_text("ga")
        .expect("jump to alternate after closing two");
    session
        .wait_until(Duration::from_secs(2), |s| s.row_contains(1, "one"))
        .expect("ga should skip closed two and open one");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
