use std::time::Duration;
use test_utils::{PTY_BACKSPACE, PtySession, PtySessionConfig, TempFile, TempTree};

/// Return the compiled Ordex binary path for PTY tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Open a new empty line below the first line and wait for Insert mode.
fn open_insert_line(session: &mut PtySession) {
    session.send_text("o").expect("open line below");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("INSERT ") && snapshot.status_line_contains("2:1")
        })
        .expect("wait for insert mode on new line");
}

/// Wait for the initial Normal-mode frame after spawning Ordex.
fn wait_for_initial_render(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");
}

#[test]
/// Confirm completion previews a selected candidate and restores the raw prefix on deselection.
fn test_completion_preview_and_no_selection_restore() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alphabet\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);

    open_insert_line(&mut session);
    session.send_text("a").expect("type completion prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("INSERT ") && snapshot.row_contains(4, "alphabet")
        })
        .expect("wait for completion popup");

    session
        .send_text("\u{e}")
        .expect("move completion selection down");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "alphabet")
        })
        .expect("wait for previewed completion");

    session
        .send_text("\u{10}")
        .expect("move selection back to none");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "a") && !snapshot.row_contains(2, "alphabet")
        })
        .expect("wait for original prefix restoration");

    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for normal mode");
}

#[test]
/// Confirm lowercase typing still matches buffer words while preserving source casing.
fn test_completion_matches_case_insensitively_and_preserves_casing() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"Message\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);

    open_insert_line(&mut session);
    session.send_text("me").expect("type lowercase prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(4, "Message")
        })
        .expect("wait for completion popup");

    session
        .send_text("\u{e}")
        .expect("select matching completion");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "Message")
        })
        .expect("wait for mixed-case preview");
}

#[test]
/// Confirm deleting the active prefix dismisses the completion popup immediately.
fn test_completion_dismisses_after_invalidating_backspace() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alphabet\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);

    open_insert_line(&mut session);
    session.send_text("a").expect("type prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(4, "alphabet")
        })
        .expect("wait for popup");

    session
        .send_text(PTY_BACKSPACE)
        .expect("delete typed prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("2:1") && !snapshot.row_contains(2, "a")
        })
        .expect("wait for invalidating backspace result");
}

#[test]
/// Confirm moving the cursor while the popup is open dismisses stale suggestions.
fn test_completion_dismisses_when_cursor_moves() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alphabet\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);

    open_insert_line(&mut session);
    session.send_text("a").expect("type prefix");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(4, "alphabet")
        })
        .expect("wait for popup");

    session.send_text("\u{1b}[D").expect("move cursor left");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("2:1") && !snapshot.row_contains(4, "alphabet")
        })
        .expect("wait for popup dismissal after cursor move");
}

#[test]
/// Confirm explicit `./` prefixes show asynchronous file-path completions without blocking insert mode.
fn test_file_path_completion_appears_for_explicit_dot_prefix() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("buffer.txt", "seed\n")
        .expect("write buffer");
    tree.write_file("src/lib.rs", "pub fn demo() {}\n")
        .expect("write source");
    tree.write_file("state/file.txt", "demo\n")
        .expect("write sibling directory");
    tree.write_file("seed.txt", "demo\n").expect("write file");

    // Run Ordex inside the fixture tree so `./` resolves against the working directory
    // for this saved buffer and the popup can enumerate local directories.
    let mut session = PtySession::spawn(
        ordex_bin(),
        &["buffer.txt"],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);
    open_insert_line(&mut session);
    session.send_text("./s").expect("type explicit path prefix");
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.contains("src/")
                && snapshot.contains("state/")
                && snapshot.contains("seed.txt")
                && snapshot.contains("directory")
                && snapshot.contains("file")
        })
        .expect("wait for file-path completion popup");

    session.send_text("\u{e}").expect("select first completion");
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(2, "./src") && !snapshot.row_contains(2, "./src/")
        })
        .expect("wait for selected path preview");
}

#[test]
/// Confirm explicit path requests merge file-path and buffer-word candidates in one popup.
fn test_file_path_completion_merges_with_buffer_words() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("buffer.txt", "stateful\n")
        .expect("write buffer");
    tree.write_file("state/file.txt", "demo\n")
        .expect("write directory");

    // The saved buffer contributes `stateful` through the buffer-word source while
    // the working directory contributes `state/` through the async file-path source.
    let mut session = PtySession::spawn(
        ordex_bin(),
        &["buffer.txt"],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    wait_for_initial_render(&mut session);
    open_insert_line(&mut session);
    session
        .send_text("./sta")
        .expect("type explicit path prefix");
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.contains("state/")
                && snapshot.contains("stateful")
                && snapshot.contains("directory")
        })
        .expect("wait for merged completion popup");
}
