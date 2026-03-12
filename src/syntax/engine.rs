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
use std::path::Path;

/// One styled region within a logical buffer line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightSpan {
    /// Source line index.
    pub(crate) line_index: usize,
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
            line_index: 0,
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
    /// Generation number that produced this state.
    pub(crate) revision: u64,
    /// Whether this line is currently stable with respect to its entry mode.
    pub(crate) stable: bool,
}

impl Default for LineLexState {
    /// Build a plain, stable line state.
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
    /// Cached per-line spans.
    pub(crate) spans_by_line: Vec<Vec<HighlightSpan>>,
    /// First dirty line waiting for relexing, if any.
    pub(crate) dirty_start_line: Option<usize>,
    /// Monotonic syntax-generation counter.
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
            spans_by_line: vec![Vec::new()],
            dirty_start_line: None,
            generation: 0,
            fully_lexed: true,
        }
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
        self.document.spans_by_line = vec![Vec::new(); line_count];
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
        self.document
            .spans_by_line
            .get(line_index)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return the current syntax-generation number.
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
        self.document.spans_by_line = vec![Vec::new(); line_count];
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

        // Full-document lexing guarantees correct inherited state for multiline
        // constructs before the first frame is rendered.
        for line_index in 0..line_count {
            let line = buffer.line_for_display(line_index).unwrap_or_default();
            let mut parsed = lex_profile_line(profile, &line, entry_mode);
            for span in &mut parsed.spans {
                span.line_index = line_index;
            }
            self.document.line_states[line_index] = LineLexState {
                line_index,
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision,
                stable: true,
            };
            self.document.spans_by_line[line_index] = parsed.spans;
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
            let previous_spans = self.document.spans_by_line[line_index].clone();
            let previous_exit = self.document.line_states[line_index].exit_mode;
            let mut parsed = lex_profile_line(profile, &line, entry_mode);
            for span in &mut parsed.spans {
                span.line_index = line_index;
            }
            let unchanged = previous_spans == parsed.spans && previous_exit == parsed.exit_mode;

            self.document.line_states[line_index] = LineLexState {
                line_index,
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision,
                stable: true,
            };
            self.document.spans_by_line[line_index] = parsed.spans;
            entry_mode = self.document.line_states[line_index].exit_mode;

            if line_index >= min_relex_line && unchanged {
                break;
            }
        }

        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Splice cached line vectors to keep unchanged tail lines aligned after edits.
    fn splice_line_caches(&mut self, edit: BufferEdit) {
        let required_len = edit.old_end_line.saturating_add(1);
        if self.document.line_states.len() < required_len {
            self.document
                .line_states
                .resize(required_len, LineLexState::default());
        }
        if self.document.spans_by_line.len() < required_len {
            self.document.spans_by_line.resize(required_len, Vec::new());
        }
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
                    self.document.spans_by_line.insert(insert_at, Vec::new());
                }
            }
            Ordering::Less => {
                let remove_start = edit.start_line.saturating_add(new_count);
                let remove_end = edit
                    .old_end_line
                    .saturating_add(1)
                    .min(self.document.line_states.len());
                if remove_start < remove_end {
                    self.document.line_states.drain(remove_start..remove_end);
                    self.document.spans_by_line.drain(remove_start..remove_end);
                }
            }
            Ordering::Equal => {}
        }

        // Keep cached line-local metadata aligned after line insertions or removals so
        // unchanged tail spans remain comparable without forcing a full relex.
        for (line_index, state) in self.document.line_states.iter_mut().enumerate() {
            state.line_index = line_index;
        }
        for (line_index, spans) in self.document.spans_by_line.iter_mut().enumerate() {
            for span in spans {
                span.line_index = line_index;
            }
        }
    }
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
            let (end_idx, end_depth) =
                consume_block_comment(profile, &chars, 0, style, depth, false);
            spans.push(HighlightSpan::styled(0, end_idx, style.span_style()));
            idx = end_idx;
            if end_depth > 0 {
                exit_mode = LineLexMode::BlockComment {
                    style,
                    depth: end_depth,
                };
                return LineParseResult { spans, exit_mode };
            }
        }
        LineLexMode::String { style, repetition } => {
            let (end_idx, closed) = consume_string(&chars, 0, style, repetition, 0);
            spans.push(HighlightSpan::styled(0, end_idx, STRING_STYLE));
            idx = end_idx;
            if !closed {
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
            let (end_idx, end_depth) = consume_block_comment(profile, &chars, idx, style, 1, true);
            spans.push(HighlightSpan::styled(idx, end_idx, style.span_style()));
            if end_depth > 0 {
                exit_mode = LineLexMode::BlockComment {
                    style,
                    depth: end_depth,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if let Some((style, repetition, opening_len)) = match_string_style(profile, &chars, idx) {
            let (end_idx, closed) = consume_string(&chars, idx, style, repetition, opening_len);
            spans.push(HighlightSpan::styled(idx, end_idx, STRING_STYLE));
            if !closed {
                exit_mode = LineLexMode::String { style, repetition };
                break;
            }
            idx = end_idx;
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

/// Consume one block comment and return its ending column and remaining depth.
fn consume_block_comment(
    profile: &LanguageProfile,
    chars: &[char],
    start: usize,
    style: CommentStyle,
    initial_depth: usize,
    initial_open_consumed: bool,
) -> (usize, usize) {
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
                return (idx, 0);
            }
            continue;
        }
        idx += 1;
    }

    (chars.len(), depth)
}

/// Return the best matching string opener at `idx`.
fn match_string_style(
    profile: &LanguageProfile,
    chars: &[char],
    idx: usize,
) -> Option<(StringStyle, usize, usize)> {
    let mut best_match = None;
    let mut best_opening_len = 0;

    // Prefer the longest opener so triple quotes beat single quotes and raw
    // strings capture their marker count before shorter styles can match.
    for style in profile.string_styles.iter().copied() {
        let Some((repetition, opening_len)) = string_opening(style, chars, idx) else {
            continue;
        };
        if opening_len > best_opening_len {
            best_match = Some((style, repetition, opening_len));
            best_opening_len = opening_len;
        }
    }

    best_match
}

/// Return the repetition count and opening length for one string style.
fn string_opening(style: StringStyle, chars: &[char], idx: usize) -> Option<(usize, usize)> {
    match style.kind {
        StringStyleKind::Delimited { open, .. } => {
            starts_with(chars, idx, open).then_some((0, open.chars().count()))
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
            (chars.get(idx + 1 + repetition).copied() == Some(quote))
                .then_some((repetition, repetition + 2))
        }
    }
}

/// Consume one string and return its ending column plus closed/open status.
fn consume_string(
    chars: &[char],
    start: usize,
    style: StringStyle,
    repetition: usize,
    opening_len: usize,
) -> (usize, bool) {
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
                return (end_idx, true);
            }
            (chars.len(), !multiline)
        }
        StringStyleKind::HashDelimited { marker, quote, .. } => {
            let search_start = start + opening_len;

            // Raw strings carry the captured repetition count forward so the same
            // closer can be recognized on later lines.
            if let Some(end_idx) =
                find_hash_string_close(chars, search_start, quote, marker, repetition)
            {
                return (end_idx, true);
            }
            (chars.len(), false)
        }
    }
}

/// Return the first matching identifier style for `chars[start..end]`.
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
