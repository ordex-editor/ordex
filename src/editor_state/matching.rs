//! `%` matching helpers for `EditorState`.

use super::*;
use crate::syntax::{CommentStyleKind, ReplayedLine};
use crate::text_buffer::TextSlice;
use std::collections::HashMap;

/// One normalized delimiter span in exclusive character coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchSpan {
    /// First character in the delimiter span.
    pub(crate) start: usize,
    /// One-past-the-end character in the delimiter span.
    pub(crate) end: usize,
}

impl MatchSpan {
    /// Return whether this span covers `char_idx`.
    fn contains(self, char_idx: usize) -> bool {
        (self.start..self.end).contains(&char_idx)
    }
}

/// Which visible endpoint role a highlighted character belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisibleMatchRole {
    /// The delimiter currently under the cursor.
    Source,
    /// The delimiter matched from the cursor-side source.
    Target,
}

/// Visible-only passive match pair used by rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibleMatch {
    /// Cursor-side delimiter span.
    source: MatchSpan,
    /// Visible matching delimiter span.
    target: MatchSpan,
}

/// Search scope used for `%` resolution and passive highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchSearchScope {
    /// Search across the full document.
    FullDocument,
    /// Search only the currently visible logical lines.
    VisibleLines {
        /// First visible logical line.
        first_line: usize,
        /// Last visible logical line.
        last_line: usize,
    },
}

/// Normalized match target resolved from the cursor or current line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchCandidate {
    /// Exact delimiter span used as the source endpoint.
    span: MatchSpan,
    /// Structural kind used to search for the mate.
    kind: MatchKind,
    /// Ignored-region fallback class when this bracket starts inside syntax the
    /// code-mode matcher should otherwise skip.
    region_class: Option<SyntaxClass>,
}

/// Search behavior for one resolved match candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    /// One bracket pair tracked by plain depth counting.
    Bracket {
        /// Opening delimiter for this pair.
        open: char,
        /// Closing delimiter for this pair.
        close: char,
        /// Direction required to find the mate from the current endpoint.
        direction: FindDirection,
    },
    /// One syntax-profile block-comment delimiter pair.
    Comment {
        /// Opening delimiter for this block-comment style.
        open: &'static str,
        /// Shared closing delimiter for the block-comment group.
        close: &'static str,
        /// Whether nested openers in this group increase comment depth.
        nests: bool,
        /// Direction required to find the mate from the current endpoint.
        direction: FindDirection,
    },
}

/// `%`-matching cache and visible passive highlight state.
#[derive(Debug, Clone)]
pub(crate) struct MatchingState {
    /// Generation that produced the cached `%` endpoint pairs.
    pub(crate) match_cache_generation: u64,
    /// Resolved `%` endpoint pairs keyed by source span start.
    pub(crate) match_cache: HashMap<usize, MatchSpan>,
    /// Visible-only passive match pair used by the renderer.
    ///
    /// This stores the cursor-side delimiter and its mate when both endpoints are
    /// already visible, so rendering can add passive highlighting without doing
    /// another full `%` search during paint.
    visible_match: Option<VisibleMatch>,
}

impl MatchingState {
    /// Build empty `%`-matching state.
    pub(crate) fn new() -> Self {
        Self {
            match_cache_generation: 0,
            match_cache: HashMap::new(),
            visible_match: None,
        }
    }

    /// Reset the cache and visible state for `generation`.
    pub(crate) fn reset(&mut self, generation: u64) {
        self.match_cache_generation = generation;
        self.match_cache.clear();
        self.visible_match = None;
    }

    /// Ensure the `%` endpoint cache matches `generation`.
    pub(crate) fn ensure_cache_generation(&mut self, generation: u64) {
        if self.match_cache_generation != generation {
            self.match_cache_generation = generation;
            self.match_cache.clear();
        }
    }

