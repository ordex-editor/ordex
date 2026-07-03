use std::fs;
use std::time::{Duration, Instant};
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Verify bracketed paste inserts a large Insert-mode payload quickly.
#[test]
fn test_bracketed_paste_in_insert_mode_is_fast() {
    let file = TempFile::new().expect("create temp file");
    let payload = (1..=100)
        .map(|line| format!("line{line:04}"))
        .collect::<Vec<_>>()
        .join("\n");
    let bracketed = format!("\u{1b}[200~{payload}\u{1b}[201~");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Wait for the initial frame before entering Insert mode and sending the paste.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("wait for initial render");
    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .expect("wait for insert mode");

    // The regression measures the full terminal-to-buffer paste path, not typing latency.
    let started = Instant::now();
    session
        .send_raw_bytes(bracketed.as_bytes())
        .expect("send bracketed paste bytes");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("100/100:9")
                && s.contains("line0100")
        })
        .expect("paste should finish");
    assert!(
        started.elapsed() <= Duration::from_millis(500),
        "paste took {:?}",
        started.elapsed()
    );

    // Save the resulting buffer so the test asserts the full pasted payload, not only the screen.
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, format!("{payload}\n"));
}

/// Verify bracketed paste in Normal mode inserts text as data instead of commands.
#[test]
fn test_bracketed_paste_in_normal_mode_inserts_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"X").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Wait for Normal mode before sending a multi-line bracketed paste payload.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "X")
        })
        .expect("wait for initial render");
    session
        .send_raw_bytes(b"\x1b[200~ab\ncd\x1b[201~")
        .expect("send normal-mode paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "Xab")
                && s.row_trimmed_ends_with(2, "cd")
        })
        .expect("normal-mode paste should insert text");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "Xab\ncd\n");
}

/// Verify single-line Normal-mode bracketed paste lands the cursor on the last inserted character.
#[test]
fn test_bracketed_paste_in_normal_mode_without_trailing_newline_ends_on_last_character() {
    let file = TempFile::new().expect("create temp file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // The empty-buffer repro should leave the cursor on `e`, not in the
    // impossible Normal-mode cell after the pasted word.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("wait for initial render");
    session
        .send_raw_bytes(b"\x1b[200~line\x1b[201~")
        .expect("send normal-mode single-line paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("1/1:4")
                && s.row_trimmed_ends_with(1, "line")
        })
        .expect("normal-mode single-line paste should end on the last character");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "line\n");
}

/// Verify bracketed paste in Visual mode replaces the active selection and returns to Normal mode.
#[test]
fn test_bracketed_paste_in_visual_mode_replaces_selected_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Select the first two characters so the paste must replace, not append after them.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("wait for initial render");
    session.send_text("vl").expect("enter visual selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("wait for visual mode");
    session
        .send_raw_bytes(b"\x1b[200~X\x1b[201~")
        .expect("send visual paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "Xcd")
        })
        .expect("visual paste should replace the selection");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "Xcd\n");
}

/// Verify Insert-mode bracketed paste ending with a newline creates a real blank EOF line.
#[test]
fn test_bracketed_paste_in_insert_mode_trailing_newline_creates_real_blank_line() {
    let file = TempFile::new().expect("create temp file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Enter Insert mode, paste one newline-terminated line, and confirm the
    // cursor lands on the next logical line instead of a rendered sentinel.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("wait for initial render");
    session.send_text("i").expect("enter insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .expect("wait for insert mode");
    session
        .send_raw_bytes(b"\x1b[200~line\n\x1b[201~")
        .expect("send insert-mode paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("2/2:1")
                && s.row_trimmed_ends_with(1, "line")
        })
        .expect("insert-mode trailing-newline paste should create line 2");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "line\n\n");
}

/// Verify Normal-mode bracketed paste ending with a newline creates a real blank EOF line.
#[test]
fn test_bracketed_paste_in_normal_mode_trailing_newline_creates_real_blank_line() {
    let file = TempFile::new().expect("create temp file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // A newline-terminated payload should create a real blank next line even
    // when the Normal-mode paste starts from an empty buffer.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("wait for initial render");
    session
        .send_raw_bytes(b"\x1b[200~line\n\x1b[201~")
        .expect("send normal-mode paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("2/2:1")
                && s.row_trimmed_ends_with(1, "line")
        })
        .expect("normal-mode trailing-newline paste should create line 2");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "line\n\n");
}

