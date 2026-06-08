use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

#[test]
fn test_search_preview_scrolls_to_next_match_outside_viewport() {
    let file = TempFile::new().expect("create temp file");
    // Create a file with target at line 10 (outside initial viewport)
    let content = (1..=20)
        .map(|i| if i == 10 { "target line".to_string() } else { format!("line {}", i) })
        .collect::<Vec<_>>()
        .join("\n");
    file.write_all(content.as_bytes()).expect("seed file");

    let config = PtySessionConfig {
        rows: 8, // Small viewport to ensure target is outside
        ..Default::default()
    };

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        config,
    )
    .expect("spawn ordex");

    // Wait for initial content - should show lines 1-6 (plus status rows)
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "line 2")
                && !s.contains_text("target")
        })
        .expect("initial content without target");

    session.send_text("/target").expect("enter search preview");
    
    // Should now show the target line centered in viewport
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/target")
                && s.contains_text("target line")
                && s.row_trimmed_ends_with(3, "line 9") // target should be around center
                && s.row_trimmed_ends_with(4, "target line")
                && s.row_trimmed_ends_with(5, "line 11")
        })
        .expect("search preview should scroll to show target");

    session.send_escape().expect("cancel search");
    
    // Should return to original viewport
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "line 2")
                && !s.contains_text("target")
        })
        .expect("escape should restore original viewport");

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

    let original_viewport = session.snapshot();
    
    session.send_text("/target").expect("enter search preview");
    
    // Should still show the same viewport since target is already visible
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/target")
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "target here")
        })
        .expect("search preview should not scroll when match is visible");

    // Verify viewport didn't change
    assert_eq!(session.snapshot().rows(), original_viewport.rows());

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
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "line 1")
        })
        .expect("initial content");

    let original_viewport = session.snapshot();
    
    session.send_text("/missing").expect("enter search preview");
    
    // Should keep original viewport since no matches exist
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/missing")
                && s.row_trimmed_ends_with(1, "line 1")
                && s.row_trimmed_ends_with(2, "line 2")
        })
        .expect("search preview should not scroll when no matches");

    // Verify viewport didn't change
    assert_eq!(session.snapshot().rows(), original_viewport.rows());

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
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(4, "target end")
        })
        .expect("cursor at end");

    session.send_text("/target").expect("enter search preview");
    
    // Should wrap to beginning and show first target
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.message_line_contains("/target")
                && s.row_trimmed_ends_with(1, "target start")
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
        .map(|i| if i == 10 { "target line".to_string() } else { format!("line {}", i) })
        .collect::<Vec<_>>()
        .join("\n");
    file.write_all(content.as_bytes()).expect("seed file");

    let config = PtySessionConfig {
        rows: 8,
        ..Default::default()
    };

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().unwrap()],
        config,
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.row_trimmed_ends_with(1, "line 1")
                && !s.contains_text("target")
        })
        .expect("initial content");

    session.send_text("/target").expect("enter search preview");
    
    // Should show target centered
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("SEARCH ")
                && s.contains_text("target line")
        })
        .expect("preview shows target");

    session.send_enter().expect("execute search");
    
    // Should stay on the scrolled viewport after executing search
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains_text("target line")
                && s.status_line_contains("10/20:1") // cursor on target line
        })
        .expect("enter keeps scrolled viewport");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}