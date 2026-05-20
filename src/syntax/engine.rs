//! Incremental syntax-highlighting engine.
//!
//! The engine keeps editor-owned derived state for the current document and
//! lexes lines with shared helpers driven by profile data.
//!
//! The cache design is inspired by Xi's syntax-highlighting write-ups: the
//! document keeps sparse restart checkpoints plus frontier markers, while the
//! renderer materializes only one bounded flat span window for the visible
//! region. Checkpoints remember resumable lexer state, frontiers mark where
//! downstream syntax may still be stale, and the span window holds the exact
//! spans that the TUI needs right now.

use crate::syntax::helpers::{
    LineCursor, consume_identifier, consume_number, identifier_can_start, number_can_start,
};
use crate::syntax::markup::lex_markup_line;
use crate::syntax::profile::*;
use crate::syntax::profiles::detect_language_details;
use crate::text_buffer::{TextBuffer, TextSlice};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

/// One styled region within a logical buffer line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightSpan {
    /// Inclusive start column.
    pub(crate) start_col: usize,
    /// Exclusive end column.
    pub(crate) end_col: usize,
    /// Semantic syntax class for this span.
    pub(crate) class: SyntaxClass,
    /// Optional semantic modifier layered on top of the class.
    pub(crate) modifier: Option<SyntaxModifier>,
}

impl HighlightSpan {
    /// Return whether this span covers `column`.
    pub(crate) fn covers(&self, column: usize) -> bool {
        (self.start_col..self.end_col).contains(&column)
    }

    /// Build one span from a shared semantic style.
    pub(crate) fn styled(start_col: usize, end_col: usize, style: SpanStyle) -> Self {
        Self {
            start_col,
            end_col,
            class: style.class,
            modifier: style.modifier,
        }
    }
}

/// Edit-range description passed from editor mutations into the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BufferEdit {
    /// First affected logical line.
    pub(crate) start_line: usize,
    /// Last affected logical line before the edit.
    pub(crate) old_end_line: usize,
    /// Last affected logical line after the edit.
    pub(crate) new_end_line: usize,
    /// Whether the edit changes the inherited lexer state for later lines.
    ///
    /// `true` means later sparse checkpoints and frontiers must be rebuilt from
    /// replay. `false` means the untouched suffix still inherits the same entry
    /// state, so its sparse metadata can be shifted and reused.
    pub(crate) may_change_later_line_state: bool,
}

/// How the active profile was detected, or that plain fallback was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetectionSource {
    /// Detection matched an exact filename.
    MatchByFilename,
    /// Detection matched a file extension.
    MatchByExtension,
    /// No profile matched and rendering fell back to plain text.
    PlainFallback,
}

/// Carry-over lexer state inherited from the previous line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LineLexMode {
    /// No multiline construct is currently open.
    #[default]
    Plain,
    /// A block comment continues from the previous line.
    BlockComment {
        /// Metadata for the active block comment.
        style: CommentStyle,
        /// Current block nesting depth.
        depth: usize,
    },
    /// A multiline string continues from the previous line.
    String {
        /// Metadata for the active string style.
        style: StringStyle,
        /// Captured dynamic state required to recognize the closing delimiter.
        state: StringContinuation,
    },
    /// A markup fenced block continues from the previous line.
    MarkupFence {
        /// Fence marker character, either `` ` `` or `~`.
        marker: char,
        /// Minimum fence length required to close the block.
        count: usize,
        /// Style applied to every line inside the delimited block.
        style: SpanStyle,
    },
}

/// Captured continuation state for multiline string families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StringContinuation {
    /// The string uses only static delimiters.
    #[default]
    Simple,
    /// A raw hash string carries the marker repetition count forward.
    Hash {
        /// Number of repeated markers captured from the opener.
        repetition: usize,
    },
    /// A C++ raw string carries a custom delimiter forward.
    CppRaw {
        /// Captured delimiter bytes.
        delimiter: [u8; 16],
        /// Number of delimiter bytes currently in use.
        len: usize,
    },
}

/// Per-line lex result returned by the generic lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineParseResult {
    /// Ordered, non-overlapping line-local spans.
    pub(crate) spans: Vec<HighlightSpan>,
    /// Exit mode inherited by the next logical line.
    pub(crate) exit_mode: LineLexMode,
}

/// Cached state for one logical line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineLexState {
    /// Entry mode inherited from the previous line.
    pub(crate) entry_mode: LineLexMode,
    /// Exit mode produced after lexing this line.
    pub(crate) exit_mode: LineLexMode,
    /// Syntax-generation number that produced this cached line state.
    ///
    /// The engine increments the document generation each time it opens a new
    /// document or applies an edit. Revisions on line states let tests and
    /// incremental relex logic distinguish cache entries produced before the
    /// current edit from ones refreshed during the current generation.
    pub(crate) revision: u64,
    /// Whether this line is stable for its current inherited entry mode.
    ///
    /// A stable line is one whose cached spans and exit mode already match what
    /// the lexer would produce if re-run with the same `entry_mode`. Once an
    /// incremental relex reaches a stable line after the edited region, later
    /// lines can keep their cached results because the carried multiline state
    /// will no longer change downstream.
    pub(crate) stable: bool,
}

/// Exact replay result for one logical line.
///
/// A replayed line is produced by restarting from the nearest syntax checkpoint
/// and re-running the lexer until this logical line is reached. Callers use it
/// when they need exact off-screen syntax state without mutating the prepared
/// visible span window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayedLine<'a> {
    /// Logical line index in the buffer.
    pub(crate) line_index: usize,
    /// Borrowed display text for this logical line with trailing line breaks removed.
    ///
    /// This stays as a trimmed rope-backed slice so callers can inspect replayed
    /// text without paying an allocation just to remove trailing line breaks.
    pub(crate) text: TextSlice<'a>,
    /// Entry lexer mode inherited from the previous line before replaying text.
    pub(crate) entry_mode: LineLexMode,
    /// Exit lexer mode produced after replaying this line.
    pub(crate) exit_mode: LineLexMode,
    /// Exact highlight spans produced for the replayed text.
    pub(crate) spans: Vec<HighlightSpan>,
}

impl Default for LineLexState {
    /// Build a plain, stable line state with no inherited multiline context.
    fn default() -> Self {
        Self {
            entry_mode: LineLexMode::Plain,
            exit_mode: LineLexMode::Plain,
            revision: 0,
            stable: true,
        }
    }
}

/// Sparse checkpoint used to restart incremental lexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    /// Cached lex state for the checkpoint line.
    ///
    /// Checkpoints are the sparse restart points in the Xi-inspired design:
    /// replay can resume from one of these lines instead of re-lexing from the
    /// top of the document every time the visible window moves.
    pub(crate) state: LineLexState,
}

/// Flat span cache for one bounded contiguous line window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SpanWindowCache {
    /// First logical line covered by `ranges`.
    pub(crate) start_line: usize,
    /// Per-line ranges into `spans`, indexed relative to `start_line`.
    pub(crate) ranges: Vec<Range<usize>>,
    /// Flat storage for every cached span inside the prepared window.
    pub(crate) spans: Vec<HighlightSpan>,
}

impl SpanWindowCache {
    /// Clear the prepared window and drop all cached spans.
    fn clear(&mut self) {
        self.start_line = 0;
        self.ranges.clear();
        self.spans.clear();
    }

    /// Return whether the prepared window includes the requested line.
    fn contains_line(&self, line_index: usize) -> bool {
        line_index >= self.start_line && line_index < self.start_line + self.ranges.len()
    }

    /// Return the cached spans for one line inside the prepared window.
    fn spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        if !self.contains_line(line_index) {
            return &[];
        }
        let range = &self.ranges[line_index - self.start_line];
        &self.spans[range.clone()]
    }

    /// Replace the window with a fresh flat span table.
    fn rebuild_flat(
        &mut self,
        start_line: usize,
        ranges: Vec<Range<usize>>,
        spans: Vec<HighlightSpan>,
    ) {
        self.clear();
        self.start_line = start_line;
        self.ranges = ranges;
        self.spans = spans;
    }
}

