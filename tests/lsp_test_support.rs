use std::thread;
use std::time::{Duration, Instant};
use test_utils::PtySession;

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
            return;
        }
        // Retry the hover request until the workspace analysis is ready enough to
        // answer symbol lookups reliably for the shared fixture project.
        assert!(Instant::now() < deadline, "warmup hover should succeed");
        thread::sleep(Duration::from_millis(100));
    }
}