/// Verify Visual-mode bracketed paste ending with a newline replaces with a real EOF blank line.
#[test]
fn test_bracketed_paste_in_visual_mode_trailing_newline_creates_real_blank_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"x").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Replacing the only selected character leaves the paste at EOF, so this
    // regression proves the new blank line is backed by real buffer content.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "x")
        })
        .expect("wait for initial render");
    session.send_text("v").expect("enter visual mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("wait for visual mode");
    session
        .send_raw_bytes(b"\x1b[200~line\n\x1b[201~")
        .expect("send visual-mode paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.status_line_contains("2/2:1")
                && s.row_trimmed_ends_with(1, "line")
        })
        .expect("visual-mode trailing-newline paste should create line 2");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "line\n\n");
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
            s.status_line_contains("INSERT ") && s.row_trimmed_ends_with(1, "worldhello")
        })
        .expect("wait for insert mode render");

    session.exit_to_normal_mode(Duration::from_secs(2));

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("1/1:6") && s.row_trimmed_ends_with(1, "worldhello")
        })
        .expect("cursor should have moved");

    session.send_text(":wq").expect("send save and quit");
    session.send_enter().expect("send enter");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, " worldhello\n");

    // Reopen the saved file and verify the written text is visible on screen.
    let mut reopen = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex for reopen");

    reopen
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "worldhello")
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
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2:1")
        })
        .expect("wait for initial render");

    // Open a line below line 1 and type content.
    session.send_text("ox").expect("open line below and type");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("2/3:2")
                && s.row_trimmed_ends_with(2, "x")
        })
        .expect("line opened below and insert mode active");

    // Open a line above the current line and type content.
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("2/3:1")
        })
        .expect("cursor after first open-line edit");
    session.send_text("Oy").expect("open line above and type");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("2/4:2")
                && s.row_trimmed_ends_with(2, "y")
        })
        .expect("line opened above and insert mode active");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("2/4:1")
        })
        .expect("cursor after second open-line edit");
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha\ny\nx\nbeta\n");
}

#[test]
/// Verify `O` from an empty file opens a distinct top line that survives save.
fn test_open_line_above_in_empty_file_preserves_opened_blank_line() {
    let file = TempFile::new().expect("create temp file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Typing after `O` should edit the new top line and leave the original blank line below it.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/1:1")
        })
        .expect("wait for initial render");
    session.send_text("Ox").expect("open line above and type");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && s.status_line_contains("1/2:2")
                && s.row_trimmed_ends_with(1, "x")
        })
        .expect("new top line should enter insert mode");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "x\n\n");
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta")
        })
        .expect("wait for initial render");

    session.send_text("diw").expect("delete inner word");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "beta")
        })
        .expect("diw should delete first word and stay in normal");

    session
        .send_text("ciwz")
        .expect("change inner word and insert text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_trimmed_ends_with(1, "z")
        })
        .expect("ciw should enter insert mode");
    session.exit_to_normal_mode(Duration::from_secs(2));

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, " z\n");
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "x(a(b)c)y")
        })
        .expect("wait for initial render");

    session
        .send_text("llllda(")
        .expect("move to inner paren and delete around");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "x(ac)y")
        })
        .expect("da( should delete smallest surrounding pair");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "x(ac)y\n");
}

#[test]
fn test_dw_binding_deletes_to_next_word_boundary() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha beta gamma").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta gamma")
        })
        .expect("wait for initial render");

    session
        .send_text("dw")
        .expect("delete to next word boundary");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1).is_some_and(|line| {
                    let line = line.trim_end();
                    line.ends_with("beta gamma") && !line.contains("alpha")
                })
        })
        .expect("dw should delete the first word and following separator");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "beta gamma\n");
}

#[test]
fn test_yy_binding_yanks_current_line_linewise() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1)
                    .is_some_and(|line| line.trim_end().ends_with("alpha"))
                && s.row(2)
                    .is_some_and(|line| line.trim_end().ends_with("beta"))
        })
        .expect("wait for initial render");

    session
        .send_text("yyp")
        .expect("yank current line and paste it below");
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
        .expect("yy should paste a duplicate line below the cursor");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha\nalpha\nbeta\n");
}

#[test]
fn test_c_e_binding_changes_through_big_word_end() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha.beta rest").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha.beta rest")
        })
        .expect("wait for initial render");

    session
        .send_text("cEZ")
        .expect("change through WORD end and insert replacement");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_trimmed_ends_with(1, "Z rest")
        })
        .expect("cE should delete alpha.beta and enter insert mode");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "Z rest\n");
}

