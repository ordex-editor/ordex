//! Incremental syntax-highlighting engine.
//!
//! The engine keeps editor-owned derived state for the current document and
//! lexes lines with shared helpers driven by profile data.

use crate::syntax::helpers::{
    find_delimited_close, find_hash_string_close, identifier_can_start, number_can_start,
    scan_identifier, scan_number, starts_with,
};
use crate::syntax::markup::lex_markup_line;
use crate::syntax::profile::*;
use crate::syntax::profiles::detect_language_details;
use crate::text_buffer::TextBuffer;
use std::cmp::Ordering;
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
        /// Repetition count captured by dynamic delimiters such as Rust raw strings.
        repetition: usize,
    },
    /// A markup fenced block continues from the previous line.
    MarkupFence {
        /// Fence marker character, either `` ` `` or `~`.
        marker: char,
        /// Minimum fence length required to close the block.
        count: usize,
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
    /// Source line index.
    pub(crate) line_index: usize,
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

impl Default for LineLexState {
    /// Build a plain, stable line state with no inherited multiline context.
    fn default() -> Self {
        Self {
            line_index: 0,
            entry_mode: LineLexMode::Plain,
            exit_mode: LineLexMode::Plain,
            revision: 0,
            stable: true,
        }
    }
}

/// Highlight state for the current document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentHighlightState {
    /// Currently active profile, if any.
    pub(crate) active_profile: Option<LanguageId>,
    /// Source of the active detection result.
    pub(crate) detection_source: DetectionSource,
    /// Cached per-line entry and exit modes.
    pub(crate) line_states: Vec<LineLexState>,
    /// Flat storage for all cached spans in line order.
    pub(crate) spans_by_line: Vec<HighlightSpan>,
    /// Per-line ranges into `spans_by_line`.
    pub(crate) span_ranges_by_line: Vec<Range<usize>>,
    /// First dirty line waiting for relexing, if any.
    pub(crate) dirty_start_line: Option<usize>,
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
            line_states: vec![LineLexState::default()],
            spans_by_line: Vec::new(),
            span_ranges_by_line: std::iter::once(0..0).collect(),
            dirty_start_line: None,
            generation: 0,
            fully_lexed: true,
        }
    }
}

impl DocumentHighlightState {
    /// Reset the span cache to `line_count` empty per-line entries.
    fn reset_span_cache(&mut self, line_count: usize) {
        self.spans_by_line.clear();
        self.span_ranges_by_line = vec![0..0; line_count];
    }

    /// Clear all cached spans while preserving no line ranges.
    fn clear_span_cache(&mut self) {
        self.spans_by_line.clear();
        self.span_ranges_by_line.clear();
    }

