use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Create one temporary Rust file for a large-file integration test.
fn large_file() -> TempFile {
    TempFile::with_suffix(".rs").expect("create large temporary rust file")
}

/// Write a 50,000-line Rust fixture with a multiline comment near the tail.
fn write_large_rust_fixture(path: &Path) {
    let file = File::create(path).expect("create large fixture");
    let mut writer = BufWriter::new(file);

    // Stream the large fixture directly to disk before spawning the editor.
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
    let file = large_file();
    write_large_rust_fixture(file.path());

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("temp path utf8")],
        Default::default(),
    )
    .expect("spawn ordex");
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");

    session
        .send_text(":49999")
        .expect("go to tail comment line");
    session.send_enter().expect("execute go to line");
    session
        .wait_until(Duration::from_secs(3), |snapshot| {
            snapshot.status_line_contains("49999/50000:1") && snapshot.contains("open comment")
        })
        .expect("tail comment should be visible");

    session.read_available().expect("collect tail transcript");
    let snapshot = session.snapshot();
    assert!(
        snapshot.contains("\u{1b}[38;5;249m"),
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
        snapshot.contains("\u{1b}[38;5;179m\u{1b}[1mlet"),
        "tail code should be re-highlighted as code after closing the comment"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(3))
        .expect("quit cleanly");
}