    /// Return the cached `%` target for `source_start`, if any.
    pub(crate) fn cached_match(&self, source_start: usize) -> Option<MatchSpan> {
        self.match_cache.get(&source_start).copied()
    }

    /// Cache one resolved `%` pair in both directions.
    pub(crate) fn cache_match_pair(&mut self, source: MatchSpan, target: MatchSpan) {
        self.match_cache.insert(source.start, target);
        self.match_cache.insert(target.start, source);
    }

    /// Return the visible match role covering `char_idx`, if any.
    pub(crate) fn visible_match_role(&self, char_idx: usize) -> Option<VisibleMatchRole> {
        let visible = self.visible_match?;
        if visible.source.contains(char_idx) {
            Some(VisibleMatchRole::Source)
        } else if visible.target.contains(char_idx) {
            Some(VisibleMatchRole::Target)
        } else {
            None
        }
    }

    /// Return whether one visible passive match endpoint intersects `line_idx`.
    pub(crate) fn line_has_visible_match(&self, buffer: &TextBuffer, line_idx: usize) -> bool {
        let Some(visible) = self.visible_match else {
            return false;
        };
        let line_start = buffer.line_to_char(line_idx);
        let line_end = line_start + buffer.line_len(line_idx);

        // Endpoint spans are exclusive, so any overlap means the rendered line
        // needs styled output even if syntax and selection are otherwise empty.
        visible.source.start < line_end && line_start < visible.source.end
            || visible.target.start < line_end && line_start < visible.target.end
    }

    /// Return a stable snapshot of the current visible passive match spans.
    pub(crate) fn visible_match_snapshot(&self) -> Option<(usize, usize, usize, usize)> {
        let visible = self.visible_match?;
        Some((
            visible.source.start,
            visible.source.end,
            visible.target.start,
            visible.target.end,
        ))
    }
}

/// Supported single-character bracket pairs for `%`.
const MATCHABLE_BRACKETS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];

/// Return the syntax class covering `column`, if any.
fn span_class_at(spans: &[HighlightSpan], column: usize) -> Option<SyntaxClass> {
    spans
        .iter()
        .find(|span| span.covers(column))
        .map(|span| span.class)
}

/// Return whether `class` represents syntax that code-mode bracket matching skips.
fn is_ignored_match_class(class: Option<SyntaxClass>) -> bool {
    matches!(class, Some(SyntaxClass::Comment | SyntaxClass::String))
}

/// Return whether `token` starts at `column` inside `text`.
fn text_matches_at(text: &TextSlice<'_>, column: usize, token: &str) -> bool {
    let mut suffix = text.chars().skip(column);

    // Rope-backed text may span multiple chunks, so compare by characters rather
    // than relying on one contiguous `&str` slice.
    token
        .chars()
        .all(|token_char| suffix.next() == Some(token_char))
}

/// Return one normalized bracket candidate for `ch`, if supported.
fn bracket_kind_for_char(ch: char) -> Option<MatchKind> {
    MATCHABLE_BRACKETS.iter().find_map(|&(open, close)| {
        if ch == open {
            Some(MatchKind::Bracket {
                open,
                close,
                direction: FindDirection::Forward,
            })
        } else if ch == close {
            Some(MatchKind::Bracket {
                open,
                close,
                direction: FindDirection::Backward,
            })
        } else {
            None
        }
    })
}

/// Prepare visible syntax, then refresh passive match state for the viewport.
pub(super) fn sync_visible_match_for_viewport(editor: &mut EditorState) {
    let content_height = editor.viewport.height();
    if content_height == 0 {
        editor.matching.visible_match = None;
        return;
    }

    let first_line = editor.viewport.first_visible_line();
    let last_line = first_line.saturating_add(content_height.saturating_sub(1));
    editor
        .syntax
        .prepare_visible_lines(&editor.buffer, first_line, last_line);
    refresh_visible_match(editor, content_height);
}