#[test]
fn test_ye_then_p_pastes_yanked_span() {
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta")
        })
        .expect("wait for initial render");

    session
        .send_text("yewP")
        .expect("yank to word end, move, and paste");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha alphabeta")
        })
        .expect("ye should populate the yank buffer for subsequent paste");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha alphabeta\n");
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta")
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta")
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("wait for initial render");

    session
        .send_text("vld")
        .expect("select two chars and delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "1 cd")
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("wait for initial render");

    session
        .send_text("vlcZ")
        .expect("select two chars, change, and insert");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.row_trimmed_ends_with(1, "1 Zcd")
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
fn test_visual_block_delete_removes_rectangular_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd\na\nabc\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("wait for initial render");

    session
        .send_text("l\u{16}jjld")
        .expect("select block and delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1).is_some_and(|line| line.trim_end() == "   1 ad")
                && s.row(2).is_some_and(|line| line.trim_end() == "   2 a")
                && s.row(3).is_some_and(|line| line.trim_end() == "   3 a")
        })
        .expect("visual block delete should remove the rectangular slice");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "ad\na\na\n");
}

#[test]
fn test_visual_block_yank_then_paste_interleaves_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcd\na\nabc\nwxyz\nmno\npqrs\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "abcd")
        })
        .expect("wait for initial render");

    session
        .send_text("l\u{16}jjlyj0lP")
        .expect("select block, yank it, move, and paste before cursor");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1).is_some_and(|line| line.trim_end() == "   1 abcd")
                && s.row(2).is_some_and(|line| line.trim_end() == "   2 a")
                && s.row(3).is_some_and(|line| line.trim_end() == "   3 abc")
                && s.row(4)
                    .is_some_and(|line| line.trim_end() == "   4 wbcxyz")
                && s.row(5).is_some_and(|line| line.trim_end() == "   5 mno")
                && s.row(6)
                    .is_some_and(|line| line.trim_end() == "   6 pbcqrs")
        })
        .expect("visual block paste should interleave the rectangular text at the new target");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "abcd\na\nabc\nwbcxyz\nmno\npbcqrs\n");
}

#[test]
fn test_visual_block_a_appends_at_block_end_on_each_selected_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"fn main() {\n    println!(\"Hello, world!\");\n}\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for initial render");

    session
        .send_text("lll\u{16}jlllA123")
        .expect("select block and append at its right edge");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1)
                    .is_some_and(|line| line.trim_end() == "   1 fn main123() {")
                && s.row(2).is_some_and(|line| {
                    line.trim_end() == "   2     pri123ntln!(\"Hello, world!\");"
                })
                && s.row(3).is_some_and(|line| line.trim_end() == "   3 }")
        })
        .expect("visual block A should append at the selected block edge");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(
        saved,
        "fn main123() {\n    pri123ntln!(\"Hello, world!\");\n}\n"
    );
}

#[test]
fn test_visual_block_a_pads_short_last_line_to_block_end() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"fn main() {\n    println!(\"Hello, world!\");\n}\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for initial render");

    session
        .send_text("llllll\u{16}jjA123")
        .expect("select block through short last line and append at block end");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1)
                    .is_some_and(|line| line.trim_end() == "   1 fn main123() {")
                && s.row(2).is_some_and(|line| {
                    line.trim_end() == "   2     pri123ntln!(\"Hello, world!\");"
                })
                && s.row(3)
                    .is_some_and(|line| line.trim_end() == "   3 }      123")
        })
        .expect("visual block A should pad short rows to the block end");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(
        saved,
        "fn main123() {\n    pri123ntln!(\"Hello, world!\");\n}      123\n"
    );
}

#[test]
fn test_visual_block_i_uses_block_start_on_short_last_line() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"fn main() {\n    println!(\"Hello, world!\");\n}\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "fn main() {")
        })
        .expect("wait for initial render");

    session
        .send_text("llllll\u{16}jjI123")
        .expect("select block through short last line and insert at block start");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row(1)
                    .is_some_and(|line| line.trim_end() == "   1 123fn main() {")
                && s.row(2).is_some_and(|line| {
                    line.trim_end() == "   2 123    println!(\"Hello, world!\");"
                })
                && s.row(3).is_some_and(|line| line.trim_end() == "   3 123}")
        })
        .expect("visual block I should anchor to the block start on every row");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(
        saved,
        "123fn main() {\n123    println!(\"Hello, world!\");\n123}\n"
    );
}