/// Highlight state for the current document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentHighlightState {
    /// Currently active profile, if any.
    pub(crate) active_profile: Option<LanguageId>,
    /// Source of the active detection result.
    pub(crate) detection_source: DetectionSource,
    /// Sparse restart points used to resume incremental lexing.
    pub(crate) checkpoints: BTreeMap<usize, Checkpoint>,
    /// Dirty lines whose downstream syntax may still need revalidation.
    ///
    /// Frontiers are the sparse "work remains from here" markers from the
    /// Xi-style model. The earliest frontier at or before a requested window
    /// tells the engine where carried multiline state might still diverge.
    pub(crate) frontier: BTreeSet<usize>,
    /// Flat spans for the currently prepared viewport window.
    ///
    /// This is the exact render cache: only visible/recent lines are
    /// materialized densely, while checkpoints/frontiers remain sparse.
    pub(crate) span_window: SpanWindowCache,
    /// Monotonic syntax-generation counter for the current document cache.
    ///
    /// Each document open or text edit advances this number. Cached line states
    /// record the generation that produced them so incremental tests can verify
    /// how far relexing propagated and the engine can reason about cache freshness.
    pub(crate) generation: u64,
    /// Whether the document has reached full lex correctness.
    pub(crate) fully_lexed: bool,
}

impl Default for DocumentHighlightState {
    /// Build an empty plain-text highlight state.
    fn default() -> Self {
        Self {
            active_profile: None,
            detection_source: DetectionSource::PlainFallback,
            checkpoints: BTreeMap::from([(
                0,
                Checkpoint {
                    state: LineLexState::default(),
                },
            )]),
            frontier: BTreeSet::new(),
            span_window: SpanWindowCache::default(),
            generation: 0,
            fully_lexed: true,
        }
    }
}

impl DocumentHighlightState {
    /// Return the cached spans for `line_index`, or an empty slice when missing.
    fn spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        self.span_window.spans_for_line(line_index)
    }

    /// Replace the sparse state with one plain-text baseline at line 0.
    fn reset_plain(&mut self) {
        self.checkpoints.clear();
        self.checkpoints.insert(
            0,
            Checkpoint {
                state: LineLexState::default(),
            },
        );
        self.frontier.clear();
        self.span_window.clear();
        self.fully_lexed = true;
    }

    /// Shift sparse line-indexed caches after one line-count delta.
    ///
    /// Stateful edits can change the carried lexer mode for every later line, so
    /// only strictly earlier sparse metadata stays safe in that case. Plain edits
    /// keep the old suffix valid after a line-number shift.
    fn shift_after_edit(&mut self, edit: BufferEdit) {
        if !edit.may_change_later_line_state {
            let old_count = edit
                .old_end_line
                .saturating_sub(edit.start_line)
                .saturating_add(1);
            let new_count = edit
                .new_end_line
                .saturating_sub(edit.start_line)
                .saturating_add(1);
            let delta = new_count as isize - old_count as isize;
            let shift_from = edit.old_end_line.saturating_add(1);

            // Plain edits keep the same carried entry mode for the untouched
            // suffix, so later checkpoints/frontiers can follow the line splice.
            shift_sparse_keys(&mut self.checkpoints, edit.start_line, shift_from, delta);
            shift_sparse_keys_set(&mut self.frontier, edit.start_line, shift_from, delta);
            self.checkpoints.entry(0).or_insert_with(|| Checkpoint {
                state: LineLexState::default(),
            });
            self.span_window.clear();
            return;
        }

        // Prefix checkpoints and dirty markers remain valid because the edit
        // cannot change their carried state. Later sparse entries must be
        // rebuilt from replay because even unchanged text can inherit a new
        // multiline state after the edit.
        self.checkpoints
            .retain(|line_index, _| *line_index < edit.start_line);
        self.frontier
            .retain(|line_index| *line_index < edit.start_line);
        self.checkpoints.entry(0).or_insert_with(|| Checkpoint {
            state: LineLexState::default(),
        });

        // Any overlapping prepared spans may now point at the wrong logical
        // lines, so drop the window and rebuild it lazily when rendering asks.
        self.span_window.clear();
    }

    /// Return the latest checkpoint at or before `line_index`.
    fn checkpoint_before_or_at(&self, line_index: usize) -> (usize, &Checkpoint) {
        self.checkpoints
            .range(..=line_index)
            .next_back()
            .or_else(|| self.checkpoints.first_key_value())
            .map(|(line, checkpoint)| (*line, checkpoint))
            .expect("line zero checkpoint should always exist")
    }

    /// Return the earliest dirty frontier line that can affect `end_line`.
    fn frontier_before_or_at(&self, end_line: usize) -> Option<usize> {
        self.frontier.range(..=end_line).next().copied()
    }

    /// Return whether the prepared window fully covers the requested line span.
    fn window_covers(&self, start_line: usize, end_line: usize) -> bool {
        self.span_window.contains_line(start_line) && self.span_window.contains_line(end_line)
    }

    /// Return the number of sparse checkpoints currently tracked.
    #[cfg(test)]
    pub(crate) fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// Return the cached state for one checkpoint line, if present.
    #[cfg(test)]
    pub(crate) fn checkpoint_state(&self, line_index: usize) -> Option<&LineLexState> {
        self.checkpoints
            .get(&line_index)
            .map(|checkpoint| &checkpoint.state)
    }

    /// Return the current prepared span-window size.
    #[cfg(test)]
    pub(crate) fn span_window_line_count(&self) -> usize {
        self.span_window.ranges.len()
    }
}

/// Stateful syntax-highlighting engine owned by `EditorState`.
#[derive(Debug, Clone, Default)]
pub(crate) struct SyntaxEngine {
    document: DocumentHighlightState,
}

/// Lex one line using the supplied profile.
pub(crate) fn lex_profile_line(
    profile: &LanguageProfile,
    line: &str,
    entry_mode: LineLexMode,
) -> LineParseResult {
    if let Some(markup_rules) = profile.markup_rules {
        lex_markup_line(profile, line, entry_mode, markup_rules)
    } else {
        lex_code_line(profile, line, entry_mode)
    }
}

impl SyntaxEngine {
    /// Create a fresh syntax engine with plain fallback state.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Advance the document generation and normalize cached revisions on rollover.
    fn advance_generation(&mut self) {
        if self.document.generation == u64::MAX {
            // A rollover would otherwise reuse `u64::MAX` forever because the old
            // saturating behavior could no longer distinguish refreshed lines from
            // stale cached ones. Reset cached revisions first, then reserve `1`
            // for the generation about to be produced by this open or edit.
            for checkpoint in self.document.checkpoints.values_mut() {
                checkpoint.state.revision = 0;
            }
            self.document.generation = 1;
            return;
        }
        self.document.generation += 1;
    }

    /// Open a document, detect its profile, and fully lex it top-to-bottom.
    pub(crate) fn open_document(&mut self, path: Option<&Path>, buffer: &TextBuffer) {
        self.advance_generation();
        self.document.reset_plain();
        match detect_language_details(path) {
            Some((profile, source)) => {
                self.document.active_profile = Some(profile.id);
                self.document.detection_source = source;
                self.document.frontier.insert(0);
                self.document.fully_lexed = false;
                self.prepare_visible_lines(buffer, 0, 0);
            }
            None => {
                self.document.active_profile = None;
                self.document.detection_source = DetectionSource::PlainFallback;
                self.clear_plain_state();
            }
        }
    }

    /// Apply one buffer edit and mark the affected suffix dirty for re-lexing.
    pub(crate) fn apply_edit(&mut self, edit: BufferEdit) {
        self.advance_generation();
        self.splice_line_caches(edit);
        match self.active_profile_definition() {
            Some(_) => {
                self.document.frontier.insert(edit.start_line);
                self.document.fully_lexed = false;
            }
            None => self.clear_plain_state(),
        }
    }

