mod config_test_support;

use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, ScreenSnapshot, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

fn snapshot_contains(snapshot: &ScreenSnapshot, needle: &str) -> bool {
    let mut row = 1;
    while let Some(line) = snapshot.row(row) {
        if line.contains(needle) {
            return true;
        }
        row += 1;
    }
    false
}

/// Return the repository fixture path used for EOF rendering regressions.
fn eof_fixture_path(name: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    root.join("tests")
        .join("fixtures")
        .join("eof")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn test_line_numbers_render_with_eof_tildes() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 40,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "   1 alpha")
                && s.row_contains(2, "   2 beta")
                && s.row_contains(3, "   ~")
        })
        .expect("initial numbered frame");

    assert!(snapshot.row_contains(1, "   1 alpha"));
    assert!(snapshot.row_contains(2, "   2 beta"));
    assert!(snapshot.row_contains(3, "   ~"));

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_trailing_newline_fixture_does_not_render_a_phantom_line() {
    let fixture = eof_fixture_path("trailing_newline_main.rs");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[fixture.as_str()],
        PtySessionConfig {
            cols: 40,
            rows: 10,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_contains(1, "   1 fn main() {")
                && s.row_contains(2, "   2     println!(\"Hello, world!\");")
                && s.row_contains(3, "   3 }")
                && s.row_contains(4, "   ~")
        })
        .expect("initial numbered frame without a phantom line");

    assert!(snapshot.row_contains(1, "   1 fn main() {"));
    assert!(snapshot.row_contains(2, "   2     println!(\"Hello, world!\");"));
    assert!(snapshot.row_contains(3, "   3 }"));
    assert!(snapshot.row_contains(4, "   ~"));
    assert!(
        !snapshot.row_contains(4, "   4"),
        "a trailing newline must not create a visible fourth buffer line"
    );

    session.send_text("G").expect("jump to last line");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3:1") && s.row_contains(4, "   ~")
        })
        .expect("cursor stays on the third logical line");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_relative_line_numbers_render_from_config() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\ngamma\ndelta\n")
        .expect("seed file");

    let config = config_test_support::write_config(
        r#"
[editor]
relative_line_numbers = true
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("jj").expect("move to third line");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("3:1")
                && s.row_contains(1, "   2 alpha")
                && s.row_contains(2, "   1 beta")
                && s.row_contains(3, "   3 gamma")
                && s.row_contains(4, "   1 delta")
        })
        .expect("relative line numbers should render");

    assert!(snapshot.row_contains(1, "   2 alpha"));
    assert!(snapshot.row_contains(2, "   1 beta"));
    assert!(snapshot.row_contains(3, "   3 gamma"));
    assert!(snapshot.row_contains(4, "   1 delta"));

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_relative_line_numbers_refresh_after_large_insert_mode_bracketed_paste() {
    let file = TempFile::new().expect("create temp file");
    let config = config_test_support::write_config(
        r#"
[editor]
relative_line_numbers = true
"#,
    );
    let payload = (1..=100)
        .map(|line| format!("line{line:04}"))
        .collect::<Vec<_>>()
        .join("\n");
    let bracketed = format!("\u{1b}[200~{payload}\u{1b}[201~");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        PtySessionConfig {
            cols: 40,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex with config");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait normal mode");
    session.send_text("i").expect("enter insert mode");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("INSERT ")
        })
        .and_then(|_| {
            session
                .send_raw_bytes(bracketed.as_bytes())
                .expect("send bracketed paste");
            session.wait_until(Duration::from_secs(2), |s| {
                s.status_line_contains("INSERT ")
                    && s.status_line_contains("100:9")
                    && snapshot_contains(s, "100 line0100")
                    && snapshot_contains(s, "  1 line0099")
            })
        })
        .expect("relative numbers should refresh after insert paste");

    assert!(snapshot_contains(&snapshot, "100 line0100"));
    assert!(snapshot_contains(&snapshot, "  1 line0099"));

    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_relative_line_numbers_refresh_after_visual_bracketed_paste() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"x").expect("seed file");
    let config = config_test_support::write_config(
        r#"
[editor]
relative_line_numbers = true
"#,
    );
    let payload = (1..=100)
        .map(|line| format!("line{line:04}"))
        .collect::<Vec<_>>()
        .join("\n");
    let bracketed = format!("\u{1b}[200~{payload}\u{1b}[201~");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        PtySessionConfig {
            cols: 40,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex with config");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait normal mode");
    session.send_text("v").expect("enter visual mode");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("VISUAL ")
        })
        .and_then(|_| {
            session
                .send_raw_bytes(bracketed.as_bytes())
                .expect("send bracketed paste");
            session.wait_until(Duration::from_secs(2), |s| {
                s.status_line_contains("NORMAL ")
                    && s.status_line_contains("100:8")
                    && snapshot_contains(s, "100 line0100")
                    && snapshot_contains(s, "  1 line0099")
            })
        })
        .expect("relative numbers should refresh after visual paste");

    assert!(snapshot_contains(&snapshot, "100 line0100"));
    assert!(snapshot_contains(&snapshot, "  1 line0099"));

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_line_number_gutter_expands_for_four_digit_lines() {
    let file = TempFile::new().expect("create temp file");
    for i in 1..=1100 {
        file.writeln(&format!("line {}", i)).expect("append line");
    }

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 80,
            rows: 12,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "    1 line 1")
        })
        .expect("initial render");

    session.send_text(":1000").expect("goto line 1000");
    session.send_enter().expect("execute goto");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("1000:1"))
        .expect("goto line 1000");

    assert!(
        snapshot_contains(&snapshot, "1000 line 1000"),
        "expected a visible row to show expanded 4-digit line number"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_line_number_gutter_stays_pinned_during_horizontal_scroll() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"abcdefghijklmnopqrstuvwxyz\n")
        .expect("seed file");

    let config = config_test_support::write_config(
        r#"
[editor]
soft_wrap = false
"#,
    );

    // Start at the narrow width directly so the test does not race the initial
    // render against an immediate SIGWINCH-driven redraw in CI.
    let mut session = PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        PtySessionConfig {
            cols: 20,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex with config");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "   1 abcdefghijklmno")
        })
        .expect("initial render before horizontal scrolling");

    session
        .send_text("llllllllllllllllllll")
        .expect("move right repeatedly");

    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.row_contains(1, "   1 ") && s.row_contains(1, "lmnopqrstuvwxyz")
        })
        .expect("horizontal scroll applied");

    assert!(snapshot.row_contains(1, "   1 "));
    assert!(
        !snapshot.row_contains(1, "abcdefghijklmnop"),
        "content should be horizontally shifted while gutter stays fixed"
    );

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_narrow_terminal_keeps_gutter_and_stays_stable() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"hello\n").expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 temp path")],
        PtySessionConfig {
            cols: 2,
            rows: 8,
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("N"))
        .expect("initial render at narrow width");

    session.send_text("ll").expect("move cursor");
    session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("N"))
        .expect("editor remains responsive");

    session.send_text(":q").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