#[test]
fn test_undo_and_redo_insert_session_bindings() {
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "hello")
        })
        .expect("wait for initial render");

    session.send_text("iXY").expect("insert text");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "XYhello")
        })
        .expect("insert session should be visible");

    session.send_text("u").expect("undo insert session");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "hello")
        })
        .expect("undo should restore original text");

    session.send_text("\x12").expect("redo insert session");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "XYhello")
        })
        .expect("redo should restore inserted text");

    session.send_text(":q!").expect("quit session");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha beta")
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
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "One line")
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
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    file.write_all(b"/* open comment\nfn main() {}\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
                && snapshot.row_trimmed_ends_with(2, "fn main() {}")
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
    // Wait until both the code line is visible and the keyword highlighting
    // escape has been emitted to confirm the re-highlight render completed.
    let snapshot = session
        .wait_until(Duration::from_secs(4), |snapshot| {
            snapshot.row_trimmed_ends_with(2, "fn main() {}")
                && (snapshot.contains("\u{1b}[38;5;179m\u{1b}[1m")
                    || snapshot.contains("\u{1b}[38;5;173m"))
        })
        .expect("closing the block comment should restore Rust syntax highlighting");

    assert!(snapshot.row_trimmed_ends_with(2, "fn main() {}"));
    assert!(
        snapshot.contains("\u{1b}[38;5;179m\u{1b}[1m") || snapshot.contains("\u{1b}[38;5;173m"),
        "closing the block comment should restore Rust syntax highlighting"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_delete_to_last_line_with_g_motion() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha")
        })
        .expect("wait for initial render");

    // Move to "beta" (line 2) then dG: deletes "beta" and "gamma", leaving "alpha".
    session.send_text("j").expect("move to second line");
    session
        .send_text("dG")
        .expect("delete from current line to end of file");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.status_line_contains("1/1:")
        })
        .expect("wait for deletion");

    session.send_text("p").expect("paste yanked text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(2, "beta")
                && s.row_trimmed_ends_with(3, "gamma")
        })
        .expect("wait for paste");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_delete_to_first_line_with_gg_motion() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha")
        })
        .expect("wait for initial render");

    session.send_text("j").expect("move to second line");
    session
        .send_text("dgg")
        .expect("delete from first line to current line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "gamma")
                && s.status_line_contains("1/1:")
        })
        .expect("wait for deletion");

    session.send_text("p").expect("paste yanked text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "gamma")
                && s.row_trimmed_ends_with(2, "alpha")
                && s.row_trimmed_ends_with(3, "beta")
        })
        .expect("wait for paste");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_change_to_last_line_with_g_motion() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha")
        })
        .expect("wait for initial render");

    // Move to "beta" (line 2) then cG: replaces "beta" and "gamma" with new content.
    session.send_text("j").expect("move to second line");
    session
        .send_text("cG")
        .expect("change from current line to end of file");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .expect("wait for insert mode");
    session.send_text("new content").expect("enter new content");
    session.send_escape().expect("exit insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.row_trimmed_ends_with(2, "new content")
                && s.status_line_contains("2/2:")
        })
        .expect("wait for change");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_yank_to_last_line_with_g_motion() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha")
        })
        .expect("wait for initial render");

    // Move to "beta" (line 2) then yG: yanks "beta" and "gamma".
    session.send_text("j").expect("move to second line");
    session
        .send_text("yG")
        .expect("yank from current line to end of file");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for yank");

    // Paste after "alpha": buffer becomes alpha / beta / gamma / beta / gamma.
    session.send_text("gg").expect("move to first line");
    session.send_text("p").expect("paste yanked text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.row_trimmed_ends_with(2, "beta")
                && s.row_trimmed_ends_with(3, "gamma")
                && s.row_trimmed_ends_with(4, "beta")
                && s.row_trimmed_ends_with(5, "gamma")
        })
        .expect("wait for paste");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_yank_to_first_line_with_gg_motion() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha")
        })
        .expect("wait for initial render");

    // Move to "beta" (line 2) then ygg: yanks "alpha" and "beta".
    session.send_text("j").expect("move to second line");
    session
        .send_text("ygg")
        .expect("yank from first line to current line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for yank");

    // Paste below "beta": buffer becomes alpha / beta / alpha / beta / gamma.
    session.send_text("p").expect("paste yanked text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "alpha")
                && s.row_trimmed_ends_with(2, "beta")
                && s.row_trimmed_ends_with(3, "alpha")
                && s.row_trimmed_ends_with(4, "beta")
                && s.row_trimmed_ends_with(5, "gamma")
        })
        .expect("wait for paste");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_macro_recording_replays_insert_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "ab")
        })
        .expect("wait for initial render");

    session
        .send_text("qaiX")
        .expect("start recording and insert text");
    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "Xab")
        })
        .expect("inserted text should remain visible while recording");
    session
        .send_text("q@a")
        .expect("stop recording and replay macro");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "XXab")
        })
        .expect("macro replay should repeat the insert session");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_macro_recording_replays_command_mode_input() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"one\ntwo\nthree\nfour\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/4:1")
        })
        .expect("wait for initial render");

    session
        .send_text("qa:3")
        .expect("start recording goto-line macro");
    session.send_enter().expect("execute goto-line command");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(3, "three")
        })
        .expect("recorded command should move to line three");

    session
        .send_text("qgg@a")
        .expect("stop recording, reset cursor, and replay");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("3/4:1"))
        .expect("macro replay should repeat the recorded command");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_delete_inner_double_quote() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"hello world\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"hello world\"")
        })
        .expect("wait for initial render");

    // Move cursor to 'h' (index 1) then delete inner quote.
    session
        .send_text("ldi\"")
        .expect("move right and delete inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"\"")
        })
        .expect("di\" should delete content between quotes");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "\"\"\n");
}