    /// Return the active language identifier, if any.
    #[cfg(test)]
    pub(crate) fn active_profile(&self) -> Option<LanguageId> {
        self.document.active_profile
    }

    /// Borrow ordered highlight spans for one line.
    pub(crate) fn spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        self.document.spans_for_line(line_index)
    }

    /// Return the active profile's block-comment metadata, if any.
    pub(crate) fn active_comment_styles(&self) -> &'static [CommentStyle] {
        self.active_profile_definition()
            .map_or(&[], |profile| profile.comment_styles)
    }

    /// Compute exact highlight spans for one line without mutating the prepared window.
    #[cfg(test)]
    pub(crate) fn compute_spans_for_line(
        &self,
        buffer: &TextBuffer,
        line_index: usize,
    ) -> Vec<HighlightSpan> {
        self.replay_exact_line(buffer, line_index)
            .map_or_else(Vec::new, |(_, parsed)| parsed.spans)
    }

    /// Return the exact exit mode produced by replaying one inclusive line range.
    pub(crate) fn exit_mode_for_range(
        &self,
        buffer: &TextBuffer,
        start_line: usize,
        end_line: usize,
    ) -> LineLexMode {
        let Some(profile) = self.active_profile_definition() else {
            return LineLexMode::Plain;
        };
        let line_count = buffer.lines_count().max(1);
        let start = start_line.min(line_count.saturating_sub(1));
        let end = end_line.min(line_count.saturating_sub(1));
        if start > end {
            return LineLexMode::Plain;
        }
        let mut entry_mode = if start == 0 {
            LineLexMode::Plain
        } else {
            self.exact_exit_mode_for_line(buffer, start - 1)
        };

        // Replay only the edited range so callers can compare the inherited
        // suffix state before and after one mutation without touching caches.
        for line_index in start..=end {
            let line = buffer
                .line_for_display_string(line_index)
                .expect("edited range line must exist");
            entry_mode = lex_profile_line(profile, &line, entry_mode).exit_mode;
        }
        entry_mode
    }

    /// Replay an exact line range without mutating the prepared visible window.
    pub(crate) fn replay_line_range<'a>(
        &self,
        buffer: &'a TextBuffer,
        start_line: usize,
        end_line: usize,
    ) -> Vec<ReplayedLine<'a>> {
        let line_count = buffer.lines_count().max(1);
        let start = start_line.min(line_count.saturating_sub(1));
        let end = end_line.min(line_count.saturating_sub(1));
        if start > end {
            return Vec::new();
        }

        let Some(profile) = self.active_profile_definition() else {
            // Plain-text fallback still needs stable line records so callers can
            // share one replay API regardless of whether syntax highlighting is active.
            return (start..=end)
                .map(|line_index| {
                    let text = buffer
                        .line_for_display(line_index)
                        .expect("plain replay line must exist");
                    ReplayedLine {
                        line_index,
                        text,
                        entry_mode: LineLexMode::Plain,
                        exit_mode: LineLexMode::Plain,
                        spans: Vec::new(),
                    }
                })
                .collect();
        };

        let (checkpoint_line, checkpoint) = self.document.checkpoint_before_or_at(start);
        let mut entry_mode = checkpoint.state.entry_mode;
        let mut replayed = Vec::with_capacity(end - start + 1);

        // Replay forward from the nearest checkpoint so callers get exact line
        // state without perturbing the shared visible-line cache.
        for line_index in checkpoint_line..=end {
            let text = buffer
                .line_for_display(line_index)
                .expect("replayed line must exist");
            let parse_text = buffer
                .line_for_display_string(line_index)
                .expect("replayed line must exist");
            let parsed = lex_profile_line(profile, &parse_text, entry_mode);
            let exit_mode = parsed.exit_mode;
            if line_index >= start {
                replayed.push(ReplayedLine {
                    line_index,
                    text,
                    entry_mode,
                    exit_mode,
                    spans: parsed.spans,
                });
            }
            entry_mode = exit_mode;
        }

        replayed
    }

    /// Prepare one visible line range so future span lookups are exact.
    pub(crate) fn prepare_visible_lines(
        &mut self,
        buffer: &TextBuffer,
        first_line: usize,
        last_line: usize,
    ) {
        if self.active_profile_definition().is_none() {
            self.document.span_window.clear();
            self.document.fully_lexed = true;
            return;
        }

        let line_count = buffer.lines_count().max(1);
        let start_line = first_line.min(line_count.saturating_sub(1));
        let end_line = last_line.min(line_count.saturating_sub(1));
        let margin = 16;
        let window_start = start_line.saturating_sub(margin);
        let window_end = (end_line + margin).min(line_count.saturating_sub(1));
        let dirty_before_window = self.document.frontier_before_or_at(window_end);
        if dirty_before_window.is_none() && self.document.window_covers(window_start, window_end) {
            return;
        }

        let profile = self
            .active_profile_definition()
            .expect("active profile should still exist while preparing spans");
        self.rebuild_window(buffer, profile, window_start, window_end);
    }

    /// Return the current syntax-generation number for the cached document state.
    pub(crate) fn generation(&self) -> u64 {
        self.document.generation
    }

    /// Return whether the current document state is fully lexed and stable.
    #[cfg(test)]
    pub(crate) fn is_fully_lexed(&self) -> bool {
        self.document.fully_lexed
    }

    /// Return a shared reference to the full document state.
    #[cfg(test)]
    pub(crate) fn document_state(&self) -> &DocumentHighlightState {
        &self.document
    }

    /// Compute the exact lexer state for one line for test assertions.
    #[cfg(test)]
    pub(crate) fn line_state_for_test(
        &mut self,
        buffer: &TextBuffer,
        line_index: usize,
    ) -> LineLexState {
        let target = line_index.min(buffer.lines_count().max(1).saturating_sub(1));
        let Some(profile) = self.active_profile_definition() else {
            return LineLexState {
                revision: self.document.generation,
                ..LineLexState::default()
            };
        };
        let (checkpoint_line, checkpoint) = self.document.checkpoint_before_or_at(target);
        let mut entry_mode = checkpoint.state.entry_mode;

        // Tests need the exact state for arbitrary lines even when that line is
        // not itself stored as a sparse checkpoint.
        for replay_line in checkpoint_line..=target {
            let line = buffer
                .line_for_display_string(replay_line)
                .expect("line state replay target must exist");
            let parsed = lex_profile_line(profile, &line, entry_mode);
            let state = LineLexState {
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision: self.document.generation,
                stable: true,
            };
            if replay_line == target {
                return state;
            }
            entry_mode = state.exit_mode;
        }

        LineLexState::default()
    }

    /// Replay one exact line from the nearest checkpoint without mutating caches.
    fn replay_exact_line(
        &self,
        buffer: &TextBuffer,
        line_index: usize,
    ) -> Option<(LineLexMode, LineParseResult)> {
        let profile = self.active_profile_definition()?;
        let target = line_index.min(buffer.lines_count().max(1).saturating_sub(1));
        let (checkpoint_line, checkpoint) = self.document.checkpoint_before_or_at(target);
        let mut entry_mode = checkpoint.state.entry_mode;

        // Replaying from the nearest checkpoint keeps this exact while leaving
        // the shared prepared span window untouched for later renders.
        for replay_line in checkpoint_line..=target {
            let line = buffer
                .line_for_display_string(replay_line)
                .expect("replayed line must exist while computing exact syntax state");
            let parsed = lex_profile_line(profile, &line, entry_mode);
            if replay_line == target {
                return Some((entry_mode, parsed));
            }
            entry_mode = parsed.exit_mode;
        }
        None
    }

    /// Return the exact exit mode for one logical line without mutating caches.
    fn exact_exit_mode_for_line(&self, buffer: &TextBuffer, line_index: usize) -> LineLexMode {
        self.replay_exact_line(buffer, line_index)
            .map_or(LineLexMode::Plain, |(_, parsed)| parsed.exit_mode)
    }

    /// Replace the document with plain fallback state.
    fn clear_plain_state(&mut self) {
        self.document.reset_plain();
    }

    /// Return the built-in definition for the active language id.
    fn active_profile_definition(&self) -> Option<&'static LanguageProfile> {
        let active_id = self.document.active_profile?;
        crate::syntax::profiles::builtin_profiles()
            .iter()
            .find(|profile| profile.id == active_id)
    }

    /// Rebuild the prepared span window from the sparse checkpoint/frontier cache.
    fn rebuild_window(
        &mut self,
        buffer: &TextBuffer,
        profile: &'static LanguageProfile,
        window_start: usize,
        window_end: usize,
    ) {
        let line_count = buffer.lines_count().max(1);
        let dirty_frontier = self.document.frontier_before_or_at(window_end);
        let replay_start = dirty_frontier
            .map(|frontier_line| frontier_line.min(window_start))
            .unwrap_or(window_start)
            .min(line_count.saturating_sub(1));
        let (checkpoint_line, checkpoint) = self.document.checkpoint_before_or_at(replay_start);
        let mut entry_mode = checkpoint.state.entry_mode;
        let revision = self.document.generation;
        let mut window_ranges = Vec::with_capacity(window_end - window_start + 1);
        let mut window_spans = Vec::new();
        let checkpoint_interval = 64;
        let mut stabilized_after_dirty = false;

        // Replay must begin at or before `window_start`; otherwise a dirty
        // frontier inside the window would shift later spans onto earlier
        // visible lines. The nearest checkpoint before that replay start gives
        // us the entry lexer mode to resume from.
        for line_index in checkpoint_line..line_count {
            let line = buffer
                .line_for_display_string(line_index)
                .expect("window rebuild line must exist");
            let parsed = lex_profile_line(profile, &line, entry_mode);
            let LineParseResult { spans, exit_mode } = parsed;
            let state = LineLexState {
                entry_mode,
                exit_mode,
                revision,
                stable: true,
            };
            if line_index >= window_start && line_index <= window_end {
                // Materialize visible spans directly into the flat render cache
                // buffers so window rebuilds avoid one transient allocation per
                // line in the prepared range.
                let start = window_spans.len();
                window_spans.extend(spans);
                let end = window_spans.len();
                window_ranges.push(start..end);
            }

            // Checkpoint insertions are intentionally sparse. The window start
            // pins the current viewport, and periodic restart points cap the
            // replay distance for later windows or edits.
            let matched_previous_checkpoint = dirty_frontier.is_some_and(|frontier_line| {
                line_index > frontier_line
                    && self
                        .document
                        .checkpoints
                        .get(&line_index)
                        .is_some_and(|checkpoint| {
                            checkpoint.state.entry_mode == state.entry_mode
                                && checkpoint.state.exit_mode == state.exit_mode
                        })
            });
            if line_index == 0
                // Keep line zero as the permanent root restart point.
                || line_index == window_start
                // Pin the window start so a nearby follow-up render can resume
                // close to the viewport even if no periodic checkpoint lands there.
                || (line_index >= checkpoint_line
                    // Periodic checkpoints cap worst-case replay distance when
                    // the viewport moves or an edit dirties a later suffix.
                    && (line_index - checkpoint_line).is_multiple_of(checkpoint_interval))
            {
                self.document.checkpoints.insert(
                    line_index,
                    Checkpoint {
                        state: state.clone(),
                    },
                );
            }

            // Once replay catches a previously cached state after the dirty
            // frontier, the remaining suffix can keep reusing older sparse
            // checkpoints because the carried multiline state has converged.
            if matched_previous_checkpoint {
                stabilized_after_dirty = true;
            }
            entry_mode = state.exit_mode;

            // Stop as soon as the visible window is exact and the downstream
            // suffix is known stable, or immediately after the requested window
            // for fully clean viewport motion.
            if stabilized_after_dirty && line_index >= window_end {
                break;
            }
            if line_index == window_end && self.document.frontier_before_or_at(window_end).is_none()
            {
                break;
            }
        }

        self.document
            .span_window
            .rebuild_flat(window_start, window_ranges, window_spans);
        if stabilized_after_dirty || window_end + 1 >= line_count {
            self.document.frontier.clear();
            self.document.fully_lexed = true;
        } else {
            self.document.frontier = BTreeSet::from([window_end + 1]);
            self.document.fully_lexed = false;
        }
    }

    /// Splice cached line metadata after one text edit.
    ///
    /// A splice inserts or removes lines in the middle, preserving the
    /// untouched prefix and suffix. Sparse checkpoints and frontiers are keyed
    /// by logical line number, so a splice only retargets those sparse keys
    /// and invalidates the prepared span window instead of repairing a dense
    /// whole-document span table.
    fn splice_line_caches(&mut self, edit: BufferEdit) {
        self.document.shift_after_edit(edit);
    }
}

