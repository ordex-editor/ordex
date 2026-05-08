//! Search-result highlight helpers for `EditorState`.

use super::*;

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
}

impl SearchHighlightState {
    /// Build empty search-highlight state.
    pub(crate) fn new() -> Self {
        Self {
            preview: SearchPreview::Inactive,
            visible_matches: Vec::new(),
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

    /// Replace the cached visible match list.
    pub(crate) fn set_visible_matches(&mut self, visible_matches: Vec<SearchMatch>) {
        self.visible_matches = visible_matches;
    }

    /// Return whether a visible search highlight covers `char_idx`.
    ///
    /// Returns `true` when `char_idx` is inside one cached visible search-result
    /// span, and `false` when no cached visible search-result span covers it.
    pub(crate) fn contains_char(&self, char_idx: usize) -> bool {
        self.visible_matches
            .iter()
            .any(|search_match| (search_match.start..search_match.end).contains(&char_idx))
    }

    /// Return whether one cached visible search highlight intersects `line_idx`.
    ///
    /// Returns `true` when one cached visible search-result span overlaps the
    /// logical line, and `false` when the line has no cached visible result.
    pub(crate) fn line_has_visible_match(&self, buffer: &TextBuffer, line_idx: usize) -> bool {
        let line_start = buffer.line_to_char(line_idx);
        let line_end = line_start + buffer.line_len(line_idx);

        // Search-result spans are exclusive, so any overlap means the renderer
        // must take the styled-content path for this line.
        self.visible_matches
            .iter()
            .any(|search_match| search_match.start < line_end && line_start < search_match.end)
    }

    /// Return a stable snapshot of the cached visible search-result spans.
    pub(crate) fn snapshot(&self) -> Vec<(usize, usize)> {
        self.visible_matches
            .iter()
            .map(|search_match| (search_match.start, search_match.end))
            .collect()
    }
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
        editor.search_highlighting.set_visible_matches(Vec::new());
        return;
    };
    if content_height == 0 {
        editor.search_highlighting.set_visible_matches(Vec::new());
        return;
    }

    let line_count = editor.buffer.lines_count();
    if line_count == 0 {
        editor.search_highlighting.set_visible_matches(Vec::new());
        return;
    }

    // Visible search highlighting is line-scoped so row wrapping and horizontal
    // scroll can reuse the same cached spans without re-running regex searches.
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
        .set_visible_matches(visible_matches);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Search mode should suppress committed highlights until the preview is valid.
    fn test_sync_for_viewport_hides_committed_highlights_for_empty_search_prompt() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("alpha beta");
        editor.last_search = Some(SearchQuery::compile("alpha").expect("compile regex"));

        editor.enter_search_prompt();
        sync_for_viewport(&mut editor);

        assert_eq!(
            editor.search_highlight_snapshot(),
            Vec::<(usize, usize)>::new()
        );
    }

    #[test]
    /// Invalid preview regexes should suppress visible highlights until the query is fixed.
    fn test_sync_for_viewport_hides_highlights_for_invalid_preview_regex() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("alpha beta");
        editor.enter_search_prompt();
        editor.replace_active_prompt_text("(?=beta)".to_string());
        sync_for_viewport(&mut editor);

        assert_eq!(
            editor.search_highlight_snapshot(),
            Vec::<(usize, usize)>::new()
        );
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
}
