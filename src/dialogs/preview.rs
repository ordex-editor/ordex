//! Shared picker-preview state and snapshot builders.

use super::picker::{PickerPreviewLine, PickerPreviewPopup};
use crate::spinner::Spinner;
use crate::syntax::{HighlightSpan, ReplayedLine, SyntaxEngine};
use crate::text_buffer::TextBuffer;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

const PREVIEW_MAX_LINES: usize = 40;
const PREVIEW_SPINNER_INTERVAL_MS: u128 = 100;

/// Initial viewport placement used when constructing one preview snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerPreviewFocus {
    /// Show the beginning of the file or buffer.
    Top,
    /// Center the preview around the requested zero-based line index.
    Center(usize),
}

/// Mutable preview state shared by preview-capable picker dialogs.
#[derive(Debug, Default)]
pub(crate) struct PickerPreviewState {
    /// Stable identifier for the currently previewed source, used to skip redundant reloads.
    current_key: Option<String>,
    /// Render-facing snapshot shown in the preview pane, or `None` when no preview is active.
    popup: Option<PickerPreviewPopup>,
    /// Background worker loading a disk-backed preview, if any.
    load: Option<PickerPreviewLoad>,
    /// Spinner advanced while a background load is in flight.
    spinner: Spinner,
}

/// One in-flight disk-backed preview load plus its cancellation handle.
#[derive(Debug)]
struct PickerPreviewLoad {
    receiver: Receiver<PickerPreviewEvent>,
    cancel: Arc<AtomicBool>,
    started_at: Instant,
}

/// Final preview event produced by the background loader.
#[derive(Debug)]
enum PickerPreviewEvent {
    Loaded(Result<PickerPreviewPopup, String>),
}

impl PickerPreviewState {
    /// Create one empty preview state with no active source.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Clear the current preview and stop any in-flight worker.
    pub(crate) fn clear(&mut self) {
        self.cancel_load();
        self.current_key = None;
        self.popup = None;
    }

    /// Store one already-built preview snapshot for an in-memory source.
    pub(crate) fn show_sync(&mut self, key: String, popup: PickerPreviewPopup) {
        // Reusing the same key means the selected source did not change, so the
        // existing preview can stay visible without restarting any work.
        if self.current_key.as_deref() == Some(key.as_str()) && self.load.is_none() {
            return;
        }
        self.cancel_load();
        self.current_key = Some(key);
        self.popup = Some(popup);
    }

    /// Replace the current preview with a blank popup that keeps the pane visible.
    pub(crate) fn show_empty(&mut self, key: String) {
        // Reusing the same key avoids redundant work when the empty state is
        // already on screen from a prior call.
        if self.current_key.as_deref() == Some(key.as_str()) && self.load.is_none() {
            return;
        }
        self.cancel_load();
        self.current_key = Some(key);
        self.popup = Some(PickerPreviewPopup::empty());
    }

    /// Start one background load for a disk-backed preview source.
    pub(crate) fn load_file(
        &mut self,
        key: String,
        path: PathBuf,
        display_path: String,
        focus: PickerPreviewFocus,
    ) {
        // Reusing the same key avoids thrashing worker threads while the picker
        // keeps the same selected row across unrelated UI updates.
        if self.current_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.cancel_load();
        self.current_key = Some(key);
        self.popup = Some(PickerPreviewPopup::loading(display_path.clone()));
        self.load = Some(PickerPreviewLoad::spawn(path, display_path, focus));
    }

    /// Drain any finished worker result and advance the loading spinner.
    ///
    /// Returns `true` when the visible preview state changed, and `false` when
    /// no new snapshot or spinner frame needs a redraw.
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(load) = &self.load {
            match load.receiver.try_recv() {
                Ok(PickerPreviewEvent::Loaded(result)) => {
                    self.popup = Some(match result {
                        Ok(popup) => popup,
                        Err(message) => PickerPreviewPopup::error(message),
                    });
                    self.load = None;
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.popup = Some(PickerPreviewPopup::error(
                        "Preview stopped unexpectedly".to_string(),
                    ));
                    self.load = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(load) = &self.load
            && self
                .spinner
                .sync_to_elapsed(load.started_at, PREVIEW_SPINNER_INTERVAL_MS)
        {
            changed = true;
        }
        changed
    }

    /// Return the render-facing popup snapshot for the current preview.
    pub(crate) fn popup(&self) -> Option<PickerPreviewPopup> {
        let mut popup = self.popup.clone()?;
        if self.load.is_some() {
            popup.status_message = Some(format!(
                "{} Loading preview...",
                self.spinner.current_frame()
            ));
        }
        Some(popup)
    }

    /// Return whether the preview still has background work in flight.
    pub(crate) fn is_loading(&self) -> bool {
        self.load.is_some()
    }

    /// Stop the current worker, if any, without clearing the visible popup.
    fn cancel_load(&mut self) {
        if let Some(load) = &self.load {
            load.cancel.store(true, Ordering::Relaxed);
        }
        self.load = None;
    }
}

impl PickerPreviewLoad {
    /// Spawn one worker that reads a file from disk and builds a preview snapshot.
    fn spawn(path: PathBuf, display_path: String, focus: PickerPreviewFocus) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let started_at = Instant::now();
        thread::spawn(move || {
            // Cancellation is checked before and after file IO so picker motion
            // can drop stale work quickly without surfacing outdated snapshots.
            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }
            let result = build_preview_popup_from_file(&path, &display_path, focus);
            if worker_cancel.load(Ordering::Relaxed) {
                return;
            }
            let _ = sender.send(PickerPreviewEvent::Loaded(
                result.map_err(|error| format!("Failed to load preview: {error}")),
            ));
        });
        Self {
            receiver,
            cancel,
            started_at,
        }
    }
}