/// Shift sparse map keys after one middle line splice.
fn shift_sparse_keys<T>(
    entries: &mut BTreeMap<usize, T>,
    remove_start: usize,
    shift_from: usize,
    delta: isize,
) {
    let mut shifted = BTreeMap::new();
    let original = std::mem::take(entries);

    // Sparse checkpoints are intentionally few, so rebuilding this map keeps
    // splice handling simple without bringing dense per-line metadata back.
    // Taking the original map first also drops stale entries unless they are
    // deliberately reinserted into the rebuilt map.
    for (line_index, value) in original {
        if (remove_start..shift_from).contains(&line_index) {
            continue;
        }
        let new_index = if line_index >= shift_from {
            // Entries in the untouched suffix move by the splice delta so they
            // keep pointing at the same logical content after the edit.
            line_index.saturating_add_signed(delta)
        } else {
            // Prefix entries stay at the same logical line numbers.
            line_index
        };
        shifted.insert(new_index, value);
    }
    *entries = shifted;
}

/// Shift sparse set keys after one middle line splice.
fn shift_sparse_keys_set(
    entries: &mut BTreeSet<usize>,
    remove_start: usize,
    shift_from: usize,
    delta: isize,
) {
    let mut shifted = BTreeSet::new();
    let original = std::mem::take(entries);

    // Frontier markers are sparse like checkpoints, so rebuilding this set
    // keeps splice work proportional to cached metadata instead of file size.
    // Taking the original set first also drops stale entries unless they are
    // deliberately reinserted into the rebuilt set.
    for line_index in original {
        if (remove_start..shift_from).contains(&line_index) {
            continue;
        }
        let new_index = if line_index >= shift_from {
            // Dirty markers after the splice follow the surviving suffix lines.
            line_index.saturating_add_signed(delta)
        } else {
            // Dirty markers before the splice still refer to the same prefix.
            line_index
        };
        shifted.insert(new_index);
    }
    *entries = shifted;
}

/// Captured opening metadata for one string literal.
#[derive(Debug, Clone)]
struct StringOpening<'a> {
    /// Captured continuation state carried by the opener.
    state: StringContinuation,
    /// Cursor position immediately after the opener.
    end: LineCursor<'a>,
}

/// Best string-style match found at one source position.
#[derive(Debug, Clone)]
struct StringMatch<'a> {
    /// String style selected for the opener.
    style: StringStyle,
    /// Opening metadata captured for that style.
    opening: StringOpening<'a>,
}