/// Recompute visible-only passive match spans from the current cursor position.
pub(super) fn refresh_visible_match(editor: &mut EditorState, content_height: usize) {
    // Passive bracket highlighting is useful in both modal (normal/visual) and
    // insert modes, so allow it whenever the editor is in one of those.
    let modal_or_insert = editor.mode_uses_modal_bindings() || editor.mode().is_insert();
    if !modal_or_insert || content_height == 0 {
        editor.matching.visible_match = None;
        return;
    }

    let first_line = editor.viewport.first_visible_line();
    let last_line = first_line.saturating_add(content_height.saturating_sub(1));
    let scope = MatchSearchScope::VisibleLines {
        first_line,
        last_line,
    };
    let Some(candidate) = resolve_match_candidate(editor, false) else {
        editor.matching.visible_match = None;
        return;
    };
    let Some(target) = find_match_for_candidate(editor, candidate, scope) else {
        editor.matching.visible_match = None;
        return;
    };

    // Passive highlighting is intentionally cursor-local, so only matches
    // anchored at the current cursor delimiter participate in rendering.
    editor.matching.visible_match = Some(VisibleMatch {
        source: candidate.span,
        target,
    });
}

/// Jump from the current or next-on-line delimiter to its matching endpoint.
pub(super) fn jump_to_matching_delimiter(editor: &mut EditorState) {
    let Some(candidate) = resolve_match_candidate(editor, true) else {
        return;
    };

    editor
        .matching
        .ensure_cache_generation(editor.syntax.generation());
    let target = editor
        .matching
        .cached_match(candidate.span.start)
        .or_else(|| find_match_for_candidate(editor, candidate, MatchSearchScope::FullDocument));
    let Some(target) = target else {
        return;
    };

    editor.cursor = Cursor::from_char_index(&editor.buffer, target.start);
}

/// Resolve the starting character index of the `%` matching delimiter target.
pub(super) fn matching_target_start(editor: &mut EditorState) -> Option<usize> {
    let candidate = resolve_match_candidate(editor, true)?;
    editor
        .matching
        .ensure_cache_generation(editor.syntax.generation());
    editor
        .matching
        .cached_match(candidate.span.start)
        .or_else(|| find_match_for_candidate(editor, candidate, MatchSearchScope::FullDocument))
        .map(|target| target.start)
}

/// Resolve the current `%` source delimiter from the cursor or current line.
fn resolve_match_candidate(
    editor: &EditorState,
    allow_next_on_line: bool,
) -> Option<MatchCandidate> {
    let replayed = replay_line(editor, editor.cursor.line())?;
    // In insert mode the cursor sits between characters, so both the character
    // at the cursor column and the one before it are "adjacent" and should be
    // checked for a bracket to match.
    let in_insert = editor.mode().is_insert();
    find_candidate_on_line(editor, &replayed, CandidateSearch::Cursor)
        .or_else(|| {
            in_insert
                .then(|| find_candidate_on_line(editor, &replayed, CandidateSearch::BeforeCursor))
                .flatten()
        })
        .or_else(|| {
            allow_next_on_line
                .then(|| find_candidate_on_line(editor, &replayed, CandidateSearch::Next))
                .flatten()
        })
}

/// Replay one logical line without perturbing the visible syntax window.
fn replay_line(editor: &EditorState, line_index: usize) -> Option<ReplayedLine<'_>> {
    editor
        .syntax
        .replay_line_range(&editor.buffer, line_index, line_index)
        .into_iter()
        .next()
}

/// Candidate search mode for one logical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateSearch {
    /// Only consider the cursor column itself.
    Cursor,
    /// Search to the right of the cursor for the first usable candidate.
    Next,
    /// Check the character immediately before the cursor column (insert mode).
    BeforeCursor,
}