    /// Return the cached spans for `line_index`, or an empty slice when missing.
    fn spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        let Some(range) = self.span_ranges_by_line.get(line_index) else {
            return &[];
        };
        &self.spans_by_line[range.clone()]
    }

    /// Append one line's spans to the flat cache and record its range.
    fn push_line_spans(&mut self, spans: Vec<HighlightSpan>) {
        let start = self.spans_by_line.len();
        self.spans_by_line.extend(spans);
        let end = self.spans_by_line.len();
        self.span_ranges_by_line.push(start..end);
    }

    /// Ensure the per-line span-range table is long enough for `required_len`.
    fn ensure_span_range_len(&mut self, required_len: usize) {
        if self.span_ranges_by_line.len() >= required_len {
            return;
        }
        let anchor = self.spans_by_line.len();
        self.span_ranges_by_line
            .resize(required_len, anchor..anchor);
    }

    /// Insert `count` empty line ranges at `insert_at`.
    fn insert_empty_line_ranges(&mut self, insert_at: usize, count: usize) {
        let anchor = self
            .span_ranges_by_line
            .get(insert_at)
            .map(|range| range.start)
            .unwrap_or(self.spans_by_line.len());
        for _ in 0..count {
            self.span_ranges_by_line.insert(insert_at, anchor..anchor);
        }
    }

    /// Remove line ranges in `[remove_start, remove_end)` and their flat spans.
    ///
    /// The flat span buffer stores all lines back-to-back, so removing lines is
    /// a two-step operation: first remove the contiguous span slice owned by the
    /// deleted lines, then subtract that removed length from every later line
    /// range. Because line ranges stay ordered and non-overlapping, the spans for
    /// all removed lines are also one contiguous flat slice.
    fn remove_line_ranges(&mut self, remove_start: usize, remove_end: usize) {
        // Per-line ranges stay sorted in flat-buffer order, so the first removed
        // line start and last removed line end bracket the exact span slice that
        // must be deleted from `spans_by_line`.
        let remove_span_start = self.span_ranges_by_line[remove_start].start;
        let remove_span_end = self.span_ranges_by_line[remove_end - 1].end;
        let removed_span_count = remove_span_end.saturating_sub(remove_span_start);
        if removed_span_count > 0 {
            self.spans_by_line.drain(remove_span_start..remove_span_end);
        }

        // Drop the line-to-range entries next; after this drain, every later
        // line still points at its old flat-buffer offsets and must be shifted
        // left by the number of spans removed above.
        self.span_ranges_by_line.drain(remove_start..remove_end);
        for range in self.span_ranges_by_line.iter_mut().skip(remove_start) {
            range.start = range.start.saturating_sub(removed_span_count);
            range.end = range.end.saturating_sub(removed_span_count);
        }
    }

    /// Replace one line's cached spans and shift later line ranges as needed.
    ///
    /// Updating a single line may change how many spans it owns. The flat buffer
    /// splice swaps just that line's subrange in place, then later line ranges
    /// are shifted by the net span-count delta so they still point at the same
    /// logical lines as before. Earlier ranges remain valid because the edit is
    /// confined to the current line's contiguous segment.
    fn replace_line_spans(&mut self, line_index: usize, new_spans: Vec<HighlightSpan>) {
        // Capture the current flat subrange for this line before the splice so
        // we can measure the old span count and reuse the same starting offset.
        let old_range = self.span_ranges_by_line[line_index].clone();
        let old_len = old_range.end.saturating_sub(old_range.start);
        let new_len = new_spans.len();
        self.spans_by_line.splice(old_range.clone(), new_spans);

        // The edited line keeps the same starting position in the flat buffer;
        // only its exclusive end changes to reflect the replacement span count.
        self.span_ranges_by_line[line_index] = old_range.start..(old_range.start + new_len);

        match new_len.cmp(&old_len) {
            Ordering::Greater => {
                let added = new_len - old_len;
                // A longer replacement pushes every later line range to the
                // right by the net number of inserted spans.
                for range in self.span_ranges_by_line.iter_mut().skip(line_index + 1) {
                    range.start += added;
                    range.end += added;
                }
            }
            Ordering::Less => {
                let removed = old_len - new_len;
                // A shorter replacement pulls every later line range left by
                // the net number of removed spans.
                for range in self.span_ranges_by_line.iter_mut().skip(line_index + 1) {
                    range.start -= removed;
                    range.end -= removed;
                }
            }
            Ordering::Equal => {}
        }
    }

    /// Return the number of per-line span slots currently tracked.
    #[cfg(test)]
    pub(crate) fn span_line_count(&self) -> usize {
        self.span_ranges_by_line.len()
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
        lex_markup_line(line, entry_mode, markup_rules)
    } else {
        lex_code_line(profile, line, entry_mode)
    }
}

impl SyntaxEngine {
    /// Create a fresh syntax engine with plain fallback state.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Open a document, detect its profile, and fully lex it top-to-bottom.
    pub(crate) fn open_document(&mut self, path: Option<&Path>, buffer: &TextBuffer) {
        self.document.generation = self.document.generation.saturating_add(1);
        let line_count = buffer.lines_count().max(1);
        self.document.line_states = vec![LineLexState::default(); line_count];
        self.document.reset_span_cache(line_count);
        self.document.dirty_start_line = None;
        match detect_language_details(path) {
            Some((profile, source)) => {
                self.document.active_profile = Some(profile.id);
                self.document.detection_source = source;
                self.lex_all(buffer, profile);
            }
            None => {
                self.document.active_profile = None;
                self.document.detection_source = DetectionSource::PlainFallback;
                self.clear_plain_state(line_count);
            }
        }
    }