/// Lex one code-like line from the supplied entry mode.
fn lex_code_line(
    profile: &LanguageProfile,
    line: &str,
    entry_mode: LineLexMode,
) -> LineParseResult {
    let mut cursor = LineCursor::new(line);
    let mut spans = Vec::new();
    let mut exit_mode = LineLexMode::Plain;

    // Continued block comments and multiline strings must be handled before any
    // ordinary token detection so inherited state stays authoritative.
    match entry_mode {
        LineLexMode::BlockComment { style, depth } => {
            let start_col = cursor.col();
            let remaining_depth = consume_block_comment(profile, &mut cursor, style, depth, false);
            spans.push(HighlightSpan::styled(
                start_col,
                cursor.col(),
                style.span_style(),
            ));
            if remaining_depth > 0 {
                exit_mode = LineLexMode::BlockComment {
                    style,
                    depth: remaining_depth,
                };
                return LineParseResult { spans, exit_mode };
            }
        }
        LineLexMode::String { style, state } => {
            let start_col = cursor.col();
            let closed = consume_string_body(&mut cursor, style, state);
            spans.push(HighlightSpan::styled(start_col, cursor.col(), STRING_STYLE));
            if !closed {
                exit_mode = LineLexMode::String { style, state };
                return LineParseResult { spans, exit_mode };
            }
        }
        LineLexMode::Plain | LineLexMode::MarkupFence { .. } => {}
    }

    // After inherited state is cleared, scan the visible line left-to-right and
    // let the first matching token class claim each region.
    while !cursor.is_empty() {
        let start_col = cursor.col();

        if let Some(style) =
            match_comment_style(profile, cursor.remaining(), CommentStyleKind::Line)
        {
            cursor.advance_to_end();
            spans.push(HighlightSpan::styled(
                start_col,
                cursor.col(),
                style.span_style(),
            ));
            break;
        }
        if let Some(style) =
            match_comment_style(profile, cursor.remaining(), CommentStyleKind::Block)
        {
            let remaining_depth = consume_block_comment(profile, &mut cursor, style, 1, true);
            spans.push(HighlightSpan::styled(
                start_col,
                cursor.col(),
                style.span_style(),
            ));
            if remaining_depth > 0 {
                exit_mode = LineLexMode::BlockComment {
                    style,
                    depth: remaining_depth,
                };
                break;
            }
            continue;
        }
        if let Some(string_match) = match_string_style(profile, &cursor) {
            let mut end = string_match.opening.end;
            let closed =
                consume_string_body(&mut end, string_match.style, string_match.opening.state);
            spans.push(HighlightSpan::styled(start_col, end.col(), STRING_STYLE));
            cursor = end;
            if !closed {
                exit_mode = LineLexMode::String {
                    style: string_match.style,
                    state: string_match.opening.state,
                };
                break;
            }
            continue;
        }
        if number_can_start(&cursor, profile.number_pattern) {
            consume_number(&mut cursor, profile.number_pattern);
            spans.push(HighlightSpan::styled(start_col, cursor.col(), NUMBER_STYLE));
            continue;
        }
        if let Some(identifier) = profile.identifier
            && cursor
                .peek()
                .is_some_and(|ch| identifier_can_start(identifier, ch))
        {
            let token_prefix = cursor.prefix();
            let token_start = cursor.mark();
            consume_identifier(&mut cursor, identifier);
            if let Some(style) = identifier_style(
                profile,
                token_prefix,
                cursor.slice_since(token_start),
                cursor.remaining(),
            ) {
                spans.push(HighlightSpan::styled(start_col, cursor.col(), style));
            }
            continue;
        }
        if punctuation_matches(profile, &cursor) {
            spans.push(HighlightSpan::styled(
                start_col,
                start_col + 1,
                PUNCTUATION_STYLE,
            ));
        }
        cursor.advance_char();
    }

    LineParseResult { spans, exit_mode }
}

/// Return the longest matching comment opener of the requested kind.
fn match_comment_style(
    profile: &LanguageProfile,
    text: &str,
    kind: CommentStyleKind,
) -> Option<CommentStyle> {
    profile
        .comment_styles
        .iter()
        .filter(|style| style.kind == kind && text.starts_with(style.open))
        .max_by_key(|style| style.open.chars().count())
        .copied()
}

/// Return the longest matching nested block-comment opener for `style`.
fn nested_block_opener(
    profile: &LanguageProfile,
    text: &str,
    style: CommentStyle,
) -> Option<CommentStyle> {
    let Some(close) = style.close else {
        return None;
    };
    profile
        .comment_styles
        .iter()
        .filter(|candidate| {
            candidate.kind == CommentStyleKind::Block
                && candidate.nests
                && candidate.close == Some(close)
                && text.starts_with(candidate.open)
        })
        .max_by_key(|candidate| candidate.open.chars().count())
        .copied()
}

/// Consume one block comment.
///
/// # Parameters
/// - `profile`: Language profile that defines nested block-comment styles.
/// - `cursor`: Cursor positioned at the current block-comment scan location.
/// - `style`: Active block-comment style being consumed.
/// - `initial_depth`: Nesting depth already in effect at `start`.
/// - `initial_open_consumed`: Whether the opener at `start` was already counted.
///
/// Returns the remaining block-comment nesting depth after consuming as much of
/// the current line as possible. A return value of `0` means the comment closed.
fn consume_block_comment(
    profile: &LanguageProfile,
    cursor: &mut LineCursor<'_>,
    style: CommentStyle,
    initial_depth: usize,
    initial_open_consumed: bool,
) -> usize {
    let close = style
        .close
        .expect("block comment styles must define a closing delimiter");
    let mut depth = initial_depth;
    let mut at_initial_position = true;

    // When nesting is enabled, any opener that shares the same closing delimiter
    // increases the depth; otherwise only the closing delimiter matters.
    while !cursor.is_empty() {
        if style.nests
            && let Some(nested_style) = nested_block_opener(profile, cursor.remaining(), style)
        {
            if !(initial_open_consumed && at_initial_position) {
                depth += 1;
            }
            cursor.advance_if_starts_with(nested_style.open);
            at_initial_position = false;
            continue;
        }
        if cursor.advance_if_starts_with(close) {
            depth = depth.saturating_sub(1);
            at_initial_position = false;
            if depth == 0 {
                return 0;
            }
            continue;
        }
        cursor.advance_char();
        at_initial_position = false;
    }

    depth
}

/// Return the best matching string opener at the current cursor position.
///
/// # Parameters
/// - `profile`: Language profile that defines the candidate string styles.
/// - `cursor`: Cursor positioned at the potential opener.
fn match_string_style<'a>(
    profile: &LanguageProfile,
    cursor: &LineCursor<'a>,
) -> Option<StringMatch<'a>> {
    let mut best_match = None;
    let mut best_opening_len = 0usize;

    // Prefer the longest opener so triple quotes beat single quotes and raw
    // strings capture their marker count before shorter styles can match.
    for style in profile.string_styles.iter().copied() {
        let Some(opening) = string_opening(style, cursor) else {
            continue;
        };
        if !string_can_continue(style) {
            let mut probe = opening.end.clone();
            if !consume_string_body(&mut probe, style, opening.state) {
                continue;
            }
        }
        let opening_len = opening.end.col() - cursor.col();
        if opening_len > best_opening_len {
            best_match = Some(StringMatch { style, opening });
            best_opening_len = opening_len;
        }
    }

    best_match
}

