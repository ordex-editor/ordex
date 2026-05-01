//! Shared PTY helpers for LSP footer visibility and startup-settle waits.

use crate::{PtySession, ScreenSnapshot};
use std::thread;
use std::time::Duration;

/// Configuration for one startup-analysis settle wait in PTY-backed LSP tests.
#[derive(Debug, Clone, Copy)]
pub struct StartupAnalysisWaitOptions {
    /// Whether the helper should first wait for one visible progress footer.
    pub wait_for_visible_progress: bool,
    /// Number of consecutive idle samples required before startup is considered settled.
    pub idle_samples: usize,
    /// Delay between consecutive idle samples.
    pub sample_gap: Duration,
    /// Timeout applied to each idle-sample wait.
    pub idle_timeout: Duration,
    /// Whether the idle state also requires the status line to be free of diagnostics.
    pub require_clear_diagnostics: bool,
}

impl Default for StartupAnalysisWaitOptions {
    /// Return the common startup-settle policy used by most LSP PTY tests.
    fn default() -> Self {
        Self {
            wait_for_visible_progress: false,
            idle_samples: 5,
            sample_gap: Duration::from_millis(200),
            idle_timeout: Duration::from_secs(12),
            require_clear_diagnostics: true,
        }
    }
}

/// Return whether the LSP progress footer is visible in the current screen.
pub fn overlay_footer_visible(screen: &ScreenSnapshot) -> bool {
    (24..=27).any(|row| screen.row_contains(row, "rust-analyzer"))
}

/// Return whether the LSP progress footer is absent from the current screen.
pub fn overlay_footer_hidden(screen: &ScreenSnapshot) -> bool {
    (24..=27).all(|row| !screen.row_contains(row, "rust-analyzer"))
}

/// Wait until startup analysis settles according to `options`.
pub fn wait_for_startup_analysis_to_settle(
    session: &mut PtySession,
    options: StartupAnalysisWaitOptions,
) {
    if options.wait_for_visible_progress {
        let _ = session.wait_until(Duration::from_secs(8), overlay_footer_visible);
    }
    // Some servers briefly hide the footer between startup phases, so require a
    // sustained streak of idle samples before treating startup as settled.
    for _ in 0..options.idle_samples {
        session
            .wait_until(options.idle_timeout, |screen| {
                overlay_footer_hidden(screen)
                    && (!options.require_clear_diagnostics || !screen.status_line_contains("● "))
            })
            .expect("startup analysis should settle");
        thread::sleep(options.sample_gap);
    }
}