    /// Apply one buffer edit and synchronously re-lex until the state stabilizes.
    pub(crate) fn apply_edit(&mut self, buffer: &TextBuffer, edit: BufferEdit) {
        self.document.generation = self.document.generation.saturating_add(1);
        self.document.dirty_start_line = Some(edit.start_line);
        self.splice_line_caches(edit);
        match self.active_profile_definition() {
            Some(profile) => self.relex_from(buffer, profile, edit),
            None => self.clear_plain_state(buffer.lines_count().max(1)),
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

    /// Replace the document with plain fallback state sized to `line_count`.
    fn clear_plain_state(&mut self, line_count: usize) {
        self.document.line_states = (0..line_count)
            .map(|line_index| LineLexState {
                line_index,
                revision: self.document.generation,
                ..LineLexState::default()
            })
            .collect();
        self.document.reset_span_cache(line_count);
        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Return the built-in definition for the active language id.
    fn active_profile_definition(&self) -> Option<&'static LanguageProfile> {
        let active_id = self.document.active_profile?;
        crate::syntax::profiles::builtin_profiles()
            .iter()
            .find(|profile| profile.id == active_id)
    }

    /// Fully lex the current buffer from the first line to the last line.
    fn lex_all(&mut self, buffer: &TextBuffer, profile: &'static LanguageProfile) {
        let mut entry_mode = LineLexMode::Plain;
        let revision = self.document.generation;
        let line_count = buffer.lines_count().max(1);
        self.document.clear_span_cache();

        // Full-document lexing guarantees correct inherited state for multiline
        // constructs before the first frame is rendered.
        for line_index in 0..line_count {
            let line = buffer.line_for_display(line_index).unwrap_or_default();
            let parsed = lex_profile_line(profile, &line, entry_mode);
            self.document.line_states[line_index] = LineLexState {
                line_index,
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision,
                stable: true,
            };
            self.document.push_line_spans(parsed.spans);
            entry_mode = self.document.line_states[line_index].exit_mode;
        }

        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Re-lex from the first dirty line until the carried state stabilizes.
    fn relex_from(
        &mut self,
        buffer: &TextBuffer,
        profile: &'static LanguageProfile,
        edit: BufferEdit,
    ) {
        let line_count = buffer.lines_count().max(1);
        let start_line = edit.start_line.min(line_count.saturating_sub(1));
        let min_relex_line = edit.new_end_line.min(line_count.saturating_sub(1));
        let mut entry_mode = if start_line == 0 {
            LineLexMode::Plain
        } else {
            self.document.line_states[start_line - 1].exit_mode
        };
        let revision = self.document.generation;
        self.document.fully_lexed = false;

        // Continue until the edited region and any dependent multiline state have
        // both stabilized. Unchanged tail lines can keep their cached spans.
        for line_index in start_line..line_count {
            let line = buffer.line_for_display(line_index).unwrap_or_default();
            let previous_exit = self.document.line_states[line_index].exit_mode;
            let parsed = lex_profile_line(profile, &line, entry_mode);
            let unchanged = self.spans_for_line(line_index) == parsed.spans.as_slice()
                && previous_exit == parsed.exit_mode;

            self.document.line_states[line_index] = LineLexState {
                line_index,
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision,
                stable: true,
            };
            if !unchanged {
                self.document.replace_line_spans(line_index, parsed.spans);
            }
            entry_mode = self.document.line_states[line_index].exit_mode;

            if line_index >= min_relex_line && unchanged {
                break;
            }
        }

        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Splice cached line metadata after one text edit.
    ///
    /// The syntax engine stores per-line state plus a flat span buffer. A splice
    /// updates those caches so unchanged tail lines remain aligned with the
    /// buffer after inserted or removed line breaks, allowing incremental relex
    /// to resume from the first dirty line instead of rebuilding the whole file.
    fn splice_line_caches(&mut self, edit: BufferEdit) {
        let required_len = edit.old_end_line.saturating_add(1);
        if self.document.line_states.len() < required_len {
            self.document
                .line_states
                .resize(required_len, LineLexState::default());
        }
        self.document.ensure_span_range_len(required_len);
        let old_count = edit
            .old_end_line
            .saturating_sub(edit.start_line)
            .saturating_add(1);
        let new_count = edit
            .new_end_line
            .saturating_sub(edit.start_line)
            .saturating_add(1);
        match new_count.cmp(&old_count) {
            Ordering::Greater => {
                let diff = new_count - old_count;
                let insert_at = edit
                    .old_end_line
                    .saturating_add(1)
                    .min(self.document.line_states.len());
                for _ in 0..diff {
                    self.document
                        .line_states
                        .insert(insert_at, LineLexState::default());
                }
                self.document.insert_empty_line_ranges(insert_at, diff);
            }
            Ordering::Less => {
                let remove_start = edit.start_line.saturating_add(new_count);
                let remove_end = edit
                    .old_end_line
                    .saturating_add(1)
                    .min(self.document.line_states.len());
                if remove_start < remove_end {
                    self.document.line_states.drain(remove_start..remove_end);
                    self.document.remove_line_ranges(remove_start, remove_end);
                }
            }
            Ordering::Equal => {}
        }

        // Keep cached line-local metadata aligned after line insertions or
        // removals so unchanged tail states stay comparable without a full relex.
        for (line_index, state) in self.document.line_states.iter_mut().enumerate() {
            state.line_index = line_index;
        }
    }
}

/// Result of consuming one block-comment region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockCommentConsumeResult {
    /// Exclusive end column of the consumed region.
    end_col: usize,
    /// Remaining nesting depth after the scan stops.
    remaining_depth: usize,
}

/// Captured opening metadata for one string literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StringOpening {
    /// Marker repetition count captured from the opener.
    repetition: usize,
    /// Number of characters consumed by the opener.
    opening_len: usize,
}

/// Best string-style match found at one source position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StringMatch {
    /// String style selected for the opener.
    style: StringStyle,
    /// Opening metadata captured for that style.
    opening: StringOpening,
}

/// Result of consuming one string literal body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StringConsumeResult {
    /// Exclusive end column of the consumed region.
    end_col: usize,
    /// Whether the string closed on the current line.
    closed: bool,
}