/// Return opening metadata for one string style.
///
/// # Parameters
/// - `style`: Candidate string style to test.
/// - `cursor`: Cursor positioned at the opener.
fn string_opening<'a>(style: StringStyle, cursor: &LineCursor<'a>) -> Option<StringOpening<'a>> {
    match style.kind {
        StringStyleKind::Delimited { open, .. } => {
            let mut end = cursor.clone();
            // Fixed delimiters only need a direct prefix check to establish the opener.
            end.advance_if_starts_with(open).then_some(StringOpening {
                state: StringContinuation::Simple,
                end,
            })
        }
        StringStyleKind::PrefixedDelimited { prefixes, open, .. } => {
            let end = match_prefixed_opening(prefixes, open, cursor)?;
            Some(StringOpening {
                state: StringContinuation::Simple,
                end,
            })
        }
        StringStyleKind::HashDelimited {
            prefixes,
            marker,
            quote,
        } => {
            let mut best = None;
            let mut best_len = 0usize;
            for prefix in prefixes {
                let mut end = cursor.clone();
                if !end.advance_if_starts_with(prefix) {
                    continue;
                }
                // Prefer the longest prefix/hash run so overlapping raw-string
                // spellings keep the most specific opener.
                let mut repetition = 0usize;
                while end.peek() == Some(marker) {
                    end.advance_char();
                    repetition += 1;
                }
                if end.peek() != Some(quote) {
                    continue;
                }
                end.advance_char();
                let opening_len = end.col() - cursor.col();
                if opening_len > best_len {
                    best = Some(StringOpening {
                        state: StringContinuation::Hash { repetition },
                        end,
                    });
                    best_len = opening_len;
                }
            }
            best
        }
        StringStyleKind::CppRaw {
            prefixes,
            max_delimiter_len,
        } => {
            let mut best = None;
            let mut best_len = 0usize;
            for prefix in prefixes {
                let mut end = cursor.clone();
                if !end.advance_if_starts_with(prefix) || !end.advance_if_starts_with("R\"") {
                    continue;
                }

                // Capture the raw delimiter now so later lines can match the
                // exact `)delimiter"` closer without rescanning the opener.
                let mut delimiter = [0u8; 16];
                let mut len = 0usize;
                while let Some(ch) = end.peek() {
                    if ch == '(' {
                        end.advance_char();
                        let opening_len = end.col() - cursor.col();
                        if opening_len > best_len {
                            best = Some(StringOpening {
                                state: StringContinuation::CppRaw { delimiter, len },
                                end,
                            });
                            best_len = opening_len;
                        }
                        break;
                    }
                    if !is_valid_cpp_raw_delimiter_char(ch) || len >= max_delimiter_len {
                        break;
                    }
                    delimiter[len] = ch as u8;
                    len += 1;
                    end.advance_char();
                }
            }
            best
        }
    }
}

/// Consume one string body from the current cursor position.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the string body or continued-line body.
/// - `style`: Active string style being consumed.
/// - `state`: Captured continuation state required by dynamic string families.
///
/// Returns `true` when the current line reaches the string closer and `false`
/// when the string remains open for a later line.
fn consume_string_body(
    cursor: &mut LineCursor<'_>,
    style: StringStyle,
    state: StringContinuation,
) -> bool {
    match style.kind {
        StringStyleKind::Delimited { close, escape, .. }
        | StringStyleKind::PrefixedDelimited { close, escape, .. } => {
            consume_delimited_string_body(cursor, close, escape)
        }
        StringStyleKind::HashDelimited { marker, quote, .. } => {
            // Raw strings carry the captured repetition count forward so the same
            // closer can be recognized on later lines without rescanning the line.
            let StringContinuation::Hash { repetition } = state else {
                return false;
            };
            while !cursor.is_empty() {
                if consume_hash_close(cursor, quote, marker, repetition) {
                    return true;
                }
                cursor.advance_char();
            }
            false
        }
        StringStyleKind::CppRaw { .. } => {
            let StringContinuation::CppRaw { delimiter, len } = state else {
                return false;
            };
            while !cursor.is_empty() {
                if consume_cpp_raw_close(cursor, delimiter, len) {
                    return true;
                }
                cursor.advance_char();
            }
            false
        }
    }
}

/// Match the longest prefixed opening made from one prefix and one delimiter.
///
/// # Parameters
/// - `prefixes`: Prefix spellings that may appear before the delimiter.
/// - `open`: Delimiter text that must follow a matching prefix.
/// - `cursor`: Cursor positioned at the potential opening sequence.
fn match_prefixed_opening<'a>(
    prefixes: &'static [&'static str],
    open: &str,
    cursor: &LineCursor<'a>,
) -> Option<LineCursor<'a>> {
    let mut best = None;
    let mut best_len = 0usize;
    for prefix in prefixes {
        let mut end = cursor.clone();
        if !end.advance_if_starts_with(prefix) || !end.advance_if_starts_with(open) {
            continue;
        }
        let opening_len = end.col() - cursor.col();
        if opening_len > best_len {
            best = Some(end);
            best_len = opening_len;
        }
    }
    best
}

/// Consume one delimited string body with the supplied closing behavior.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the first character inside the string body.
/// - `close`: Closing delimiter that terminates the string.
/// - `escape`: Escape mode that determines how embedded delimiters are skipped.
fn consume_delimited_string_body(
    cursor: &mut LineCursor<'_>,
    close: &str,
    escape: EscapeMode,
) -> bool {
    let mut escaped = false;
    let repeated_quote = close
        .chars()
        .next()
        .filter(|_| close.chars().count() == 1 && escape == EscapeMode::RepeatQuote);

    // Fixed-delimiter strings reuse the same search helper for ordinary quoted,
    // prefixed, multiline, and repeated-quote forms in one pass.
    while !cursor.is_empty() {
        let ch = cursor
            .peek()
            .expect("non-empty cursor should expose one current character");
        if escape == EscapeMode::Backslash && !escaped && ch == '\\' {
            escaped = true;
            cursor.advance_char();
            continue;
        }
        if escaped {
            escaped = false;
            cursor.advance_char();
            continue;
        }
        if let Some(quote) = repeated_quote
            && cursor.peek() == Some(quote)
            && cursor.peek_second() == Some(quote)
        {
            cursor.advance_char();
            cursor.advance_char();
            continue;
        }
        if cursor.advance_if_starts_with(close) {
            return true;
        }
        cursor.advance_char();
    }
    false
}

/// Consume one raw-string closer when it is present at the current cursor position.
///
/// Returns `true` when the closer matches and advances `cursor` past it, or
/// `false` when the cursor stays at the original position.
fn consume_hash_close(
    cursor: &mut LineCursor<'_>,
    quote: char,
    marker: char,
    repeats: usize,
) -> bool {
    let mut lookahead = cursor.clone();
    if lookahead.peek() != Some(quote) {
        return false;
    }
    lookahead.advance_char();
    for _ in 0..repeats {
        if lookahead.peek() != Some(marker) {
            return false;
        }
        lookahead.advance_char();
    }
    *cursor = lookahead;
    true
}

/// Return whether `ch` is valid inside a C++ raw-string delimiter.
fn is_valid_cpp_raw_delimiter_char(ch: char) -> bool {
    ch.is_ascii() && !ch.is_ascii_whitespace() && !matches!(ch, '(' | ')' | '\\')
}

/// Consume one C++ raw-string closer when it is present at the current cursor position.
///
/// # Parameters
/// - `cursor`: Cursor positioned where a raw-string closer may begin.
/// - `delimiter`: Fixed delimiter bytes captured from the opening `R"delim(` sequence.
/// - `len`: Number of delimiter bytes that are valid in `delimiter`.
///
/// # Returns
/// - `true` when the closer matches and advances `cursor` past `)delimiter"`.
/// - `false` when no closer matches and `cursor` stays at the original position.
fn consume_cpp_raw_close(cursor: &mut LineCursor<'_>, delimiter: [u8; 16], len: usize) -> bool {
    let mut lookahead = cursor.clone();
    if lookahead.peek() != Some(')') {
        return false;
    }
    lookahead.advance_char();
    for expected in delimiter.iter().take(len) {
        if lookahead.peek() != Some(char::from(*expected)) {
            return false;
        }
        lookahead.advance_char();
    }
    if lookahead.peek() != Some('"') {
        return false;
    }
    lookahead.advance_char();
    *cursor = lookahead;
    true
}

/// Return whether `style` may continue onto a later line when left unclosed.
fn string_can_continue(style: StringStyle) -> bool {
    match style.kind {
        StringStyleKind::Delimited { multiline, .. }
        | StringStyleKind::PrefixedDelimited { multiline, .. } => multiline,
        StringStyleKind::HashDelimited { .. } | StringStyleKind::CppRaw { .. } => true,
    }
}

/// Return the first matching identifier style for `chars[start..end]`.
///
/// # Parameters
/// - `profile`: Language profile that provides identifier classification rules.
/// - `prefix`: Source slice immediately before the identifier token.
/// - `token`: Source slice for the identifier token.
/// - `rest`: Source slice immediately after the identifier token.
fn identifier_style(
    profile: &LanguageProfile,
    prefix: &str,
    token: &str,
    rest: &str,
) -> Option<SpanStyle> {
    profile
        .identifier_rules
        .iter()
        .find(|rule| identifier_rule_matches(**rule, prefix, token, rest))
        .map(|rule| rule.style)
}

