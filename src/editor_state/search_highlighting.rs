//! Search-result highlight helpers for `EditorState`.

use super::EditorState;
use crate::mode::Mode;
use crate::search::{SearchMatch, SearchQuery};
use crate::text_buffer::TextBuffer;

/// One visible search-result span in line-local display columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SearchHighlightSpan {
    /// First covered display column on the line.
    pub(crate) start_col: usize,
    /// One-past-the-end covered display column on the line.
    pub(crate) end_col: usize,
}

impl SearchHighlightSpan {
    /// Return whether this visible span covers `column`.
    pub(crate) fn covers(self, column: usize) -> bool {
        (self.start_col..self.end_col).contains(&column)
    }
}

/// Visible search-result spans grouped by logical line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchHighlightLine {
    /// Logical line index for these spans.
    line_idx: usize,
    /// Visible spans on that line in ascending column order.
    spans: Vec<SearchHighlightSpan>,
}

/// Preview-query state derived from the active `/` prompt.
#[derive(Debug, Clone, Default)]
enum SearchPreview {
    /// No `/` prompt is active, so committed search state drives highlights.
    #[default]
    Inactive,
    /// `/` is active with an empty query, so no results are highlighted.
    Empty,
    /// `/` is active with an invalid regex, so no results are highlighted.
    Invalid,
    /// `/` is active with one compiled preview query.
    Query(SearchQuery),
}

/// Search-result highlight state cached for the visible viewport.
#[derive(Debug, Clone)]
pub(crate) struct SearchHighlightState {
    preview: SearchPreview,
    visible_matches: Vec<SearchMatch>,
    visible_lines: Vec<SearchHighlightLine>,
}

impl SearchHighlightState {
    /// Build empty search-highlight state.
    pub(crate) fn new() -> Self {
        Self {
            preview: SearchPreview::Inactive,
            visible_matches: Vec::new(),
            visible_lines: Vec::new(),
        }
    }

    /// Sync the preview query from the current editor mode.
    pub(crate) fn sync_preview_from_mode(&mut self, mode: &Mode) {
        self.preview = match mode.search_string() {
            Some("") => SearchPreview::Empty,
            Some(pattern) => match SearchQuery::compile(pattern) {
                Ok(query) => SearchPreview::Query(query),
                Err(_) => SearchPreview::Invalid,
            },
            None => SearchPreview::Inactive,
        };
    }

    /// Return the query that should drive visible search-result highlights.
    fn active_query<'a>(&'a self, committed: Option<&'a SearchQuery>) -> Option<&'a SearchQuery> {
        match &self.preview {
            SearchPreview::Inactive => committed,
            SearchPreview::Query(query) => Some(query),
            SearchPreview::Empty | SearchPreview::Invalid => None,
        }
    }

    /// Replace the cached visible match list and per-line spans.
    pub(crate) fn set_visible_matches(
        &mut self,
        visible_matches: Vec<SearchMatch>,
        buffer: &TextBuffer,
    ) {
        self.visible_lines = build_visible_lines(buffer, &visible_matches);
        self.visible_matches = visible_matches;
    }

    /// Return whether one cached visible search highlight intersects `line_idx`.
    ///
    /// Returns `true` when one cached visible search-result span overlaps the
    /// logical line, and `false` when the line has no cached visible result.
    pub(crate) fn line_has_visible_match(&self, line_idx: usize) -> bool {
        self.line_spans(line_idx)
            .is_some_and(|spans| !spans.is_empty())
    }

    /// Return the visible search-result spans for `line_idx`, if any.
    pub(crate) fn line_spans(&self, line_idx: usize) -> Option<&[SearchHighlightSpan]> {
        let line_idx = self
            .visible_lines
            .binary_search_by_key(&line_idx, |line| line.line_idx)
            .ok()?;
        Some(&self.visible_lines[line_idx].spans)
    }

    /// Return a stable snapshot of the cached visible search-result spans.
    pub(crate) fn snapshot(&self) -> Vec<(usize, usize)> {
        self.visible_matches
            .iter()
            .map(|search_match| (search_match.start, search_match.end))
            .collect()
    }
}

/// Convert visible character-based matches into per-line display spans.
fn build_visible_lines(
    buffer: &TextBuffer,
    visible_matches: &[SearchMatch],
) -> Vec<SearchHighlightLine> {
    let mut visible_lines = Vec::new();

    for &search_match in visible_matches {
        if search_match.start >= search_match.end {
            continue;
        }

        // Regex matches may span multiple lines, so split them into one
        // display-span segment per visible logical line.
        let start_line = buffer.char_to_line(search_match.start);
        let end_line = buffer.char_to_line(search_match.end.saturating_sub(1));
        for line_idx in start_line..=end_line {
            let line_start = buffer.line_to_char(line_idx);
            let line_end = line_start + buffer.line_len(line_idx);
            let start_col = search_match.start.max(line_start) - line_start;
            let end_col = search_match.end.min(line_end) - line_start;
            if start_col >= end_col {
                continue;
            }
            push_line_span(
                &mut visible_lines,
                line_idx,
                SearchHighlightSpan { start_col, end_col },
            );
        }
    }

    visible_lines
}

