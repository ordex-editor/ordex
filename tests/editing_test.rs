use std::fs;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_insert_text_and_save() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    session.send_text("i world").expect("type in insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_contains(1, "worldhello")
        })
        .expect("wait for insert mode render");

    session.exit_to_normal_mode(Duration::from_secs(2));

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1:7") && s.row_contains(1, "worldhello")
        })
        .expect("cursor should have moved");

    session.send_text(":wq").expect("send save and quit");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, " worldhello");

    // Reopen the saved file and verify the written text is visible on screen.
    let mut reopen = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex for reopen");

    reopen
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "worldhello")
        })
        .expect("saved text should be visible after reopen");

    reopen.send_text(":q").expect("quit reopen session");
    reopen.send_enter().expect("execute quit");
    reopen
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("reopen quit cleanly");
}

#[test]
fn test_open_line_bindings_in_normal_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1:1")
        })
        .expect("wait for initial render");

    // Open a line below line 1 and type content.
    session.send_text("ox").expect("open line below and type");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("2:2")
                && s.row_contains(2, "x")
        })
        .expect("line opened below and insert mode active");

    // Open a line above the current line and type content.
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("2:1")
        })
        .expect("cursor after first open-line edit");
    session.send_text("Oy").expect("open line above and type");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("2:2")
                && s.row_contains(2, "y")
        })
        .expect("line opened above and insert mode active");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("2:1")
        })
        .expect("cursor after second open-line edit");
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha\ny\nx\nbeta");
}

#[test]
fn test_inner_word_bindings_in_normal_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "alpha beta")
        })
        .expect("wait for initial render");

    session.send_text("diw").expect("delete inner word");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "beta")
        })
        .expect("diw should delete first word and stay in normal");

    session
        .send_text("ciwz")
        .expect("change inner word and insert text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_contains(1, "z")
        })
        .expect("ciw should enter insert mode");
    session.exit_to_normal_mode(Duration::from_secs(2));

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, " z");
}

#[test]
fn test_delete_around_paren_binding() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"x(a(b)c)y").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "x(a(b)c)y")
        })
        .expect("wait for initial render");

    session
        .send_text("llllda(")
        .expect("move to inner paren and delete around");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "x(ac)y")
        })
        .expect("da( should delete smallest surrounding pair");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "x(ac)y");
}

#[test]
fn test_ciw_with_space_still_allows_escape_to_normal_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "alpha beta")
        })
        .expect("wait for initial render");

    session
        .send_text("ciwC o")
        .expect("change inner word then insert text containing a space");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_contains(1, "C o")
        })
        .expect("ciw should still be in insert mode after spaced text");

    session.exit_to_normal_mode(Duration::from_secs(2));
}

#[test]
fn test_ciw_space_escape_in_same_input_burst_returns_normal_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "alpha beta")
        })
        .expect("wait for initial render");

    session
        .send_text("ciwC o\x1b")
        .expect("send ciw, spaced insert text, and escape in one burst");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("should return to normal mode");
}

#[test]
fn test_visual_delete_removes_selected_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abcd")
        })
        .expect("wait for initial render");

    session
        .send_text("vld")
        .expect("select two chars and delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "1 cd")
        })
        .expect("visual delete should remove selected text");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "cd\n");
}

#[test]
fn test_visual_change_enters_insert_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abcd")
        })
        .expect("wait for initial render");

    session
        .send_text("vlcZ")
        .expect("select two chars, change, and insert");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_contains(1, "1 Zcd")
        })
        .expect("visual change should enter insert mode");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "Zcd\n");
}

#[test]
fn test_ciw_space_escape_followed_by_o_does_not_stick_in_insert() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "alpha beta")
        })
        .expect("wait for initial render");

    session
        .send_text("ciwC o\x1b")
        .expect("insert spaced text then escape");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("should not remain in insert mode");

    session
        .send_text(":")
        .expect("ensure next key is handled from normal");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("COMMAND ")
        })
        .expect("should enter command mode after exiting insert");
}

#[test]
fn test_user_repro_one_line_ciw_c_space_o_escape_exits_insert() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"One line").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "One line")
        })
        .expect("wait for initial render");

    session
        .send_text("ciwC o\x1b")
        .expect("user repro: ciw then insert 'C o' then escape");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("Esc should leave insert mode");
}

#[test]
fn test_edit_closing_block_comment_rehighlights_following_code() {
    let file = std::env::temp_dir().join(format!("ordex_edit_syntax_{}.rs", std::process::id()));
    fs::write(&file, b"/* open comment\nfn main() {}\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ") && snapshot.row_contains(2, "fn main() {}")
        })
        .expect("wait for initial render");

    session
        .read_available()
        .expect("collect initial transcript");
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("\u{1b}[38;5;179m\u{1b}[1mfn"),
        "open block comment should not highlight following code as Rust yet"
    );

    session.clear_transcript();
    session.send_text("$a */").expect("close block comment");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "fn main() {}")
        })
        .expect("code line should remain visible after edit");

    session.read_available().expect("collect edited transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.row_contains(2, "fn main() {}"));
    assert!(
        snapshot.contains("\u{1b}[38;5;179m\u{1b}[1m") || snapshot.contains("\u{1b}[38;5;173m"),
        "closing the block comment should restore Rust syntax highlighting"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
    let _ = fs::remove_file(file);
}