/// Build one preview snapshot from already-loaded buffer state.
pub(crate) fn build_preview_popup(
    buffer: &TextBuffer,
    syntax: &SyntaxEngine,
    display_path: String,
    focus: PickerPreviewFocus,
) -> PickerPreviewPopup {
    let (start_line, end_line) = preview_line_range(buffer, focus);
    let replayed = syntax.replay_line_range(buffer, start_line, end_line);
    let lines = replayed
        .iter()
        .map(|line| build_preview_line(line, focus))
        .collect();
    PickerPreviewPopup::ready(display_path, lines)
}

/// Build one preview snapshot by loading a file from disk.
fn build_preview_popup_from_file(
    path: &Path,
    display_path: &str,
    focus: PickerPreviewFocus,
) -> std::io::Result<PickerPreviewPopup> {
    let file = File::open(path)?;
    let buffer = TextBuffer::from_reader(file)?;
    let mut syntax = SyntaxEngine::new();
    syntax.open_document(Some(path), &buffer);
    Ok(build_preview_popup(
        &buffer,
        &syntax,
        display_path.to_string(),
        focus,
    ))
}

/// Return the inclusive logical line range that should appear in the preview.
fn preview_line_range(buffer: &TextBuffer, focus: PickerPreviewFocus) -> (usize, usize) {
    let line_count = buffer.lines_count().max(1);
    let max_line = line_count.saturating_sub(1);
    let window_len = PREVIEW_MAX_LINES.min(line_count).max(1);
    match focus {
        PickerPreviewFocus::Top => (0, window_len.saturating_sub(1)),
        PickerPreviewFocus::Center(target_line) => {
            // Center the requested target line while keeping the returned range
            // within the real document bounds near the start or end of the file.
            let centered_start = target_line.saturating_sub(window_len / 2);
            let end_line = (centered_start + window_len.saturating_sub(1)).min(max_line);
            let start_line = end_line.saturating_add(1).saturating_sub(window_len);
            (start_line, end_line)
        }
    }
}

/// Convert one exact replay line into the render-facing preview line model.
fn build_preview_line(line: &ReplayedLine<'_>, focus: PickerPreviewFocus) -> PickerPreviewLine {
    // The preview keeps original line numbers so navigation-oriented pickers can
    // highlight the exact target line inside the surrounding context window.
    let highlighted =
        matches!(focus, PickerPreviewFocus::Center(target) if target == line.line_index);
    let text = line.text.to_string();
    PickerPreviewLine {
        line_number: line.line_index + 1,
        spans: merge_line_spans(&line.spans, text.chars().count()),
        text,
        highlighted,
    }
}

/// Normalize highlight spans so the rendered preview can style trailing text safely.
fn merge_line_spans(spans: &[HighlightSpan], line_len: usize) -> Vec<HighlightSpan> {
    // Preview rendering truncates lines horizontally, so preserving the exact
    // semantic ranges is enough as long as zero-width or empty spans are dropped.
    spans
        .iter()
        .filter(|span| span.start_col < span.end_col && span.start_col < line_len)
        .map(|span| HighlightSpan {
            start_col: span.start_col,
            end_col: span.end_col.min(line_len),
            class: span.class,
            modifier: span.modifier,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `show_empty()` sets the popup to an empty preview.
    #[test]
    fn test_show_empty_sets_empty_popup() {
        let mut state = PickerPreviewState::new();
        state.show_empty("test_key".to_string());
        let popup = state.popup().expect("popup should be Some");
        assert!(popup.is_empty());
    }

    /// Verify that calling `show_empty()` twice with the same key is idempotent.
    #[test]
    fn test_show_empty_is_idempotent_for_same_key() {
        let mut state = PickerPreviewState::new();
        state.show_empty("test_key".to_string());
        let first_popup = state.popup().expect("popup should be Some");
        state.show_empty("test_key".to_string());
        let second_popup = state.popup().expect("popup should still be Some");
        assert_eq!(first_popup, second_popup);
    }

    /// Verify that `show_empty()` replaces a previous sync preview.
    #[test]
    fn test_show_empty_replaces_sync_preview() {
        let mut state = PickerPreviewState::new();
        state.show_sync(
            "buffer:1".to_string(),
            PickerPreviewPopup::ready("test.rs".to_string(), Vec::new()),
        );
        assert!(!state.popup().expect("popup should be Some").is_empty());
        state.show_empty("none".to_string());
        let popup = state.popup().expect("popup should be Some");
        assert!(popup.is_empty());
    }
}