/// Return whether one identifier rule matches the current token and context.
///
/// # Parameters
/// - `rule`: Identifier rule to evaluate.
/// - `prefix`: Source slice immediately before `token`.
/// - `token`: Already collected identifier text.
/// - `rest`: Source slice immediately after `token`.
fn identifier_rule_matches(rule: IdentifierRule, prefix: &str, token: &str, rest: &str) -> bool {
    let token_matches = match rule.match_kind {
        IdentifierMatch::Any => true,
        IdentifierMatch::ExactWords(words) => words.contains(&token),
        IdentifierMatch::ExactWordsIgnoreAsciiCase(words) => {
            words.iter().any(|word| word.eq_ignore_ascii_case(token))
        }
    };
    if !token_matches {
        return false;
    }

    // Context filters let the generic lexer classify constructs like TOML bare
    // keys without inventing language-specific token walkers.
    match rule.context {
        IdentifierContext::Anywhere => true,
        IdentifierContext::AfterChar {
            ch,
            allow_whitespace,
            require_line_start,
        } => prefix_matches_after_char(prefix, ch, allow_whitespace, require_line_start),
        IdentifierContext::BeforeChar {
            ch,
            allow_whitespace,
        } => {
            let rest = if allow_whitespace {
                rest.trim_start_matches(|c: char| c.is_whitespace())
            } else {
                rest
            };
            rest.starts_with(ch)
        }
    }
}

/// Return whether `prefix` ends with `ch` under the requested spacing rules.
fn prefix_matches_after_char(
    prefix: &str,
    ch: char,
    allow_whitespace: bool,
    require_line_start: bool,
) -> bool {
    let prefix = if allow_whitespace {
        prefix.trim_end_matches(|c: char| c.is_whitespace())
    } else {
        prefix
    };
    if !prefix.ends_with(ch) {
        return false;
    }

    // Some languages only treat `#directive` spellings as preprocessors when
    // the hash begins the logical line after optional indentation.
    if !require_line_start {
        return true;
    }
    let before_char = &prefix[..prefix.len() - ch.len_utf8()];
    before_char.chars().all(|c| c.is_whitespace())
}