/// Lex one code-like line from the supplied entry mode.
fn lex_code_line(
    profile: &LanguageProfile,
    line: &str,
    entry_mode: LineLexMode,
) -> LineParseResult {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut idx = 0;
    let mut exit_mode = LineLexMode::Plain;

    // Continued block comments and multiline strings must be handled before any
    // ordinary token detection so inherited state stays authoritative.
    match entry_mode {
        LineLexMode::BlockComment { style, depth } => {
            let comment = consume_block_comment(profile, &chars, 0, style, depth, false);
            spans.push(HighlightSpan::styled(
                0,
                comment.end_col,
                style.span_style(),
            ));
            idx = comment.end_col;
            if comment.remaining_depth > 0 {
                exit_mode = LineLexMode::BlockComment {
                    style,
                    depth: comment.remaining_depth,
                };
                return LineParseResult { spans, exit_mode };
            }
        }
        LineLexMode::String { style, repetition } => {
            let string = consume_string(&chars, 0, style, repetition, 0);
            spans.push(HighlightSpan::styled(0, string.end_col, STRING_STYLE));
            idx = string.end_col;
            if !string.closed {
                exit_mode = LineLexMode::String { style, repetition };
                return LineParseResult { spans, exit_mode };
            }
        }
        LineLexMode::Plain | LineLexMode::MarkupFence { .. } => {}
    }

    // After inherited state is cleared, scan the visible line left-to-right and
    // let the first matching token class claim each region.
    while idx < chars.len() {
        if let Some(style) = match_comment_style(profile, &chars, idx, CommentStyleKind::Line) {
            spans.push(HighlightSpan::styled(idx, chars.len(), style.span_style()));
            break;
        }
        if let Some(style) = match_comment_style(profile, &chars, idx, CommentStyleKind::Block) {
            let comment = consume_block_comment(profile, &chars, idx, style, 1, true);
            spans.push(HighlightSpan::styled(
                idx,
                comment.end_col,
                style.span_style(),
            ));
            if comment.remaining_depth > 0 {
                exit_mode = LineLexMode::BlockComment {
                    style,
                    depth: comment.remaining_depth,
                };
                break;
            }
            idx = comment.end_col;
            continue;
        }
        if let Some(string_match) = match_string_style(profile, &chars, idx) {
            let string = consume_string(
                &chars,
                idx,
                string_match.style,
                string_match.opening.repetition,
                string_match.opening.opening_len,
            );
            spans.push(HighlightSpan::styled(idx, string.end_col, STRING_STYLE));
            if !string.closed {
                exit_mode = LineLexMode::String {
                    style: string_match.style,
                    repetition: string_match.opening.repetition,
                };
                break;
            }
            idx = string.end_col;
            continue;
        }
        if number_can_start(&chars, idx, profile.number_pattern) {
            let end_idx = scan_number(&chars, idx);
            spans.push(HighlightSpan::styled(idx, end_idx, NUMBER_STYLE));
            idx = end_idx;
            continue;
        }
        if let Some(identifier) = profile.identifier
            && identifier_can_start(identifier, chars[idx])
        {
            let end_idx = scan_identifier(&chars, idx, identifier);
            if let Some(style) = identifier_style(profile, &chars, idx, end_idx) {
                spans.push(HighlightSpan::styled(idx, end_idx, style));
            }
            idx = end_idx;
            continue;
        }
        if punctuation_matches(profile, &chars, idx) {
            spans.push(HighlightSpan::styled(idx, idx + 1, PUNCTUATION_STYLE));
        }
        idx += 1;
    }

    LineParseResult { spans, exit_mode }
}

