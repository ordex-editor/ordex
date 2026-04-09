//! Shared spinner state for asynchronous terminal feedback.

use std::time::Instant;

/// Ordered braille frames used for lightweight busy indicators in the UI.
const BRAILLE_SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Stateful spinner used by asynchronous overlays and popups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Spinner {
    frame: usize,
}

impl Spinner {
    /// Create one spinner at its initial frame.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Return the current spinner glyph.
    pub(crate) fn current_frame(&self) -> char {
        BRAILLE_SPINNER_FRAMES[self.frame]
    }

    /// Advance to the next spinner frame and return the new glyph.
    pub(crate) fn next_frame(&mut self) -> char {
        self.frame = (self.frame + 1) % BRAILLE_SPINNER_FRAMES.len();
        self.current_frame()
    }

    /// Synchronize the spinner to elapsed time since `started_at`.
    ///
    /// Returns `true` when the spinner moved to a different visible frame, and
    /// `false` when the elapsed time still maps to the current frame.
    pub(crate) fn sync_to_elapsed(&mut self, started_at: Instant, interval_ms: u128) -> bool {
        let normalized = ((started_at.elapsed().as_millis() / interval_ms) as usize)
            % BRAILLE_SPINNER_FRAMES.len();
        if self.frame == normalized {
            return false;
        }
        self.frame = normalized;
        true
    }
}
