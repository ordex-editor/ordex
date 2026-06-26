//! Background search-match counting for the message bar.

use crate::search::SearchQuery;
use crate::spinner::Spinner;
use crate::text_buffer::TextBuffer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

/// Stop counting after this many matches to bound scan time.
const SEARCH_COUNT_CAP: usize = 1_000_000;

/// Send one progress event per this many matches to throttle channel traffic.
const PROGRESS_BATCH_SIZE: usize = 256;

/// Maximum progress events drained per background poll tick.
const EVENTS_PER_POLL: usize = 4;

/// Spinner animation interval in milliseconds.
const SPINNER_INTERVAL_MS: u128 = 80;

/// Event sent from the background counting thread to the main thread.
enum SearchCountEvent {
    /// Running total and optional cursor-match position during scanning.
    Progress {
        total: usize,
        current_position: Option<usize>,
    },
    /// Final scan result with capped flag.
    Finished {
        total: usize,
        current_position: Option<usize>,
        capped: bool,
    },
}

/// Tracks background search-match counting state for the message bar.
///
/// `current_position` is 0-indexed internally; display adds 1.
pub(crate) struct SearchCountState {
    /// Running total matches found so far.
    total: usize,
    /// 0-based index of the last jumped match, if known.
    current_position: Option<usize>,
    /// Whether the match cap was reached.
    capped: bool,
    /// Whether the background scan is still running.
    scanning: bool,
    /// Whether count data exists and has not been invalidated.
    active: bool,
    /// Cancel flag shared with the background thread.
    cancel: Option<Arc<AtomicBool>>,
    /// Receiver for events from the background thread.
    receiver: Option<mpsc::Receiver<SearchCountEvent>>,
    /// Spinner for progress indication while scanning.
    spinner: Spinner,
    /// When the current scan started.
    started_at: Instant,
}

impl SearchCountState {
    /// Build empty search-count state.
    pub(crate) fn new() -> Self {
        Self {
            total: 0,
            current_position: None,
            capped: false,
            scanning: false,
            active: false,
            cancel: None,
            receiver: None,
            spinner: Spinner::new(),
            started_at: Instant::now(),
        }
    }

    /// Start a new background count for `query` over `buffer`.
    ///
    /// `cursor_char` is the cursor position; the worker determines which match
    /// the cursor sits on by scanning from buffer start.
    pub(crate) fn start_count(
        &mut self,
        query: SearchQuery,
        buffer: TextBuffer,
        cursor_char: usize,
    ) {
        // Cancel any running scan before starting a new one.
        self.cancel_running();

        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);

        self.cancel = Some(cancel);
        self.receiver = Some(receiver);
        self.total = 0;
        self.current_position = None;
        self.capped = false;
        self.scanning = true;
        self.active = true;
        self.spinner = Spinner::new();
        self.started_at = Instant::now();