/// Return the longest matching comment opener of the requested kind.
fn match_comment_style(
    profile: &LanguageProfile,
    chars: &[char],
    idx: usize,
    kind: CommentStyleKind,
) -> Option<CommentStyle> {
    profile
        .comment_styles
        .iter()
        .filter(|style| style.kind == kind && starts_with(chars, idx, style.open))
        .max_by_key(|style| style.open.chars().count())
        .copied()
}

/// Return the longest matching nested block-comment opener for `style`.
fn nested_block_opener(
    profile: &LanguageProfile,
    chars: &[char],
    idx: usize,
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
                && starts_with(chars, idx, candidate.open)
        })
        .max_by_key(|candidate| candidate.open.chars().count())
        .copied()
}

/// Consume one block comment.
///
/// # Parameters
/// - `profile`: Language profile that defines nested block-comment styles.
/// - `chars`: Current line as a character slice.
/// - `start`: Column where scanning begins.
/// - `style`: Active block-comment style being consumed.
/// - `initial_depth`: Nesting depth already in effect at `start`.
/// - `initial_open_consumed`: Whether the opener at `start` was already counted.
fn consume_block_comment(
    profile: &LanguageProfile,
    chars: &[char],
    start: usize,
    style: CommentStyle,
    initial_depth: usize,
    initial_open_consumed: bool,
) -> BlockCommentConsumeResult {
    let close = style
        .close
        .expect("block comment styles must define a closing delimiter");
    let mut idx = start;
    let mut depth = initial_depth;

    // When nesting is enabled, any opener that shares the same closing delimiter
    // increases the depth; otherwise only the closing delimiter matters.
    while idx < chars.len() {
        if style.nests
            && let Some(nested_style) = nested_block_opener(profile, chars, idx, style)
        {
            if !(initial_open_consumed && idx == start) {
                depth += 1;
            }
            idx += nested_style.open.chars().count();
            continue;
        }
        if starts_with(chars, idx, close) {
            depth = depth.saturating_sub(1);
            idx += close.chars().count();
            if depth == 0 {
                return BlockCommentConsumeResult {
                    end_col: idx,
                    remaining_depth: 0,
                };
            }
            continue;
        }
        idx += 1;
    }

    BlockCommentConsumeResult {
        end_col: chars.len(),
        remaining_depth: depth,
    }
}