#[test]
fn test_delete_around_double_quote() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"x\"hello\"y").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "x\"hello\"y")
        })
        .expect("wait for initial render");

    // Move cursor to 'h' (inside the quotes) then delete around quote.
    session
        .send_text("llda\"")
        .expect("move inside quote and delete around");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "xy")
        })
        .expect("da\" should delete the quoted span including delimiters");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "xy\n");
}

#[test]
fn test_change_inner_single_quote() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"let x = 'abc'").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "let x = 'abc'")
        })
        .expect("wait for initial render");

    // Move cursor to 'a' (index 9) then change inner single-quote.
    session
        .send_text("lllllllllci'")
        .expect("move to inner content and change");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
                && (s.row_trimmed_ends_with(1, "let x = ''")
                    || s.row_trimmed_ends_with(1, "let x = '"))
        })
        .expect("ci' should delete inner content and enter insert mode");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit without save");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_delete_inner_backtick() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"`expr`").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "`expr`")
        })
        .expect("wait for initial render");

    // Move cursor to 'e' (index 1) then delete inner backtick.
    session
        .send_text("ldi`")
        .expect("move inside backtick and delete inner");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "``")
        })
        .expect("di` should delete content between backticks");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "``\n");
}

#[test]
fn test_delete_inner_angle_bracket() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"<hello>").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "<hello>")
        })
        .expect("wait for initial render");

    // Move cursor to 'h' (index 1) then delete inner angle bracket.
    session
        .send_text("ldi<")
        .expect("move inside angle bracket and delete inner");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "<>")
        })
        .expect("di< should delete content between angle brackets");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "<>\n");
}

#[test]
fn test_delete_around_angle_bracket() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"foo<T>bar").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foo<T>bar")
        })
        .expect("wait for initial render");

    // Move cursor to 'T' (index 4) then delete around angle bracket.
    session
        .send_text("lllllda<")
        .expect("move to T and delete around angle bracket");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "foobar")
        })
        .expect("da< should delete the angle-bracketed span including delimiters");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "foobar\n");
}

#[test]
fn test_delete_inner_paren_alias_b() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"(hello)").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "(hello)")
        })
        .expect("wait for initial render");

    // Move cursor to 'h' (index 1) then delete inner paren via alias `b`.
    session
        .send_text("ldib")
        .expect("move inside paren and delete inner via b alias");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "()")
        })
        .expect("dib should behave identically to di(");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "()\n");
}

#[test]
fn test_delete_inner_brace_alias_big_b() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"{value}").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "{value}")
        })
        .expect("wait for initial render");

    // Move cursor to 'v' (index 1) then delete inner brace via alias `B`.
    session
        .send_text("ldiB")
        .expect("move inside brace and delete inner via B alias");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "{}")
        })
        .expect("diB should behave identically to di{");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "{}\n");
}

#[test]
fn test_delete_inner_quote_cursor_on_open_quote() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"hi\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"hi\"")
        })
        .expect("wait for initial render");

    // Cursor starts on the opening `"` (index 0).
    session
        .send_text("di\"")
        .expect("delete inner quote from open-quote position");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"\"")
        })
        .expect("di\" with cursor on open quote should delete inner content");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "\"\"\n");
}

#[test]
fn test_delete_inner_quote_fallback_to_right_string() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"a\" x \"b\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"a\" x \"b\"")
        })
        .expect("wait for initial render");

    // Move cursor to 'x' (index 4, between the two strings) then delete inner quote.
    // The fallback should find the next string to the right and delete its contents.
    session
        .send_text("llllldi\"")
        .expect("move to x and delete inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"a\" x \"\"")
        })
        .expect("di\" outside a string should select the next string to the right");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "\"a\" x \"\"\n");
}