/// Append one visible span to the grouped line table.
fn push_line_span(
    visible_lines: &mut Vec<SearchHighlightLine>,
    line_idx: usize,
    span: SearchHighlightSpan,
) {
    if visible_lines
        .last()
        .is_some_and(|line| line.line_idx == line_idx)
    {
        visible_lines
            .last_mut()
            .expect("last line should exist when the index matches")
            .spans
            .push(span);
        return;
    }

    visible_lines.push(SearchHighlightLine {
        line_idx,
        spans: vec![span],
    });
}

/// Rebuild the preview query and cached visible matches for the current viewport.
pub(super) fn sync_for_viewport(editor: &mut EditorState) {
    editor
        .search_highlighting
        .sync_preview_from_mode(&editor.mode);
    refresh_visible_matches(editor, editor.viewport.height());
}

/// Refresh cached visible search-result spans for the current viewport.
pub(super) fn refresh_visible_matches(editor: &mut EditorState, content_height: usize) {
    let Some(query) = editor
        .search_highlighting
        .active_query(editor.last_search.as_ref())
        .cloned()
    else {
        editor
            .search_highlighting
            .set_visible_matches(Vec::new(), &editor.buffer);
        return;
    };
    if content_height == 0 {
        editor
            .search_highlighting
            .set_visible_matches(Vec::new(), &editor.buffer);
        return;
    }

    let line_count = editor.buffer.lines_count();
    if line_count == 0 {
        editor
            .search_highlighting
            .set_visible_matches(Vec::new(), &editor.buffer);
        return;
    }

    // Visible search highlighting stays viewport-scoped so prompt edits reuse a
    // bounded scan similar to mature editors that only decorate what is on screen.
    let first_line = editor
        .viewport
        .first_visible_line()
        .min(line_count.saturating_sub(1));
    let last_line = first_line
        .saturating_add(content_height.saturating_sub(1))
        .min(line_count.saturating_sub(1));
    let start_char = editor.buffer.line_to_char(first_line);
    let end_char = if last_line + 1 < line_count {
        editor.buffer.line_to_char(last_line + 1)
    } else {
        editor.buffer.chars_count()
    };
    let visible_matches = query.find_all_in_char_range(&editor.buffer, start_char, end_char);
    editor
        .search_highlighting
        .set_visible_matches(visible_matches, &editor.buffer);
}

#[cfg(test)]
mod tests {
    use super::{
        SearchHighlightSpan, build_visible_lines, refresh_visible_matches, sync_for_viewport,
    };
    use crate::editor_state::EditorState;
    use crate::search::{SearchMatch, SearchQuery};
    use crate::text_buffer::TextBuffer;

    #[test]
    /// Search mode should suppress committed highlights until the preview is valid.
    fn test_sync_for_viewport_hides_committed_highlights_for_empty_search_prompt() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("alpha beta");
        editor.last_search = Some(SearchQuery::compile("alpha").expect("compile regex"));

        editor.enter_search_prompt();
        sync_for_viewport(&mut editor);

        assert_eq!(editor.search_highlight_snapshot(), Vec::new());
    }

    #[test]
    /// Invalid preview regexes should suppress visible highlights until the query is fixed.
    fn test_sync_for_viewport_hides_highlights_for_invalid_preview_regex() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("alpha beta");
        editor.enter_search_prompt();
        editor.replace_active_prompt_text("(?=beta)".to_string());
        sync_for_viewport(&mut editor);

        assert_eq!(editor.search_highlight_snapshot(), Vec::new());
    }

    #[test]
    /// Committed searches should drive viewport highlights when no preview is active.
    fn test_refresh_visible_matches_uses_committed_last_search() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("alpha\nbeta\nalpha");
        editor.last_search = Some(SearchQuery::compile("alpha").expect("compile regex"));

        sync_for_viewport(&mut editor);

        assert_eq!(editor.search_highlight_snapshot(), vec![(0, 5), (11, 16)]);
    }

    #[test]
    /// Multi-line matches should split into one visible span segment per line.
    fn test_build_visible_lines_splits_multiline_matches() {
        let buffer = TextBuffer::from_str("alpha\nbeta");
        let visible_lines = build_visible_lines(&buffer, &[SearchMatch { start: 2, end: 8 }]);

        assert_eq!(
            visible_lines[0].spans,
            vec![SearchHighlightSpan {
                start_col: 2,
                end_col: 5,
            }]
        );
        assert_eq!(
            visible_lines[1].spans,
            vec![SearchHighlightSpan {
                start_col: 0,
                end_col: 2,
            }]
        );
    }

    #[test]
    /// Line-local visible spans should stay queryable without per-character scans.
    fn test_refresh_visible_matches_exposes_line_local_spans() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("alpha beta alpha");
        editor.last_search = Some(SearchQuery::compile("alpha").expect("compile regex"));

        refresh_visible_matches(&mut editor, 1);

        assert_eq!(
            editor.visible_search_match_spans(0),
            &[
                SearchHighlightSpan {
                    start_col: 0,
                    end_col: 5,
                },
                SearchHighlightSpan {
                    start_col: 11,
                    end_col: 16,
                },
            ]
        );
    }
}
