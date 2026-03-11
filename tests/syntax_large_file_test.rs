use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use test_utils::PtySession;

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Create one temporary Rust file path for a large-file integration test.
fn large_file_path() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("ordex_large_syntax_{suffix}.rs"))
}

/// Write a 50,000-line Rust fixture with a multiline comment near the tail.
fn write_large_rust_fixture(path: &Path) {
    let file = File::create(path).expect("create large fixture");
    let mut writer = BufWriter::new(file);

    // Stream the large fixture directly to disk so the test does not build one
    // oversized in-memory string before spawning the editor.
    for _ in 0..49_998 {
        writer
            .write_all(b"let value = 1;\n")
            .expect("write fixture body");
    }
    writer
        .write_all(b"/* open comment\nlet tail = 1;\n")
        .expect("write fixture tail");
    writer.flush().expect("flush large fixture");
}

#[test]
fn test_large_supported_file_opens_scrolls_and_relexes_near_tail() {
    let path = large_file_path();
    write_large_rust_fixture(&path);

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[path.to_str().expect("temp path utf8")],
        Default::default(),
    )
    .expect("spawn ordex");
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.status_line_contains("NORMAL |")
        })
        .expect("wait for initial render");

    session
        .send_text(":49999")
        .expect("go to tail comment line");
    session.send_enter().expect("execute go to line");
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.status_line_contains("49999:1") && snapshot.contains("open comment")
        })
        .expect("tail comment should be visible");

    session.read_available().expect("collect tail transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[38;5;2m"),
        "open multiline comment should render as a comment near the file tail"
    );

    session.clear_transcript();
    session
        .send_text("$a */")
        .expect("close multiline comment near the tail");
    session.exit_to_normal_mode(Duration::from_secs(3));
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.contains("let tail = 1;")
        })
        .expect("tail code should still be visible after edit");

    session.read_available().expect("collect edited transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[38;5;4m\u{1b}[1mlet"),
        "tail code should be re-highlighted as code after closing the comment"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(3))
        .expect("quit cleanly");
    let _ = fs::remove_file(path);
}