#[test]
fn test_yank_inner_double_quote_then_paste() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"abc\" x").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"abc\" x")
        })
        .expect("wait for initial render");

    // Move to 'a' (index 1), yank inner quote, move to 'x' (index 6), paste after.
    session
        .send_text("lyi\"lllllp")
        .expect("yank inner quote then paste after x");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"abc\" xabc")
        })
        .expect("yi\" should yank the inner content; p should paste it");

    session.send_text(":q!").expect("quit without save");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_visual_select_inner_double_quote() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"\"hello\"").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"hello\"")
        })
        .expect("wait for initial render");

    // Move to 'h' (index 1), enter visual mode, select inner quote, then delete.
    session
        .send_text("lvi\"d")
        .expect("select inner quote and delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "\"\"")
        })
        .expect("vi\" should select inner quote contents; d deletes them");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "\"\"\n");
}

#[test]
fn test_visual_select_around_double_quote() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"x\"hi\"y").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "x\"hi\"y")
        })
        .expect("wait for initial render");

    // Move to 'h' (index 2), enter visual mode, select around quote, then delete.
    session
        .send_text("llva\"d")
        .expect("select around quote and delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "xy")
        })
        .expect("va\" should select around quote contents; d deletes them");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "xy\n");
}

#[test]
fn test_visual_select_inner_paren_alias_b() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"(hello)").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "(hello)")
        })
        .expect("wait for initial render");

    // Move to 'h' (index 1), enter visual mode, select inner paren via alias b, then delete.
    session
        .send_text("lvibd")
        .expect("select inner paren via b alias and delete");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "()")
        })
        .expect("vib should select inner paren; d deletes them");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "()\n");
}

#[test]
fn test_quote_text_object_does_not_cross_line_boundary() {
    let file = TempFile::new().expect("create temp file");
    // Two lines each with their own string; cursor on closing quote of first line.
    // Parity of quotes before the closing `"` is odd (opener at index 4),
    // so the algorithm finds the enclosing pair (4, 9) and does not reach line 2.
    file.write_all(b"var(\"key\");\nprintln!(\"Hello!\");")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "var(\"key\");")
        })
        .expect("wait for initial render");

    // Move cursor to the closing `"` of `"key"` (column 8, 0-indexed).
    session
        .send_text("lllllllldi\"")
        .expect("move to closing quote and delete inner");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "var(\"\");")
        })
        .expect("di\" on closing quote should delete only the same-line string contents");

    // Line 2 must remain unchanged.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(2, "println!(\"Hello!\");")
        })
        .expect("second line must not be modified");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_di_quote_ignores_unmatched_quote_in_comment() {
    // Regression: a lone `"` in a `//` comment on a previous line must not
    // shift parity and cause `di"` to target the wrong string.
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    file.write_all(b"// \"\nconst x: &str = \"hello\";")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "// \"")
        })
        .expect("wait for initial render");

    // Move to line 2 and place cursor inside "hello", then delete inner quote.
    // jllllllllllllllllllll = j + 21 'l' moves to the 'h' inside "hello".
    session
        .send_text("jlllllllllllllllllllldi\"")
        .expect("move to h in hello and delete inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(2, "const x: &str = \"\";")
        })
        .expect("di\" should delete only `hello`, not cross the comment line");

    // Line 1 must remain unchanged.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "// \"")
        })
        .expect("comment line must not be modified");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_di_quote_works_on_multiline_raw_string_first_line() {
    // Regression: di" on a cursor inside r#"..."# (first line of the string)
    // must delete the inner content spanning all lines of the raw string.
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    file.write_all(b"let s = r#\"hello,\n    world\"#;")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "let s = r#\"hello,")
        })
        .expect("wait for initial render");

    // "let s = r#" is 10 chars (cols 0-9), " is col 10, h is col 11.
    // Move right 11 times to land on 'h', then delete inner quote.
    session
        .send_text("llllllllllldi\"")
        .expect("move to h and delete inner");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "let s = r#\"\"#;")
        })
        .expect("di\" should delete all content inside the raw string");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_di_quote_works_on_multiline_raw_string_continuation_line() {
    // Regression: di" on a cursor inside r#"..."# (continuation line) must
    // also delete the inner content spanning all lines of the raw string.
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    file.write_all(b"let s = r#\"hello,\n    world\"#;")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "let s = r#\"hello,")
        })
        .expect("wait for initial render");

    // Move to line 2, then right 4 times to land on 'w' of "    world".
    session
        .send_text("jlllldi\"")
        .expect("move to w on line 2 and delete inner");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "let s = r#\"\"#;")
        })
        .expect("di\" from continuation line should delete all raw string content");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// Regression: `di"` should delete the inner markdown quoted text.
