use std::fs;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_soft_wrap_is_enabled_by_default() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnop").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 12, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "  1 abcdefgh") && s.row_contains(2, "    ijklmnop")
        })
        .expect("wrapped rows should render by default");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_j_and_k_move_by_wrapped_rows() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrst\nzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 40, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("initial cursor");

    session.send_text("j").expect("move to next wrapped row");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:37"))
        .expect("j should move within the wrapped line first");

    session
        .send_text("k")
        .expect("move back to previous wrapped row");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:1"))
        .expect("k should move back within the wrapped line");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_can_be_disabled_via_config() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnopqrstuvwxyz")
        .expect("seed file");

    let config = TempFile::new().expect("create temp config");
    config
        .write_all(
            br#"
[editor]
soft_wrap = false
 "#,
        )
        .expect("write config");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        PtySessionConfig {
            cols: 100,
            rows: 30,
        },
    )
    .expect("spawn ordex with config");
    session.resize(20, 8).expect("set terminal size");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "  1 abcdefghijklmnop") && s.row_contains(2, "  ~")
        })
        .expect("unwrapped long line should stay on one row");

    session
        .send_text("llllllllllllllllllll")
        .expect("move right repeatedly");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "klmnopqrstuvwxyz")
        })
        .expect("horizontal scrolling should remain available");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_handles_unicode_text() {
    let file = TempFile::new().expect("create temp file");
    file.write_all("éééééééééééééééééééééééééééééééééééééé".as_bytes())
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 40, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "  1 éééééééééééééééé") && s.row_contains(2, "    éé")
        })
        .expect("unicode text should wrap cleanly");

    session.send_text("j").expect("move to wrapped unicode row");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1:37"))
        .expect("wrapped unicode row should keep character indexing");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_wraps_while_in_insert_mode() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"").expect("seed file");
    let inserted = "abcdefghijklmnopqrstuvwxyzabcdefghijkl";

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 40, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .send_text(&format!("i{inserted}"))
        .expect("insert wrapped text");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "  1 abcdefghijklmnopqrstuvwxyzabcdefghij")
                && s.row_contains(2, "    kl")
                && s.status_line_contains("INSERT ")
                && s.status_line_contains("1:39")
        })
        .expect("insert mode should wrap text and keep the cursor state coherent");

    session.send_escape().expect("leave insert mode");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("return to normal mode");
    session.send_text(":q!").expect("quit without saving");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify wrapped rendering never spills into the reserved status row.
#[test]
fn test_soft_wrap_does_not_overwrite_status_bar() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(
        b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
    )
    .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 12, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "  1 abcdefgh")
                && s.row_contains(6, "    opqrstuv")
                && s.status_line_contains("NORMAL ")
        })
        .expect("wrapped content should stop before the status bar");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_soft_wrap_preserves_syntax_highlighting_across_wrapped_rows() {
    let file = std::env::temp_dir().join(format!("ordex_wrap_syntax_{}.rs", std::process::id()));
    fs::write(
        &file,
        b"fn wrap_test() { let message = \"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ\"; }\n",
    )
    .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.to_str().expect("utf8 temp path")],
        PtySessionConfig { cols: 28, rows: 8 },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.row_contains(1, "  1 fn wrap_test()")
                && snapshot.row_contains(2, "    sage = \"abcdefgh")
        })
        .expect("wrapped syntax fixture should be visible");

    session
        .read_available()
        .expect("collect wrapped transcript");
    let snapshot = session.snapshot();
    assert!(snapshot.row_contains(1, "  1 fn wrap_test()"));
    assert!(snapshot.row_contains(2, "    sage = \"abcdefgh"));
    assert!(
        snapshot.contains("\u{1b}[38;5;179m\u{1b}[1mfn"),
        "wrapped first row should retain keyword highlighting"
    );
    assert!(
        snapshot.contains("\u{1b}[38;5;79m"),
        "wrapped string literal should keep string highlighting across rows"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
    let _ = fs::remove_file(file);
}
