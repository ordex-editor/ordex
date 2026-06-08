use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

mod config_test_support;

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_search_preview_scrolls_to_next_match_outside_viewport() {
    let file = TempFile::new().expect("create temp file");
    // Create a file with target at line 15 (outside initial viewport)
    let content = (1..=20)
        .map(|i| {
            if i == 15 {
                "target line".to_string()
            } else {
                format!("line {}", i)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    file.write_all(content.as_bytes()).expect("seed file");

    let config = PtySessionConfig {
        rows: 8, // Small viewport to ensure target is outside
        ..Default::default()
    };

    let mut session = PtySession::spawn(ordex_bin(), &[file.path().to_str().unwrap()], config)
        .expect("spawn ordex");

    // Wait for initial content - should show lines 1-5 (plus status rows)
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "line 1")
                && !s.contains("target")
        })
        .expect("initial content without target");

    session.send_text("/target").expect("enter search preview");

    // Should now show the target line somewhere in viewport
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/target")
                && s.contains("target line")
        })
        .expect("search preview should scroll to show target");

    session.send_escape().expect("cancel search");

    // Should return to normal mode
    config_test_support::wait_normal_mode(&mut session);

    // Verify viewport was restored by checking target is not visible
    let snapshot = session.snapshot();
    assert!(
        !snapshot.contains("target"),
        "target should not be visible after escape"
    );
    assert!(
        snapshot.row_trimmed_ends_with(1, "line 1"),
        "should show line 1 after restore"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_preview_no_scroll_when_match_in_viewport() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 1\ntarget here\nline 3\nline 4\nline 5\n")
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
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "target here")
        })
        .expect("initial content with target visible");

    session.send_text("/target").expect("enter search preview");

    // Should still show the same viewport since target is already visible
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/target")
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "target here")
        })
        .expect("search preview should not scroll when match is visible");

    assert!(snapshot.row_trimmed_ends_with(1, "line 1"));
    assert!(snapshot.row_trimmed_ends_with(2, "target here"));

    session.send_escape().expect("cancel search");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_preview_no_scroll_when_no_matches() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"line 1\nline 2\nline 3\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "line 1")
        })
        .expect("initial content");

    session.send_text("/missing").expect("enter search preview");

    // Should keep original viewport since no matches exist
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/missing")
                && s.row_trimmed_ends_with(1, "line 1")
        })
        .expect("search preview should not scroll when no matches");

    assert!(snapshot.row_trimmed_ends_with(1, "line 1"));

    session.send_escape().expect("cancel search");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_preview_wraps_to_beginning_when_no_match_after_cursor() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"target start\nline 2\nline 3\nline 4\ntarget end\n")
        .expect("seed file");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        Default::default(),
    )
    .expect("spawn ordex");

    // Move cursor to end of file
    session.send_text("G").expect("go to end");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(5, "target end")
        })
        .expect("cursor at end");

    session.send_text("/target").expect("enter search preview");

    // Should wrap to beginning and show first target
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/target")
                && s.contains("target start")
        })
        .expect("search preview should wrap to beginning");

    session.send_escape().expect("cancel search");
    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_preview_enter_keeps_scrolled_viewport() {
    let file = TempFile::new().expect("create temp file");
    let content = (1..=20)
        .map(|i| {
            if i == 15 {
                "target line".to_string()
            } else {
                format!("line {}", i)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    file.write_all(content.as_bytes()).expect("seed file");

    let config = PtySessionConfig {
        rows: 8,
        ..Default::default()
    };

    let mut session = PtySession::spawn(ordex_bin(), &[file.path().to_str().unwrap()], config)
        .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "line 1")
                && !s.contains("target")
        })
        .expect("initial content");

    session.send_text("/target").expect("enter search preview");

    // Should show target somewhere in viewport
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ") && s.contains("target line")
        })
        .expect("preview shows target");

    session.send_enter().expect("execute search");

    // Should stay on the scrolled viewport after executing search
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("target line")
                && s.status_line_contains("15/20:1") // cursor on target line
        })
        .expect("enter keeps scrolled viewport");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn test_search_preview_moves_cursor_for_relative_line_numbers() {
    use config_test_support::{open_session_with_config, write_config};

    let file = TempFile::new().expect("create temp file");
    let content = (1..=30)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    file.write_all(content.as_bytes()).expect("seed file");

    let config = write_config(
        r#"
[editor]
relative_line_numbers = true
"#,
    );

    let mut session = open_session_with_config(&file, &config);

    // Wait for initial content
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_trimmed_ends_with(1, "line 1")
        })
        .expect("initial content");
    session.send_text("G").expect("go to end");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.status_line_contains("30/30:")
        })
        .expect("cursor at end");

    // Enter search preview - should move cursor to first match
    session.send_text("/line 10").expect("enter search preview");

    // Cursor should be on line 10
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/line 10")
                && s.status_line_contains("10/30:")
        })
        .expect("search preview shows match and moves cursor");

    // Verify cursor position in status line
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("10/30:"))
        .expect("cursor should be on line 10");
    assert!(snapshot.status_line_contains("10/30:"));

    // Cancel search - should restore original cursor position
    session.send_escape().expect("cancel search");
    config_test_support::wait_normal_mode(&mut session);

    // Verify cursor restored to line 30
    let snapshot = session
        .wait_until(Duration::from_secs(2), |s| s.status_line_contains("30/30:"))
        .expect("cursor should be restored to line 30");
    assert!(snapshot.status_line_contains("30/30:"));

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