/// Return whether the current character should be styled as punctuation.
fn punctuation_matches(profile: &LanguageProfile, cursor: &LineCursor<'_>) -> bool {
    let Some(ch) = cursor.peek() else {
        return false;
    };

    profile.punctuation_chars.contains(ch)
        && !(ch == '.'
            && cursor
                .prev()
                .is_some_and(|previous| previous.is_ascii_digit())
            && cursor
                .peek_second()
                .is_some_and(|following| following.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{BufferEdit, LineLexMode, StringContinuation, SyntaxEngine, lex_profile_line};
    use crate::syntax::profile::*;
    use crate::syntax::profiles::builtin_profiles;
    use crate::text_buffer::TextBuffer;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// Return one built-in profile by id.
    fn profile(language: LanguageId) -> &'static LanguageProfile {
        builtin_profiles()
            .iter()
            .find(|profile| profile.id == language)
            .expect("language profile should exist")
    }

    /// Verify that supported files are fully lexed on open.
    #[test]
    fn test_open_document_lexes_supported_file() {
        let buffer = TextBuffer::from_str("fn main() {\n    let x = 42;\n}\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.prepare_visible_lines(&buffer, 0, 0);
        assert_eq!(engine.active_profile(), Some(LanguageId::Rust));
        assert!(
            !engine.spans_for_line(0).is_empty(),
            "rust open should produce spans"
        );
    }

    /// Verify that unsupported files stay in plain fallback mode.
    #[test]
    fn test_open_document_falls_back_to_plain_text() {
        let buffer = TextBuffer::from_str("plain text only");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("notes.txt")), &buffer);
        assert_eq!(engine.active_profile(), None);
        assert!(engine.spans_for_line(0).is_empty());
    }

    /// Verify that forward relex stabilizes after a block comment closes.
    #[test]
    fn test_relex_stabilizes_after_multiline_comment_edit() {
        let mut buffer = TextBuffer::from_str("/* open\nstill open\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.prepare_visible_lines(&buffer, 1, 1);
        assert_eq!(
            engine.line_state_for_test(&buffer, 1).exit_mode,
            LineLexMode::BlockComment {
                style: nested_block_comment("/*", "*/"),
                depth: 1
            }
        );

        buffer.insert(buffer.chars_count(), "*/\n");
        engine.apply_edit(BufferEdit {
            start_line: 1,
            old_end_line: 1,
            new_end_line: 2,
            may_change_later_line_state: true,
        });
        engine.prepare_visible_lines(&buffer, 2, 2);

        assert_eq!(
            engine.line_state_for_test(&buffer, 2).exit_mode,
            LineLexMode::Plain
        );
    }

    /// Verify that inserting a newline only relexes through the first unchanged tail line.
    #[test]
    fn test_insert_newline_stops_before_relexing_distant_tail_lines() {
        let mut buffer =
            TextBuffer::from_str("let alpha = 1;\nlet beta = 2;\nlet gamma = 3;\nlet delta = 4;\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        let distant_revision = engine
            .document_state()
            .checkpoint_state(0)
            .expect("line zero checkpoint")
            .revision;

        // Split the first line so the edit introduces a newly inserted logical line.
        buffer.insert(4, "\n");
        engine.apply_edit(BufferEdit {
            start_line: 0,
            old_end_line: 0,
            new_end_line: 1,
            may_change_later_line_state: false,
        });
        assert_eq!(
            engine.document_state().frontier,
            std::collections::BTreeSet::from([0])
        );
        engine.prepare_visible_lines(&buffer, 0, 1);

        assert!(
            engine
                .document_state()
                .checkpoint_state(0)
                .expect("line zero checkpoint")
                .revision
                >= distant_revision
        );
        assert!(engine.is_fully_lexed());
    }

    /// Verify that removing a newline only relexes through the first unchanged tail line.
    #[test]
    fn test_remove_newline_stops_before_relexing_distant_tail_lines() {
        let mut buffer =
            TextBuffer::from_str("let alpha = 1;\nlet beta = 2;\nlet gamma = 3;\nlet delta = 4;\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        let distant_revision = engine
            .document_state()
            .checkpoint_state(0)
            .expect("line zero checkpoint")
            .revision;

        // Merge the first two lines so the edit removes one logical line.
        buffer.remove(14, 15);
        engine.apply_edit(BufferEdit {
            start_line: 0,
            old_end_line: 1,
            new_end_line: 0,
            may_change_later_line_state: false,
        });
        engine.prepare_visible_lines(&buffer, 0, 0);

        assert!(
            engine
                .document_state()
                .checkpoint_state(0)
                .expect("line zero checkpoint")
                .revision
                >= distant_revision
        );
        assert!(engine.is_fully_lexed());
    }

    /// Verify generation rollover still distinguishes refreshed and stale lines.
    #[test]
    fn test_generation_rollover_resets_cached_revisions_before_relex() {
        let mut buffer =
            TextBuffer::from_str("let alpha = 1;\nlet beta = 2;\nlet gamma = 3;\nlet delta = 4;\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.document.generation = u64::MAX;
        for checkpoint in engine.document.checkpoints.values_mut() {
            checkpoint.state.revision = u64::MAX;
        }

        // Split the first line so incremental relex stops at the first stable tail line.
        buffer.insert(4, "\n");
        engine.apply_edit(BufferEdit {
            start_line: 0,
            old_end_line: 0,
            new_end_line: 1,
            may_change_later_line_state: false,
        });
        assert!(!engine.is_fully_lexed());
        engine.prepare_visible_lines(&buffer, 0, 1);

        assert_eq!(engine.generation(), 1);
        assert_eq!(
            engine
                .document_state()
                .checkpoint_state(0)
                .expect("line zero checkpoint")
                .revision,
            1
        );
        assert!(engine.is_fully_lexed());
    }

    /// Build one repeated Rust source body for cache-reuse tests.
    fn repeated_rust_lines(line_count: usize) -> String {
        let mut source = String::new();
        for _ in 0..line_count {
            source.push_str("let value = 1;\n");
        }
        source
    }

    /// Verify suffix checkpoints survive one plain edit whose exit mode is unchanged.
    #[test]
    fn test_plain_edit_preserves_shifted_suffix_checkpoints() {
        let mut buffer = TextBuffer::from_str(&repeated_rust_lines(400));
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.prepare_visible_lines(&buffer, 160, 165);
        assert!(
            engine.document_state().checkpoint_state(128).is_some(),
            "far-window preparation should seed one later periodic checkpoint"
        );

        buffer.insert(4, "x");
        engine.apply_edit(BufferEdit {
            start_line: 0,
            old_end_line: 0,
            new_end_line: 0,
            may_change_later_line_state: false,
        });

        assert!(
            engine.document_state().checkpoint_state(128).is_some(),
            "plain edits should keep later suffix checkpoints available"
        );
    }

    /// Verify stateful edits still discard later suffix checkpoints.
    #[test]
    fn test_stateful_edit_invalidates_shifted_suffix_checkpoints() {
        let mut buffer = TextBuffer::from_str(&repeated_rust_lines(400));
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.prepare_visible_lines(&buffer, 160, 165);
        assert!(
            engine.document_state().checkpoint_state(128).is_some(),
            "far-window preparation should seed one later periodic checkpoint"
        );

        buffer.insert(0, "/*\n");
        engine.apply_edit(BufferEdit {
            start_line: 0,
            old_end_line: 0,
            new_end_line: 1,
            may_change_later_line_state: true,
        });

        assert!(
            engine.document_state().checkpoint_state(128).is_none(),
            "stateful edits should rebuild later suffix checkpoints from replay"
        );
    }

    /// Measure one far-window scroll sequence after editing one earlier line.
    fn far_window_scroll_after_edit(
        edit_line: usize,
        insert_col: usize,
        text: &str,
        stateful: bool,
    ) -> Duration {
        let mut buffer = TextBuffer::from_str(&repeated_rust_lines(5_000));
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.prepare_visible_lines(&buffer, 4_096, 4_102);
        let insert_at = buffer.line_to_char(edit_line) + insert_col;
        buffer.insert(insert_at, text);
        engine.apply_edit(BufferEdit {
            start_line: edit_line,
            old_end_line: edit_line,
            new_end_line: edit_line,
            may_change_later_line_state: stateful,
        });

        let started = Instant::now();
        for first_visible in (4_096..=4_144).step_by(4) {
            engine.prepare_visible_lines(&buffer, first_visible, first_visible + 6);
        }
        started.elapsed()
    }

    /// Verify non-stateful edits keep post-edit far-window scrolling faster than stateful edits.
    #[test]
    fn test_plain_edit_keeps_far_window_prepare_fast() {
        const NO_NOTICEABLE_FREEZE_LIMIT: Duration = Duration::from_millis(500);

        let safe_duration = (0..4)
            .map(|_| far_window_scroll_after_edit(2_048, 4, "x", false))
            .sum::<Duration>();
        let forced_invalidation_duration = (0..4)
            .map(|_| far_window_scroll_after_edit(2_048, 4, "x", true))
            .sum::<Duration>();

        assert!(
            safe_duration <= NO_NOTICEABLE_FREEZE_LIMIT,
            "plain-token edits should stay below the noticeable-freeze limit: safe={safe_duration:?}, limit={NO_NOTICEABLE_FREEZE_LIMIT:?}"
        );
        assert!(
            safe_duration < forced_invalidation_duration,
            "plain-token edits should scroll far windows faster when suffix checkpoints are reused: safe={safe_duration:?}, forced_invalidation={forced_invalidation_duration:?}"
        );
    }

    /// Verify that nested D block comments retain depth correctly.
    #[test]
    fn test_nested_d_comment_depth_is_preserved() {
        let buffer = TextBuffer::from_str("/+ outer\n/+ inner +/\nstill outer\n+/");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.d")), &buffer);
        engine.prepare_visible_lines(&buffer, 2, 2);
        assert_eq!(
            engine.line_state_for_test(&buffer, 1).exit_mode,
            LineLexMode::BlockComment {
                style: nested_block_comment("/+", "+/"),
                depth: 1
            }
        );
        assert_eq!(
            engine.line_state_for_test(&buffer, 2).exit_mode,
            LineLexMode::BlockComment {
                style: nested_block_comment("/+", "+/"),
                depth: 1
            }
        );
    }

    /// Verify that far multiline-comment windows rebuild with the carried comment state.
    #[test]
    fn test_prepare_visible_lines_keeps_far_multiline_comment_spans() {
        let mut source = String::from("/* open comment\n");

        // The regression only shows up after the prepared window moves far enough
        // to rebuild from sparse checkpoints rather than the initial viewport.
        for _ in 0..199 {
            source.push_str("comment body\n");
        }
        source.push_str("*/\nlet value = 1;\n");
        let buffer = TextBuffer::from_str(&source);
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        engine.prepare_visible_lines(&buffer, 80, 85);

        assert!(
            !engine.spans_for_line(84).is_empty(),
            "far comment lines should keep comment spans after window rebuild"
        );
    }

    /// Verify that sequential viewport-window shifts keep multiline comment spans alive.
    #[test]
    fn test_prepare_visible_lines_keeps_comment_spans_during_incremental_scroll() {
        let mut source = String::from("/* open comment\n");

        // Reproduce the integration test's long scroll so the sparse cache sees
        // the same sequence of overlapping window rebuilds as the editor.
        for _ in 0..199 {
            source.push_str("comment body\n");
        }
        source.push_str("*/\nlet value = 1;\n");
        let buffer = TextBuffer::from_str(&source);
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);

        for cursor_line in 6_usize..=47 {
            let first_visible = cursor_line.saturating_sub(3);
            let last_visible = first_visible + 5;
            engine.prepare_visible_lines(&buffer, first_visible, last_visible);
        }

        assert!(
            !engine.spans_for_line(46).is_empty(),
            "incremental scroll should keep comment spans on visible lines"
        );
    }

    /// Verify that Rust raw strings keep their captured delimiter count.
    #[test]
    fn test_rust_raw_string_uses_generic_string_state() {
        let parsed = lex_profile_line(
            profile(LanguageId::Rust),
            "let s = r###\"open",
            LineLexMode::Plain,
        );
        assert_eq!(
            parsed.exit_mode,
            LineLexMode::String {
                style: raw_hash_string(&["r", "br"], '#', '"'),
                state: StringContinuation::Hash { repetition: 3 }
            }
        );
    }

    /// Verify that TOML triple-quoted strings use shared multiline state.
    #[test]
    fn test_toml_multiline_string_uses_generic_string_state() {
        let parsed = lex_profile_line(
            profile(LanguageId::Toml),
            "value = \"\"\"",
            LineLexMode::Plain,
        );
        assert_eq!(
            parsed.exit_mode,
            LineLexMode::String {
                style: triple_double_quoted_string(),
                state: StringContinuation::Simple
            }
        );
    }

    /// Verify that range punctuation does not extend number highlighting into identifiers.
    #[test]
    fn test_rust_range_stops_number_before_identifier() {
        let line = "for _ in 0..content_height {";
        let parsed = lex_profile_line(profile(LanguageId::Rust), line, LineLexMode::Plain);
        let number_col = line.find('0').expect("find range start");
        let identifier_col = line
            .find("content_height")
            .expect("find range end identifier");

        assert!(
            parsed
                .spans
                .iter()
                .any(|span| span.class == SyntaxClass::Number && span.covers(number_col)),
            "the range start should still be highlighted as a number"
        );
        assert!(
            !parsed
                .spans
                .iter()
                .any(|span| span.class == SyntaxClass::Number && span.covers(identifier_col)),
            "the identifier after `..` should not be absorbed into the number span"
        );
    }
}
