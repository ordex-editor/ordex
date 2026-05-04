use std::io;
use std::thread;
use std::time::{Duration, Instant};
use test_utils::{PtySession, overlay_footer_hidden};

/// Move to `helper_value()` and wait until one hover request succeeds.
pub fn warm_up_helper_value_hover(session: &mut PtySession) {
    // CI can start these PTY tests while the language server is still building
    // the initial workspace graph, so the warmup tolerates one cold-start pass.
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        session
            .send_text("/helper_value()")
            .expect("search for warmup symbol");
        session.send_enter().expect("confirm warmup search");
        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.status_line_contains("4:13")
            })
            .expect("cursor should land on the warmup helper_value call");
        session.send_text("K").expect("request warmup hover");
        if session
            .wait_until(Duration::from_secs(4), |screen| {
                screen.contains("Hover") && screen.contains("fn helper_value() -> i32")
            })
            .is_ok()
        {
            session.send_text("j").expect("dismiss warmup hover");
            session
                .wait_until(Duration::from_secs(2), |screen| {
                    screen.row_contains(5, "    let _ = local_value();")
                        && screen.status_line_contains("5:13")
                })
                .expect("warmup hover should dismiss before moving down");
            wait_for_lsp_progress_to_finish(session);
            return;
        }
        // Retry the hover request until the workspace analysis is ready enough to
        // answer symbol lookups reliably for the shared fixture project.
        assert!(Instant::now() < deadline, "warmup hover should succeed");
        thread::sleep(Duration::from_millis(100));
    }
}

/// Wait until one PTY condition stays true for a short stability window.
pub fn wait_until_stable<F>(
    session: &mut PtySession,
    timeout: Duration,
    stable_for: Duration,
    mut condition: F,
) -> io::Result<()>
where
    F: FnMut(&test_utils::ScreenSnapshot) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut first_match_at = None;
    let mut last_snapshot = session.snapshot();
    while Instant::now() < deadline {
        // Pull fresh PTY output before checking the next rendered frame.
        session.read_available()?;
        let snapshot = session.snapshot();
        last_snapshot = snapshot.clone();
        if condition(&snapshot) {
            let matched_at = *first_match_at.get_or_insert_with(Instant::now);
            // Only accept the state after it has remained visible long enough to
            // outlive one transient redraw or in-flight async completion refresh.
            if Instant::now().saturating_duration_since(matched_at) >= stable_for {
                return Ok(());
            }
        } else {
            first_match_at = None;
        }
        thread::sleep(Duration::from_millis(10));
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "condition not stable before timeout; snapshot:\n{}",
            last_snapshot.raw()
        ),
    ))
}

/// Wait until the LSP progress overlay is no longer visible.
pub fn wait_for_lsp_progress_to_finish(session: &mut PtySession) {
    wait_until_stable(
        session,
        Duration::from_secs(10),
        Duration::from_millis(100),
        overlay_footer_hidden,
    )
    .expect("LSP progress overlay should clear");
}