        thread::spawn(move || {
            run_count_worker(query, buffer, cursor_char, sender, worker_cancel);
        });
    }

    /// Drain pending events from the background thread.
    pub(crate) fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };

        for _ in 0..EVENTS_PER_POLL {
            match receiver.try_recv() {
                Ok(SearchCountEvent::Progress {
                    total,
                    current_position,
                }) => {
                    self.total = total;
                    if current_position.is_some() {
                        self.current_position = current_position;
                    }
                }
                Ok(SearchCountEvent::Finished {
                    total,
                    current_position,
                    capped,
                }) => {
                    self.total = total;
                    if current_position.is_some() {
                        self.current_position = current_position;
                    }
                    self.capped = capped;
                    self.scanning = false;
                    self.receiver = None;
                    self.cancel = None;
                    break;
                }
                Err(_) => break,
            }
        }

        // Advance spinner after draining events so the next render shows progress.
        if self.scanning {
            self.spinner
                .sync_to_elapsed(self.started_at, SPINNER_INTERVAL_MS);
        }
    }

    /// Cancel any running scan and clear all count state.
    pub(crate) fn invalidate(&mut self) {
        self.cancel_running();
        self.total = 0;
        self.current_position = None;
        self.capped = false;
        self.scanning = false;
        self.active = false;
    }

    /// Return whether count data is available and has not been invalidated.
    ///
    /// Returns `true` when a count was started and not yet invalidated by a
    /// buffer edit, and `false` when no count data exists.
    pub(crate) fn is_valid(&self) -> bool {
        self.active
    }

    /// Advance the current position forward by `count`, wrapping at the total.
    pub(crate) fn advance_forward(&mut self, count: usize) {
        let Some(pos) = self.current_position else {
            return;
        };
        if self.total == 0 {
            return;
        }
        // 0-based wrapping: (pos + count) % total.
        self.current_position = Some((pos + count) % self.total);
    }

    /// Advance the current position backward by `count`, wrapping at the end.
    pub(crate) fn advance_backward(&mut self, count: usize) {
        let Some(pos) = self.current_position else {
            return;
        };
        if self.total == 0 {
            return;
        }
        // 0-based wrapping: subtract with modular arithmetic.
        let new_pos = if count > pos {
            self.total - ((count - pos) % self.total)
        } else {
            pos - count
        };
        // Ensure we don't produce total (should wrap to 0).
        self.current_position = Some(new_pos % self.total);
    }

    /// Format the count for display on the right side of the message bar.
    ///
    /// Returns `None` when there is no count to show (zero matches or inactive).
    pub(crate) fn format_message(&self) -> Option<String> {
        if !self.active || self.total == 0 {
            return None;
        }

        if self.scanning {
            let frame = self.spinner.current_frame();

            return if let Some(pos) = self.current_position {
                Some(format!("[{frame} {}/... @ {}]", self.total, pos + 1))
            } else {
                Some(format!("[{frame} {}/...]", self.total))
            };
        }

        if self.capped {
            return if let Some(pos) = self.current_position {
                Some(format!("[{}/{SEARCH_COUNT_CAP}+]", pos + 1))
            } else {
                Some(format!("[??/{SEARCH_COUNT_CAP}+]"))
            };
        }

        if let Some(pos) = self.current_position {
            Some(format!("[{}/{}]", pos + 1, self.total))
        } else {
            Some(format!("[??/{}]", self.total))
        }
    }

    /// Return whether the background thread has pending work.
    ///
    /// Returns `true` when a scan is running or events are queued, and
    /// `false` when no background polling is needed.
    pub(crate) fn should_background_poll(&self) -> bool {
        self.scanning
    }

    /// Cancel the running background thread, if any.
    fn cancel_running(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        self.cancel = None;
        self.receiver = None;
    }
}