/// Return the best matching string opener at `idx`.
///
/// # Parameters
/// - `profile`: Language profile that defines the candidate string styles.
/// - `chars`: Current line as a character slice.
/// - `idx`: Column where the potential opener begins.
fn match_string_style(
    profile: &LanguageProfile,
    chars: &[char],
    idx: usize,
) -> Option<StringMatch> {
    let mut best_match = None;
    let mut best_opening_len = 0;

    // Prefer the longest opener so triple quotes beat single quotes and raw
    // strings capture their marker count before shorter styles can match.
    for style in profile.string_styles.iter().copied() {
        let Some(opening) = string_opening(style, chars, idx) else {
            continue;
        };
        if opening.opening_len > best_opening_len {
            best_match = Some(StringMatch { style, opening });
            best_opening_len = opening.opening_len;
        }
    }

    best_match
}

/// Return opening metadata for one string style.
///
/// # Parameters
/// - `style`: Candidate string style to test.
/// - `chars`: Current line as a character slice.
/// - `idx`: Column where the opener would begin.
fn string_opening(style: StringStyle, chars: &[char], idx: usize) -> Option<StringOpening> {
    match style.kind {
        StringStyleKind::Delimited { open, .. } => {
            starts_with(chars, idx, open).then_some(StringOpening {
                repetition: 0,
                opening_len: open.chars().count(),
            })
        }
        StringStyleKind::HashDelimited {
            prefix,
            marker,
            quote,
        } => {
            if chars.get(idx).copied() != Some(prefix) {
                return None;
            }
            let mut repetition = 0;
            while chars.get(idx + 1 + repetition).copied() == Some(marker) {
                repetition += 1;
            }
            (chars.get(idx + 1 + repetition).copied() == Some(quote)).then_some(StringOpening {
                repetition,
                opening_len: repetition + 2,
            })
        }
    }
}

/// Consume one string literal.
///
/// # Parameters
/// - `chars`: Current line as a character slice.
/// - `start`: Column where the string opener begins.
/// - `style`: Active string style being consumed.
/// - `repetition`: Captured marker repetition for raw/hash-delimited strings.
/// - `opening_len`: Width of the already matched opener.
fn consume_string(
    chars: &[char],
    start: usize,
    style: StringStyle,
    repetition: usize,
    opening_len: usize,
) -> StringConsumeResult {
    match style.kind {
        StringStyleKind::Delimited {
            close,
            escape,
            multiline,
            ..
        } => {
            let search_start = start + opening_len;

            // Fixed-delimiter strings reuse the same search helper for ordinary
            // quoted strings and triple-quoted multiline strings.
            if let Some(end_idx) = find_delimited_close(chars, search_start, close, escape) {
                return StringConsumeResult {
                    end_col: end_idx,
                    closed: true,
                };
            }
            StringConsumeResult {
                end_col: chars.len(),
                closed: !multiline,
            }
        }
        StringStyleKind::HashDelimited { marker, quote, .. } => {
            let search_start = start + opening_len;

            // Raw strings carry the captured repetition count forward so the same
            // closer can be recognized on later lines.
            if let Some(end_idx) =
                find_hash_string_close(chars, search_start, quote, marker, repetition)
            {
                return StringConsumeResult {
                    end_col: end_idx,
                    closed: true,
                };
            }
            StringConsumeResult {
                end_col: chars.len(),
                closed: false,
            }
        }
    }
}

/// Return the first matching identifier style for `chars[start..end]`.
///
/// # Parameters
/// - `profile`: Language profile that provides identifier classification rules.
/// - `chars`: Current line as a character slice.
/// - `start`: Inclusive start column of the identifier token.
/// - `end`: Exclusive end column of the identifier token.
fn identifier_style(
    profile: &LanguageProfile,
    chars: &[char],
    start: usize,
    end: usize,
) -> Option<SpanStyle> {
    let token: String = chars[start..end].iter().collect();
    profile
        .identifier_rules
        .iter()
        .find(|rule| identifier_rule_matches(**rule, &token, chars, end))
        .map(|rule| rule.style)
}