/// How one block-comment delimiter candidate should relate to one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentCandidateSearch {
    /// Accept a token whose span covers the cursor column.
    Covering(usize),
    /// Accept a token whose first character starts at the scanned column.
    StartingAt(usize),
}

/// Resolve one `%` candidate from the cursor column or to its right on one line.
fn find_candidate_on_line(
    editor: &EditorState,
    line: &ReplayedLine<'_>,
    mode: CandidateSearch,
) -> Option<MatchCandidate> {
    let line_char_len = line.text.chars_count();
    let cursor_column = editor.cursor.column();
    let start_column = match mode {
        CandidateSearch::Cursor => cursor_column,
        CandidateSearch::Next => cursor_column.saturating_add(1),
        CandidateSearch::BeforeCursor => cursor_column.saturating_sub(1),
    };
    if start_column >= line_char_len {
        return None;
    }

    let line_start = editor.buffer.line_to_char(line.line_index);
    // The syntax region is determined from the column being checked, so that
    // BeforeCursor mode uses the region of the character before the cursor.
    let cursor_region = span_class_at(&line.spans, start_column)
        .filter(|class| matches!(class, SyntaxClass::Comment | SyntaxClass::String));

    // Cursor-local matching must first check whether the cursor sits anywhere
    // inside a multi-character block-comment delimiter before falling back to a
    // single-character bracket at that column.
    if matches!(mode, CandidateSearch::Cursor)
        && let Some(candidate) = comment_candidate_for_column(
            editor,
            line,
            line_start,
            CommentCandidateSearch::Covering(cursor_column),
        )
    {
        return Some(candidate);
    }

    // The cursor path checks only the current column, while the next-on-line
    // path walks right until it finds the first usable delimiter candidate.
    let end_column = match mode {
        CandidateSearch::Cursor | CandidateSearch::BeforeCursor => start_column + 1,
        CandidateSearch::Next => line_char_len,
    };

    for column in start_column..end_column {
        let class = span_class_at(&line.spans, column);
        if let Some(region_class) = cursor_region {
            // When `%` starts inside ignored syntax, it stays inside that same
            // string/comment region and uses plain bracket matching there.
            //
            // Block-comment delimiters are the one exception: even from inside
            // the comment body, `%` should still be able to lock onto a nearby
            // `/*`, `*/`, `/+`, or `+/` token and jump structurally from it.
            if class != Some(region_class) {
                return None;
            }
            if matches!(region_class, SyntaxClass::Comment)
                && let Some(candidate) = comment_candidate_for_column(
                    editor,
                    line,
                    line_start,
                    CommentCandidateSearch::StartingAt(column),
                )
            {
                return Some(candidate);
            }
            let char_idx = line_start + column;
            if let Some(kind) = editor
                .buffer
                .char_at(char_idx)
                .and_then(bracket_kind_for_char)
            {
                return Some(MatchCandidate {
                    span: MatchSpan {
                        start: char_idx,
                        end: char_idx + 1,
                    },
                    kind,
                    region_class: Some(region_class),
                });
            }
            continue;
        }

        // In code mode, `%` skips comment/string spans entirely until it finds
        // either a block-comment delimiter token or a bare bracket character.
        //
        // Check for a comment delimiter before discarding ignored spans so `%`
        // from just before `/*` or `*/` can still target that token.
        if let Some(candidate) = comment_candidate_for_column(
            editor,
            line,
            line_start,
            CommentCandidateSearch::StartingAt(column),
        ) {
            return Some(candidate);
        }
        if is_ignored_match_class(class) {
            continue;
        }
        let char_idx = line_start + column;
        if let Some(kind) = editor
            .buffer
            .char_at(char_idx)
            .and_then(bracket_kind_for_char)
        {
            return Some(MatchCandidate {
                span: MatchSpan {
                    start: char_idx,
                    end: char_idx + 1,
                },
                kind,
                region_class: None,
            });
        }
    }

    None
}