fn test_di_quote_works_in_markdown_file() {
    let file = TempFile::with_suffix(".md").expect("create temp markdown file");
    file.write_all(b"alpha \"hello\" omega").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha \"hello\" omega")
        })
        .expect("wait for initial render");

    // Move to the `h` inside the quoted text and remove the inner object.
    session
        .send_text("llllllldi\"")
        .expect("delete inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha \"\" omega")
        })
        .expect("di\" should delete the markdown quoted content");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha \"\" omega\n");
}

#[test]
/// Regression: `vi"` should select and delete inner markdown quoted text.
fn test_vi_quote_works_in_markdown_file() {
    let file = TempFile::with_suffix(".md").expect("create temp markdown file");
    file.write_all(b"alpha \"hello\" omega").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha \"hello\" omega")
        })
        .expect("wait for initial render");

    // Move to the `h` inside the quoted text and enter Visual inner-quote.
    session
        .send_text("lllllllvi\"")
        .expect("select inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .expect("vi\" should enter visual mode in markdown");

    // Delete the selected quote interior and verify the buffer content.
    session.send_text("d").expect("delete visual selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha \"\" omega")
        })
        .expect("visual inner quote delete should remove markdown quoted content");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha \"\" omega\n");
}

#[test]
/// Regression: `di"` should delete the inner AsciiDoc quoted text.
fn test_di_quote_works_in_asciidoc_file() {
    let file = TempFile::with_suffix(".adoc").expect("create temp asciidoc file");
    file.write_all(b"alpha \"hello\" omega").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha \"hello\" omega")
        })
        .expect("wait for initial render");

    // Move to the `h` inside the quoted text and remove the inner object.
    session
        .send_text("llllllldi\"")
        .expect("delete inner quote");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "alpha \"\" omega")
        })
        .expect("di\" should delete the AsciiDoc quoted content");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "alpha \"\" omega\n");
}

/// Regression: Ctrl-w on a line that contains only spaces must delete only
/// those spaces and stop at the newline, not reach into the previous line.
#[test]
fn test_ctrl_w_on_spaces_only_line_stops_at_newline() {
    let file = TempFile::new().expect("create temp file");
    // Line 1 is "prev", line 2 is three spaces followed by a newline, line 3
    // is empty. The cursor will be placed at the end of the spaces on line 2.
    file.write_all(b"prev\n   \n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2")
        })
        .expect("wait for initial render on line 1");

    // Move to line 2 and enter Insert mode at the end with `j` then `A`.
    session
        .send_text("jA")
        .expect("move to line 2, append at end");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("2/2:4")
        })
        .expect("cursor at col 4 on line 2 in insert mode");

    // Ctrl-w must only delete the three spaces; the newline must stay intact.
    session.send_text("\u{17}").expect("ctrl-w");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("2/2:1")
        })
        .expect("cursor back to col 1 on line 2 after deleting spaces");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    // The newline between "prev" and line 2 must be preserved; line 2 is now empty.
    assert_eq!(saved, "prev\n\n");
}

/// Regression: Ctrl-w at column 0 in insert mode must delete the newline,
/// joining the current line with the previous one.
#[test]
fn test_ctrl_w_at_column_0_deletes_newline_joining_lines() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"prev\nnext").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2")
        })
        .expect("wait for initial render");

    // Move to line 2 and enter Insert mode at column 0 with `I`.
    session
        .send_text("jI")
        .expect("move to line 2, insert at start");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("2/2:1")
        })
        .expect("insert mode at col 1 on line 2");

    // Ctrl-w at col 0 must delete the preceding newline, joining lines.
    session.send_text("\u{17}").expect("ctrl-w");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("1/1")
        })
        .expect("lines joined: now on line 1 of 1");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "prevnext\n");
}

/// Ctrl-w on a line with a word preceded by spaces must delete only the word,
/// leaving the spaces intact.
#[test]
fn test_ctrl_w_deletes_word_not_preceding_spaces() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"   word").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "word")
        })
        .expect("wait for initial render");

    // `A` enters Insert mode after the last character.
    session.send_text("A").expect("append at end of line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .expect("insert mode");

    // Delete the word "word"; spaces must remain.
    session.send_text("\u{17}").expect("ctrl-w deletes word");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("1/1:4")
        })
        .expect("cursor at col 4 after deleting word, spaces preserved");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    assert_eq!(saved, "   \n");
}