/// Background worker that counts matches in `buffer` for `query`.
fn run_count_worker(
    query: SearchQuery,
    buffer: TextBuffer,
    cursor_char: usize,
    sender: mpsc::Sender<SearchCountEvent>,
    cancel: Arc<AtomicBool>,
) {
    let mut current_position: Option<usize> = None;

    let total = query.for_each_match(&buffer, |match_start, index| {
        // Track which match the cursor sits on (0-based).
        if match_start <= cursor_char {
            current_position = Some(index - 1);
        }

        // Send progress periodically.
        if index % PROGRESS_BATCH_SIZE == 0 {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            if sender
                .send(SearchCountEvent::Progress {
                    total: index,
                    current_position,
                })
                .is_err()
            {
                return false;
            }
        }

        // Stop at the cap.
        if index >= SEARCH_COUNT_CAP {
            return false;
        }

        true
    });

    let capped = total >= SEARCH_COUNT_CAP;
    let _ = sender.send(SearchCountEvent::Finished {
        total,
        current_position,
        capped,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Empty buffer should produce no count.
    fn test_empty_buffer_no_count() {
        let state = SearchCountState::new();
        assert_eq!(state.format_message(), None);
    }

    #[test]
    /// Inactive state should produce no formatted message.
    fn test_inactive_no_message() {
        let state = SearchCountState::new();
        assert_eq!(state.format_message(), None);
        assert!(!state.is_valid());
    }

    #[test]
    /// Completed count with single match should show [1/1].
    fn test_single_match() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 1;
        state.current_position = Some(0);
        state.scanning = false;

        assert_eq!(state.format_message(), Some("[1/1]".to_string()));
    }

    #[test]
    /// Completed count on first of five matches should show [1/5].
    fn test_multiple_matches_first() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(0);
        state.scanning = false;

        assert_eq!(state.format_message(), Some("[1/5]".to_string()));
    }

    #[test]
    /// Completed count on third of five matches should show [3/5].
    fn test_multiple_matches_third() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(2);
        state.scanning = false;

        assert_eq!(state.format_message(), Some("[3/5]".to_string()));
    }

    #[test]
    /// Completed count without known position should show [??/total].
    fn test_unknown_position() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 42;
        state.current_position = None;
        state.scanning = false;

        assert_eq!(state.format_message(), Some("[??/42]".to_string()));
    }

    #[test]
    /// advance_forward should wrap from last to first (0-based).
    fn test_advance_forward_wraps() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(4);

        state.advance_forward(1);
        assert_eq!(state.current_position, Some(0));
    }

    #[test]
    /// advance_backward should wrap from first to last (0-based).
    fn test_advance_backward_wraps() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(0);

        state.advance_backward(1);
        assert_eq!(state.current_position, Some(4));
    }

    #[test]
    /// advance_forward with count > 1 should advance multiple positions.
    fn test_advance_forward_multiple() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(1);

        state.advance_forward(2);
        assert_eq!(state.current_position, Some(3));
    }

    #[test]
    /// Capped count should show the cap marker.
    fn test_capped_display() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = SEARCH_COUNT_CAP;
        state.current_position = Some(2);
        state.capped = true;
        state.scanning = false;

        assert_eq!(
            state.format_message(),
            Some(format!("[3/{SEARCH_COUNT_CAP}+]"))
        );
    }

    #[test]
    /// Capped count without position should show [??/cap+].
    fn test_capped_unknown_position() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = SEARCH_COUNT_CAP;
        state.current_position = None;
        state.capped = true;
        state.scanning = false;

        assert_eq!(
            state.format_message(),
            Some(format!("[??/{SEARCH_COUNT_CAP}+]"))
        );
    }

    #[test]
    /// Invalidation should clear all state and deactivate.
    fn test_invalidation_clears_state() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 10;
        state.current_position = Some(2);
        state.scanning = false;

        state.invalidate();
        assert_eq!(state.format_message(), None);
        assert!(!state.is_valid());
        assert_eq!(state.total, 0);
        assert_eq!(state.current_position, None);
    }

    #[test]
    /// Scanning state should show spinner with partial total.
    fn test_scanning_shows_spinner() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 123;
        state.current_position = None;
        state.scanning = true;
        state.started_at = Instant::now();

        let msg = state
            .format_message()
            .expect("should show scanning message");
        assert!(msg.contains("123/..."));
        assert!(!msg.contains('@'));
    }

    #[test]
    /// Scanning state with known position should include the position (1-based display).
    fn test_scanning_with_position() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 123;
        state.current_position = Some(4);
        state.scanning = true;
        state.started_at = Instant::now();

        let msg = state
            .format_message()
            .expect("should show scanning message");
        assert!(msg.contains("123/..."));
        assert!(msg.contains("@ 5"));
    }

    #[test]
    /// advance_forward on zero total should be a no-op.
    fn test_advance_forward_zero_total() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 0;
        state.current_position = None;

        state.advance_forward(1);
        assert_eq!(state.current_position, None);
    }

    #[test]
    /// advance_backward on zero total should be a no-op.
    fn test_advance_backward_zero_total() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 0;
        state.current_position = None;

        state.advance_backward(1);
        assert_eq!(state.current_position, None);
    }

    #[test]
    /// advance_forward wrapping with count > total should still wrap correctly.
    fn test_advance_forward_large_count_wraps() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(2);

        state.advance_forward(7); // (2 + 7) % 5 = 4
        assert_eq!(state.current_position, Some(4));
    }

    #[test]
    /// advance_backward with count > pos should wrap correctly.
    fn test_advance_backward_large_count_wraps() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.current_position = Some(1);

        state.advance_backward(3); // 5 - ((3 - 1) % 5) = 5 - 2 = 3
        assert_eq!(state.current_position, Some(3));
    }

    #[test]
    /// Regression: position should track the last jumped match after repeat_search.
    ///
    /// After /foo Enter (jumps to 1st match), position = 0 (0-based).
    /// After n (jumps to 2nd match), position = 1.
    /// After n (jumps to 3rd match), position = 2.
    fn test_position_tracks_last_jumped_match() {
        let mut state = SearchCountState::new();
        state.active = true;
        state.total = 5;
        state.scanning = false;

        // Simulate: /foo Enter lands on first match
        state.current_position = Some(0);
        assert_eq!(state.format_message(), Some("[1/5]".to_string()));

        // Simulate: n jumps to second match
        state.advance_forward(1);
        assert_eq!(state.format_message(), Some("[2/5]".to_string()));

        // Simulate: n jumps to third match
        state.advance_forward(1);
        assert_eq!(state.format_message(), Some("[3/5]".to_string()));

        // Simulate: N jumps back to second match
        state.advance_backward(1);
        assert_eq!(state.format_message(), Some("[2/5]".to_string()));
    }
}