/// Resolve one block-comment delimiter candidate relative to one column.
fn comment_candidate_for_column(
    editor: &EditorState,
    line: &ReplayedLine<'_>,
    line_start: usize,
    search: CommentCandidateSearch,
) -> Option<MatchCandidate> {
    // `%` treats multi-character comment delimiters as one logical token. This
    // helper centralizes both ways the line scanner can discover those tokens:
    // either by landing somewhere inside the delimiter under the cursor, or by
    // visiting candidate start columns while walking right across the line.
    //
    // Cursor-local matching uses "covering" semantics so a cursor on the
    // middle `*` in `/**` still selects the full opener. Left-to-right scans
    // use "starting at" semantics so the first visible token start wins and
    // same-column brackets do not outrank a real block-comment delimiter.
    //
    // The starting-at variant intentionally runs even when the syntax span at
    // that column is already classified as comment. Delimiter bytes often
    // inherit the surrounding comment highlight, but `%` still needs to treat
    // the token boundary itself as actionable when scanning in code.
    best_comment_candidate(editor, line, line_start, |token_start, token_len| {
        let token_end = token_start + token_len;

        // The shared token walker enumerates every possible block-comment token
        // on the line, then the search mode decides whether this token counts
        // for the current cursor lookup or next-on-line scan step.
        match search {
            CommentCandidateSearch::Covering(column) => {
                token_end <= line.text.chars_count() && (token_start..token_end).contains(&column)
            }
            CommentCandidateSearch::StartingAt(column) => token_start == column,
        }
    })
}

/// Return the best block-comment candidate accepted by `predicate`.
fn best_comment_candidate(
    editor: &EditorState,
    line: &ReplayedLine<'_>,
    line_start: usize,
    predicate: impl Fn(usize, usize) -> bool,
) -> Option<MatchCandidate> {
    let mut best = None;
    let line_char_len = line.text.chars_count();

    // Both cursor-local matching and left-to-right line scans search the same
    // block-comment token space. They differ only in how a candidate column is
    // accepted: cursor matching accepts any delimiter covering the cursor,
    // whereas next-on-line matching only accepts delimiters that start there.
    for style in editor
        .syntax
        .active_comment_styles()
        .iter()
        .copied()
        .filter(|style| style.kind == CommentStyleKind::Block)
    {
        let close = style
            .close
            .expect("block comments must define a close delimiter");
        for (token, direction) in [
            (style.open, FindDirection::Forward),
            (close, FindDirection::Backward),
        ] {
            let token_len = token.chars().count();
            for start_column in 0..line_char_len {
                let end_column = start_column + token_len;

                // The shared walker enumerates every legal delimiter start on the
                // line, then lets the caller decide whether that token should be
                // considered "under the cursor" or "the next token on the line".
                if end_column > line_char_len
                    || !predicate(start_column, token_len)
                    || !text_matches_at(&line.text, start_column, token)
                {
                    continue;
                }
                let candidate = MatchCandidate {
                    span: MatchSpan {
                        start: line_start + start_column,
                        end: line_start + end_column,
                    },
                    kind: MatchKind::Comment {
                        open: style.open,
                        close,
                        nests: style.nests,
                        direction,
                    },
                    region_class: None,
                };
                if best.is_none_or(|(best_len, _): (usize, MatchCandidate)| token_len > best_len) {
                    best = Some((token_len, candidate));
                }
            }
        }
    }

    best.map(|(_, candidate)| candidate)
}

/// Resolve the matching endpoint for one `%` source delimiter.
fn find_match_for_candidate(
    editor: &mut EditorState,
    candidate: MatchCandidate,
    scope: MatchSearchScope,
) -> Option<MatchSpan> {
    let target = match candidate.kind {
        MatchKind::Bracket { .. } => find_bracket_match(editor, candidate, scope),
        MatchKind::Comment { .. } => find_comment_match(editor, candidate, scope),
    }?;

    if matches!(scope, MatchSearchScope::FullDocument) {
        // Full-document matches are stable within one syntax generation, so
        // cache both directions for immediate repeated `%` jumps.
        editor.matching.cache_match_pair(candidate.span, target);
    }

    Some(target)
}