/// Return whether one identifier rule matches the current token and context.
///
/// # Parameters
/// - `rule`: Identifier rule to evaluate.
/// - `token`: Already collected identifier text.
/// - `chars`: Current line as a character slice.
/// - `end`: Exclusive end column of `token` inside `chars`.
fn identifier_rule_matches(rule: IdentifierRule, token: &str, chars: &[char], end: usize) -> bool {
    let token_matches = match rule.match_kind {
        IdentifierMatch::Any => true,
        IdentifierMatch::ExactWords(words) => words.contains(&token),
    };
    if !token_matches {
        return false;
    }

    // Context filters let the generic lexer classify constructs like TOML bare
    // keys without inventing language-specific token walkers.
    match rule.context {
        IdentifierContext::Anywhere => true,
        IdentifierContext::BeforeChar {
            ch,
            allow_whitespace,
        } => {
            let mut idx = end;
            if allow_whitespace {
                while chars.get(idx).is_some_and(|next| next.is_whitespace()) {
                    idx += 1;
                }
            }
            chars.get(idx).copied() == Some(ch)
        }
    }
}

/// Return whether `chars[idx]` should be styled as punctuation.
fn punctuation_matches(profile: &LanguageProfile, chars: &[char], idx: usize) -> bool {
    let ch = chars[idx];
    profile.punctuation_chars.contains(ch)
        && !(ch == '.'
            && chars
                .get(idx.wrapping_sub(1))
                .is_some_and(|prev| prev.is_ascii_digit())
            && chars.get(idx + 1).is_some_and(|next| next.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{BufferEdit, LineLexMode, SyntaxEngine, lex_profile_line};
    use crate::syntax::profile::*;
    use crate::syntax::profiles::builtin_profiles;
    use crate::text_buffer::TextBuffer;
    use std::path::Path;

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
        assert!(engine.is_fully_lexed());
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
        assert_eq!(
            engine.document_state().line_states[1].exit_mode,
            LineLexMode::BlockComment {
                style: nested_block_comment("/*", "*/"),
                depth: 1
            }
        );

        buffer.insert(buffer.chars_count(), "*/\n");
        engine.apply_edit(
            &buffer,
            BufferEdit {
                start_line: 1,
                old_end_line: 1,
                new_end_line: 2,
            },
        );

        assert_eq!(
            engine.document_state().line_states[2].exit_mode,
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
        let distant_revision = engine.document_state().line_states[3].revision;

        // Split the first line so the edit introduces a newly inserted logical line.
        buffer.insert(4, "\n");
        engine.apply_edit(
            &buffer,
            BufferEdit {
                start_line: 0,
                old_end_line: 0,
                new_end_line: 1,
            },
        );

        assert_eq!(
            engine.document_state().line_states[4].revision,
            distant_revision
        );
    }

    /// Verify that removing a newline only relexes through the first unchanged tail line.
    #[test]
    fn test_remove_newline_stops_before_relexing_distant_tail_lines() {
        let mut buffer =
            TextBuffer::from_str("let alpha = 1;\nlet beta = 2;\nlet gamma = 3;\nlet delta = 4;\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        let distant_revision = engine.document_state().line_states[3].revision;

        // Merge the first two lines so the edit removes one logical line.
        buffer.remove(14, 15);
        engine.apply_edit(
            &buffer,
            BufferEdit {
                start_line: 0,
                old_end_line: 1,
                new_end_line: 0,
            },
        );

        assert_eq!(
            engine.document_state().line_states[2].revision,
            distant_revision
        );
    }

    /// Verify that nested D block comments retain depth correctly.
    #[test]
    fn test_nested_d_comment_depth_is_preserved() {
        let buffer = TextBuffer::from_str("/+ outer\n/+ inner +/\nstill outer\n+/");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.d")), &buffer);
        assert_eq!(
            engine.document_state().line_states[1].exit_mode,
            LineLexMode::BlockComment {
                style: nested_block_comment("/+", "+/"),
                depth: 1
            }
        );
        assert_eq!(
            engine.document_state().line_states[2].exit_mode,
            LineLexMode::BlockComment {
                style: nested_block_comment("/+", "+/"),
                depth: 1
            }
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
                style: raw_hash_string('r', '#', '"'),
                repetition: 3
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
                repetition: 0
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