/// Ctrl-w with the cursor in the middle of leading spaces must delete only the
/// spaces on the current line back to the line start, not cross the newline.
#[test]
fn test_ctrl_w_in_leading_spaces_stops_at_line_start() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"prev\n    word").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("1/2")
        })
        .expect("wait for initial render");

    // Go to line 2 and enter Insert mode between the spaces, at col 3 (0-based 2).
    session
        .send_text("jlli")
        .expect("line 2, right twice, insert before col 3");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("2/2:3")
        })
        .expect("insert mode at col 3 of line 2");

    // Ctrl-w from inside the spaces must stop at the line start, not go to "prev".
    session.send_text("\u{17}").expect("ctrl-w");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ") && s.status_line_contains("2/2:1")
        })
        .expect("cursor at col 1 on line 2, did not cross newline");

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("save and quit cleanly");

    let saved = fs::read_to_string(file.path()).expect("read saved file");
    // Two leading spaces were deleted (cursor was at col 3 = after 2 spaces);
    // "  word" remains on line 2.
    assert_eq!(saved, "prev\n  word\n");
}

#[test]
/// Regression: a `{` in a `//` line comment must not be treated as a real
/// brace delimiter by `di{`. The surrounding pair must be the real code braces.
fn test_di_brace_ignores_brace_in_line_comment() {
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    // Line 1: `// {`  — comment containing a lone opening brace
    // Line 2: `fn foo() { bar }`
    file.write_all(b"// {\nfn foo() { bar }")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "// {")
        })
        .expect("wait for initial render");

    // Move to line 2. `fn foo() { ` is 11 chars (cols 0-10), `b` is col 11.
    // j + 11 rights places cursor on `b` inside `{ bar }`.
    session
        .send_text("jllllllllllldi{")
        .expect("move to b in bar and delete inner brace");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(2, "fn foo() {}")
        })
        .expect("di{ must delete only `bar`, ignoring the comment brace");

    // Line 1 must remain unchanged.
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "// {")
        })
        .expect("comment line must not be modified");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `da{` with a `{` in a comment on the previous line must delete only the real
/// surrounding brace pair, not span from the comment brace.
fn test_da_brace_ignores_brace_in_line_comment() {
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    file.write_all(b"// {\nfn foo() { bar }")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "// {")
        })
        .expect("wait for initial render");

    // j + 11 rights = cursor on `b` of `bar`.
    session
        .send_text("jlllllllllllda{")
        .expect("move to b in bar and delete around brace");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(2, "fn foo()")
        })
        .expect("da{ must delete `{ bar }`, ignoring the comment brace");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "// {")
        })
        .expect("comment line must not be modified");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `di{` with a `{` inside a string literal on the same line must not confuse
/// the brace-matching algorithm; only real code braces are matched.
fn test_di_brace_ignores_brace_in_string_literal() {
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    // `let s = "{"; fn foo() { bar }`
    file.write_all(b"let s = \"{\"; fn foo() { bar }")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "let s = \"{\"; fn foo() { bar }")
        })
        .expect("wait for initial render");

    // `let s = "{"; fn foo() { ` is 23 chars (cols 0-22), `b` is col 23.
    session
        .send_text("llllllllllllllllllllllldi{")
        .expect("move to b in bar and delete inner brace");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "let s = \"{\"; fn foo() {}")
        })
        .expect("di{ must delete only `bar`, ignoring the string brace");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// A `{` in a `//` comment that appears inside the surrounding brace pair must
/// not break the depth count. The outer pair must still be found correctly.
fn test_di_brace_comment_between_braces_not_confused() {
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    // "{\n    // {\n    bar\n}"
    // The comment `{` is inside the outer `{ … }` but must not increase depth.
    file.write_all(b"{\n    // {\n    bar\n}")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "{")
        })
        .expect("wait for initial render");

    // Move to line 3 (`    bar`). `    ` is 4 chars; `b` is col 4.
    session
        .send_text("jjlllldi{")
        .expect("move to b in bar and delete inner brace");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "{}")
        })
        .expect("outer brace still present after di{");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
/// `di(` with a `(` in a `//` comment on a previous line must ignore it.
/// Verifies the fix applies to all delimiter types, not just braces.
fn test_di_paren_ignores_paren_in_line_comment() {
    let file = TempFile::with_suffix(".rs").expect("create temp rust file");
    // Line 1: `// (`
    // Line 2: `fn foo(x: i32) { bar(x) }`
    file.write_all(b"// (\nfn foo(x: i32) { bar(x) }")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "// (")
        })
        .expect("wait for initial render");

    // Move to line 2. `fn foo(x: i32) { bar(` is 21 chars (cols 0-20), `x` is col 21.
    session
        .send_text("jllllllllllllllllllllldi(")
        .expect("move to x inside bar(x) and delete inner paren");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(2, "fn foo(x: i32) { bar() }")
        })
        .expect("di( must delete only `x` inside bar(), ignoring the comment paren");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_trimmed_ends_with(1, "// (")
        })
        .expect("comment line must not be modified");

    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