/// Return the clamped logical line bounds covered by one search scope.
fn scope_line_bounds(editor: &EditorState, scope: MatchSearchScope) -> (usize, usize) {
    let last_line = editor.buffer.lines_count().saturating_sub(1);
    match scope {
        MatchSearchScope::FullDocument => (0, last_line),
        MatchSearchScope::VisibleLines {
            first_line,
            last_line: scope_last,
        } => (first_line.min(last_line), scope_last.min(last_line)),
    }
}

/// Resolve one bracket match by scanning through exact replayed syntax spans.
fn find_bracket_match(
    editor: &EditorState,
    candidate: MatchCandidate,
    scope: MatchSearchScope,
) -> Option<MatchSpan> {
    let MatchKind::Bracket {
        open,
        close,
        direction,
    } = candidate.kind
    else {
        return None;
    };
    let source_line = editor.buffer.char_to_line(candidate.span.start);
    let source_column = candidate.span.start - editor.buffer.line_to_char(source_line);
    let (scope_start, scope_end) = scope_line_bounds(editor, scope);
    let replayed = match direction {
        FindDirection::Forward => {
            editor
                .syntax
                .replay_line_range(&editor.buffer, source_line, scope_end)
        }
        FindDirection::Backward => {
            editor
                .syntax
                .replay_line_range(&editor.buffer, scope_start, source_line)
        }
    };
    let mut depth = 1usize;

    // The forward and backward walkers share the same depth rules, but the
    // ignored-region fallback stops as soon as syntax leaves that region.
    // That keeps `%` inside one string/comment region when fallback matching is
    // active, while code-mode scans simply ignore comment/string spans.
    match direction {
        FindDirection::Forward => {
            for line in replayed {
                let line_start = editor.buffer.line_to_char(line.line_index);
                let line_len = line.text.chars_count();
                let start_column = if line.line_index == source_line {
                    source_column.saturating_add(1)
                } else {
                    0
                };
                for column in start_column..line_len {
                    // Each replayed line already carries exact syntax spans, so
                    // the bracket walker can cheaply skip ignored regions while
                    // still counting nested delimiters in code.
                    let class = span_class_at(&line.spans, column);
                    if let Some(region_class) = candidate.region_class {
                        if class != Some(region_class) {
                            return None;
                        }
                    } else if is_ignored_match_class(class) {
                        continue;
                    }
                    let ch = editor.buffer.char_at(line_start + column)?;
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some(MatchSpan {
                                start: line_start + column,
                                end: line_start + column + 1,
                            });
                        }
                    }
                }
            }
        }
        FindDirection::Backward => {
            for line in replayed.into_iter().rev() {
                let line_start = editor.buffer.line_to_char(line.line_index);
                let end_column = if line.line_index == source_line {
                    source_column
                } else {
                    line.text.chars_count()
                };
                for column in (0..end_column).rev() {
                    // Backward scans mirror the forward logic: closers increase
                    // depth, openers decrease it, and ignored regions are either
                    // skipped or terminate fallback matching.
                    let class = span_class_at(&line.spans, column);
                    if let Some(region_class) = candidate.region_class {
                        if class != Some(region_class) {
                            return None;
                        }
                    } else if is_ignored_match_class(class) {
                        continue;
                    }
                    let ch = editor.buffer.char_at(line_start + column)?;
                    if ch == close {
                        depth += 1;
                    } else if ch == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some(MatchSpan {
                                start: line_start + column,
                                end: line_start + column + 1,
                            });
                        }
                    }
                }
            }
        }
    }

    None
}

/// Resolve one block-comment delimiter match using comment-style metadata.
fn find_comment_match(
    editor: &EditorState,
    candidate: MatchCandidate,
    scope: MatchSearchScope,
) -> Option<MatchSpan> {
    let MatchKind::Comment {
        open,
        close,
        nests,
        direction,
    } = candidate.kind
    else {
        return None;
    };
    let source_line = editor.buffer.char_to_line(candidate.span.start);
    let source_column = candidate.span.start - editor.buffer.line_to_char(source_line);
    let source_len = candidate.span.end - candidate.span.start;
    let open_len = open.chars().count();
    let close_len = close.chars().count();
    let (scope_start, scope_end) = scope_line_bounds(editor, scope);
    let replayed = match direction {
        FindDirection::Forward => {
            editor
                .syntax
                .replay_line_range(&editor.buffer, source_line, scope_end)
        }
        FindDirection::Backward => {
            editor
                .syntax
                .replay_line_range(&editor.buffer, scope_start, source_line)
        }
    };
    let mut depth = 1usize;

    // Comment matching is structural, so once `%` starts on a block-comment
    // delimiter it counts nested openers/closers directly from raw text.
    // This intentionally ignores syntax classes because the delimiter tokens
    // themselves define the structure we are matching.
    match direction {
        FindDirection::Forward => {
            for line in replayed {
                let line_start = editor.buffer.line_to_char(line.line_index);
                let line_len = line.text.chars_count();
                let mut column = if line.line_index == source_line {
                    source_column + source_len
                } else {
                    0
                };
                // Nested comment styles push depth on each opener and pop it on
                // each closer; non-nesting styles only look for the first closer.
                while column < line_len {
                    if nests && text_matches_at(&line.text, column, open) {
                        depth += 1;
                        column += open_len;
                        continue;
                    }
                    if text_matches_at(&line.text, column, close) {
                        depth -= 1;
                        if depth == 0 {
                            return Some(MatchSpan {
                                start: line_start + column,
                                end: line_start + column + close_len,
                            });
                        }
                        column += close_len;
                        continue;
                    }
                    column += 1;
                }
            }
        }
        FindDirection::Backward => {
            for line in replayed.into_iter().rev() {
                let line_start = editor.buffer.line_to_char(line.line_index);
                let line_len = line.text.chars_count();
                let limit = if line.line_index == source_line {
                    source_column
                } else {
                    line_len
                };
                let mut column = limit;
                // Backward comment matching mirrors the forward scan but walks by
                // token starts so openers/closers are compared at valid columns.
                while column > 0 {
                    let start_column = column - 1;
                    if nests
                        && start_column + close_len <= limit
                        && text_matches_at(&line.text, start_column, close)
                    {
                        depth += 1;
                        column = start_column;
                        continue;
                    }
                    if start_column + open_len <= limit
                        && text_matches_at(&line.text, start_column, open)
                    {
                        depth -= 1;
                        if depth == 0 {
                            return Some(MatchSpan {
                                start: line_start + start_column,
                                end: line_start + start_column + open_len,
                            });
                        }
                        column = start_column;
                        continue;
                    }
                    column -= 1;
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::super::EditorState;
    use crate::cursor::Cursor;
    use crate::mode::Mode;
    use crate::text_buffer::TextBuffer;

    /// Create an editor with the given content in insert mode at the given column.
    fn insert_mode_editor(content: &str, column: usize) -> EditorState {
        let mut editor = EditorState::new(24);
        editor.buffer = TextBuffer::from_str(content);
        editor.cursor = Cursor::new(0, column);
        editor.mode = Mode::Insert;
        editor
    }

    /// Verify that insert mode shows a bracket match when the cursor is before an opening bracket.
    #[test]
    fn test_insert_mode_match_cursor_before_open_bracket() {
        let mut editor = insert_mode_editor("(alpha)", 0);
        editor.prepare_syntax_view(1);
        let snapshot = editor.visible_match_snapshot();
        assert!(
            snapshot.is_some(),
            "should find a match with cursor before '('"
        );
        let (src_start, _src_end, tgt_start, _tgt_end) = snapshot.unwrap();
        assert_eq!(src_start, 0);
        assert_eq!(tgt_start, 6);
    }

    /// Verify that insert mode shows a bracket match when the cursor is after a closing bracket.
    #[test]
    fn test_insert_mode_match_cursor_after_close_bracket() {
        let mut editor = insert_mode_editor("(alpha)", 7);
        editor.prepare_syntax_view(1);
        let snapshot = editor.visible_match_snapshot();
        assert!(
            snapshot.is_some(),
            "should find a match with cursor after ')'"
        );
        let (src_start, _src_end, tgt_start, _tgt_end) = snapshot.unwrap();
        assert_eq!(src_start, 6);
        assert_eq!(tgt_start, 0);
    }

    /// Verify that insert mode shows a bracket match when the cursor is right after an opening bracket.
    #[test]
    fn test_insert_mode_match_cursor_after_open_bracket() {
        let mut editor = insert_mode_editor("(alpha)", 1);
        editor.prepare_syntax_view(1);
        let snapshot = editor.visible_match_snapshot();
        assert!(
            snapshot.is_some(),
            "should find a match with cursor after '('",
        );
        let (src_start, _src_end, tgt_start, _tgt_end) = snapshot.unwrap();
        assert_eq!(src_start, 0);
        assert_eq!(tgt_start, 6);
    }

    /// Verify that insert mode shows a bracket match when the cursor is right before a closing bracket.
    #[test]
    fn test_insert_mode_match_cursor_before_close_bracket() {
        let mut editor = insert_mode_editor("(alpha)", 6);
        editor.prepare_syntax_view(1);
        let snapshot = editor.visible_match_snapshot();
        assert!(
            snapshot.is_some(),
            "should find a match with cursor before ')'",
        );
        let (src_start, _src_end, tgt_start, _tgt_end) = snapshot.unwrap();
        assert_eq!(src_start, 6);
        assert_eq!(tgt_start, 0);
    }

    /// Verify that no match is shown when no bracket is adjacent to the cursor in insert mode.
    #[test]
    fn test_insert_mode_no_match_when_no_adjacent_bracket() {
        let mut editor = insert_mode_editor("alpha", 2);
        editor.prepare_syntax_view(1);
        assert!(
            editor.visible_match_snapshot().is_none(),
            "should not find a match when no bracket is adjacent",
        );
    }

    /// Verify that nested brackets match the innermost pair adjacent to the cursor.
    #[test]
    fn test_insert_mode_match_nested_brackets() {
        let mut editor = insert_mode_editor("((a))", 0);
        editor.prepare_syntax_view(1);
        let snapshot = editor.visible_match_snapshot();
        assert!(snapshot.is_some(), "cursor before outer '(' should match");
        let (src_start, _src_end, tgt_start, _tgt_end) = snapshot.unwrap();
        assert_eq!(src_start, 0);
        assert_eq!(tgt_start, 4);

        let mut editor = insert_mode_editor("((a))", 1);
        editor.prepare_syntax_view(1);
        let snapshot = editor.visible_match_snapshot();
        assert!(snapshot.is_some(), "cursor before inner '(' should match");
        let (src_start, _src_end, tgt_start, _tgt_end) = snapshot.unwrap();
        assert_eq!(src_start, 1);
        assert_eq!(tgt_start, 3);
    }

    /// Verify that no match is shown at line start when the first character is not a bracket.
    #[test]
    fn test_insert_mode_no_match_at_line_start_without_bracket() {
        let mut editor = insert_mode_editor("hello", 0);
        editor.prepare_syntax_view(1);
        assert!(
            editor.visible_match_snapshot().is_none(),
            "should not find a match at line start without a bracket",
        );
    }
}
