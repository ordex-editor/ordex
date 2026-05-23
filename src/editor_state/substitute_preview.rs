//! Transient `:s` preview lifecycle for `EditorState`.

use super::search_highlighting::{SearchHighlightLine, SearchHighlightSpan, build_highlight_lines};
use super::*;
use crate::substitute::{
    PreviewSubstituteCommand, SubstituteCommand, SubstitutePatternPreview, SubstitutePlan,
    build_substitute_plan, parse_substitute_input,
};
use crate::syntax::HighlightSpan;

/// One visible line plus exact syntax spans replayed from the preview buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewSyntaxLine {
    line_idx: usize,
    spans: Vec<HighlightSpan>,
}

/// Incremental substitute preview mode active in command input.
#[derive(Debug, Clone)]
enum SubstitutePreviewMode {
    Search {
        preview: SubstitutePatternPreview,
        query: SearchQuery,
    },
    Replace {
        command: SubstituteCommand,
        plan: SubstitutePlan,
        buffer: TextBuffer,
        replacement_matches: Vec<SearchMatch>,
    },
}

/// One active substitute preview rendered from transient state.
#[derive(Debug, Clone)]
pub(super) struct SubstitutePreviewState {
    original_viewport: Viewport,
    mode: SubstitutePreviewMode,
    visible_highlights: Vec<SearchHighlightLine>,
    visible_syntax: Vec<PreviewSyntaxLine>,
}

impl SubstitutePreviewState {
    /// Build one search-only preview that highlights the typed substitute pattern.
    fn search(
        preview: SubstitutePatternPreview,
        query: SearchQuery,
        original_viewport: Viewport,
    ) -> Self {
        Self {
            original_viewport,
            mode: SubstitutePreviewMode::Search { preview, query },
            visible_highlights: Vec::new(),
            visible_syntax: Vec::new(),
        }
    }

    /// Build one replacement preview with a transient buffer and replacement spans.
    fn replace(
        command: SubstituteCommand,
        plan: SubstitutePlan,
        buffer: &TextBuffer,
        original_viewport: Viewport,
    ) -> Self {
        let (buffer, replacement_matches) = build_preview_buffer_and_matches(buffer, &plan);

        Self {
            original_viewport,
            mode: SubstitutePreviewMode::Replace {
                command,
                plan,
                buffer,
                replacement_matches,
            },
            visible_highlights: Vec::new(),
            visible_syntax: Vec::new(),
        }
    }

    /// Borrow the transient render buffer when replacement preview is active.
    pub(super) fn render_buffer(&self) -> Option<&TextBuffer> {
        match &self.mode {
            SubstitutePreviewMode::Search { .. } => None,
            SubstitutePreviewMode::Replace { buffer, .. } => Some(buffer),
        }
    }

    /// Return the cached preview highlight spans for `line_idx`, if any.
    pub(super) fn line_spans(&self, line_idx: usize) -> Option<&[SearchHighlightSpan]> {
        let index = self
            .visible_highlights
            .binary_search_by_key(&line_idx, |line| line.line_idx)
            .ok()?;
        Some(&self.visible_highlights[index].spans)
    }

    /// Return the cached replayed syntax spans for `line_idx`, if any.
    pub(super) fn syntax_spans(&self, line_idx: usize) -> Option<&[HighlightSpan]> {
        let index = self
            .visible_syntax
            .binary_search_by_key(&line_idx, |line| line.line_idx)
            .ok()?;
        Some(&self.visible_syntax[index].spans)
    }

    /// Recompute visible preview highlights and replayed syntax for the viewport.
    fn refresh_for_viewport(
        &mut self,
        syntax: &crate::syntax::SyntaxEngine,
        viewport: &Viewport,
        committed_buffer: &TextBuffer,
        current_line: usize,
    ) {
        let render_buffer = self.render_buffer().unwrap_or(committed_buffer);
        let visible_range = visible_char_range_for_viewport(viewport, render_buffer);
        // Search-only preview highlights the typed pattern against the committed
        // buffer, while replacement preview highlights the replacement spans in
        // the transient preview buffer.
        let visible_matches = match &self.mode {
            SubstitutePreviewMode::Search { preview, query } => {
                let (scope_start, scope_end) =
                    preview.scope.char_range(committed_buffer, current_line);
                let start_char = visible_range.0.max(scope_start);
                let end_char = visible_range.1.min(scope_end);
                query.find_all_in_char_range(committed_buffer, start_char, end_char)
            }
            SubstitutePreviewMode::Replace {
                replacement_matches,
                ..
            } => replacement_matches
                .iter()
                .copied()
                .filter(|search_match| {
                    search_match.end > visible_range.0 && search_match.start < visible_range.1
                })
                .collect(),
        };
        self.visible_highlights = build_highlight_lines(render_buffer, &visible_matches);
        // Replacement preview needs exact syntax replay from the preview buffer
        // so token boundaries remain correct after edited text shifts the line.
        self.visible_syntax = match &self.mode {
            SubstitutePreviewMode::Search { .. } => Vec::new(),
            SubstitutePreviewMode::Replace { buffer, .. } => {
                let line_count = buffer.lines_count();
                if line_count == 0 {
                    Vec::new()
                } else {
                    let first_line = viewport
                        .first_visible_line()
                        .min(line_count.saturating_sub(1));
                    let last_line = first_line
                        .saturating_add(viewport.height().saturating_sub(1))
                        .min(line_count.saturating_sub(1));
                    syntax
                        .replay_line_range(buffer, first_line, last_line)
                        .into_iter()
                        .map(|line| PreviewSyntaxLine {
                            line_idx: line.line_index,
                            spans: line.spans,
                        })
                        .collect()
                }
            }
        };
    }
}

impl EditorState {
    /// Refresh substitute preview state from the active command prompt.
    pub(super) fn refresh_substitute_preview(&mut self) {
        let Some(input) = self.mode.command_string() else {
            self.clear_substitute_preview(true);
            return;
        };
        // Parsing decides whether command input is still only a search-pattern
        // preview or whether it is ready to render a replacement preview buffer.
        match parse_substitute_input(input) {
            PreviewSubstituteCommand::NotSubstitute => {
                self.clear_substitute_preview(true);
            }
            PreviewSubstituteCommand::Incomplete { preview, .. } => {
                self.activate_incomplete_substitute_preview(preview);
            }
            PreviewSubstituteCommand::Invalid(error) => {
                self.clear_substitute_preview(true);
                self.show_status_message(error);
            }
            PreviewSubstituteCommand::Ready(command) => {
                self.activate_ready_substitute_preview(command);
            }
        }
    }

    /// Recompute viewport-scoped substitute preview caches after one render/layout change.
    pub(super) fn refresh_substitute_preview_for_viewport(&mut self) {
        let Some(preview) = self.substitute_preview.as_mut() else {
            return;
        };
        preview.refresh_for_viewport(
            &self.syntax,
            &self.viewport,
            &self.buffer,
            self.cursor.line(),
        );
    }

    /// Clear the active substitute preview and optionally restore the old viewport.
    pub(super) fn clear_substitute_preview(&mut self, restore_viewport: bool) {
        let Some(preview) = self.substitute_preview.take() else {
            return;
        };
        if restore_viewport {
            self.viewport = preview.original_viewport;
        }
        self.bump_substitute_preview_revision();
    }

    /// Consume one active replacement preview for Enter-driven command execution.
    pub(super) fn take_substitute_preview_for_commit(
        &mut self,
        command: &SubstituteCommand,
    ) -> Option<SubstitutePlan> {
        let preview = self.substitute_preview.take()?;
        self.bump_substitute_preview_revision();
        match preview.mode {
            SubstitutePreviewMode::Replace {
                command: preview_command,
                plan,
                ..
            } if preview_command == *command => Some(plan),
            SubstitutePreviewMode::Replace { .. } | SubstitutePreviewMode::Search { .. } => None,
        }
    }

    /// Return whether substitute preview currently overrides the rendered buffer.
    ///
    /// Returns `true` when the viewport should read visible text from the
    /// replacement preview buffer, and `false` for search-only preview or the
    /// ordinary committed buffer.
    #[cfg(test)]
    pub(super) fn substitute_preview_uses_render_buffer(&self) -> bool {
        self.substitute_preview
            .as_ref()
            .is_some_and(|preview| preview.render_buffer().is_some())
    }

    /// Borrow the cached preview highlight spans for `line_idx`, if any.
    pub(super) fn substitute_preview_line_spans(&self, line_idx: usize) -> &[SearchHighlightSpan] {
        self.substitute_preview
            .as_ref()
            .and_then(|preview| preview.line_spans(line_idx))
            .unwrap_or(&[])
    }

    /// Borrow replayed syntax spans for preview lines when replacement preview is active.
    pub(super) fn substitute_preview_syntax_spans(
        &self,
        line_idx: usize,
    ) -> Option<&[HighlightSpan]> {
        self.substitute_preview
            .as_ref()
            .and_then(|preview| preview.syntax_spans(line_idx))
    }

    /// Activate a search-only preview from incomplete substitute input.
    fn activate_incomplete_substitute_preview(
        &mut self,
        preview: Option<SubstitutePatternPreview>,
    ) {
        let Some(preview) = preview else {
            self.clear_substitute_preview(true);
            self.clear_status_message();
            return;
        };
        // Compile the typed pattern as soon as it becomes meaningful so `:s/foo`
        // can highlight matches even before a replacement segment exists.
        let query = match SearchQuery::compile(&preview.pattern) {
            Ok(query) => query,
            Err(regex_error) => {
                self.clear_substitute_preview(true);
                self.show_status_message(format!("Invalid regex:\n{regex_error}"));
                return;
            }
        };

        // Search-only preview should not keep a previously centered viewport
        // from an older replacement preview once the input becomes incomplete.
        let original_viewport = self
            .substitute_preview
            .as_ref()
            .map_or(self.viewport, |active| active.original_viewport);
        self.viewport = original_viewport;
        self.substitute_preview = Some(SubstitutePreviewState::search(
            preview,
            query,
            original_viewport,
        ));
        self.refresh_substitute_preview_for_viewport();
        self.bump_substitute_preview_revision();
        self.clear_status_message();
    }

    /// Activate a replacement preview from one fully previewable substitute command.
    fn activate_ready_substitute_preview(&mut self, command: SubstituteCommand) {
        let plan = match build_substitute_plan(&command, &self.buffer, self.cursor.line()) {
            Ok(plan) => plan,
            Err(error) => {
                self.clear_substitute_preview(true);
                self.show_status_message(error);
                return;
            }
        };
        if plan.substitution_count() == 0 {
            self.clear_substitute_preview(true);
            self.clear_status_message();
            return;
        }

        // Preserve the original viewport across preview refreshes so Escape can
        // restore the same origin even after several intermediate edits.
        let original_viewport = self
            .substitute_preview
            .as_ref()
            .map_or(self.viewport, |preview| preview.original_viewport);
        let preview =
            SubstitutePreviewState::replace(command, plan, &self.buffer, original_viewport);

        // This mirrors Neovim's in-buffer inccommand behavior: once a real
        // replacement preview exists, keep the command-line cursor fixed but
        // recenter the viewport on the first changed region.
        if let Some(preview_cursor) = preview.first_preview_cursor() {
            self.viewport.align_cursor_center(
                &preview_cursor,
                preview.render_buffer().expect("replace preview buffer"),
            );
        }
        self.substitute_preview = Some(preview);
        self.refresh_substitute_preview_for_viewport();
        self.bump_substitute_preview_revision();
        self.clear_status_message();
    }

    /// Advance the redraw token for substitute preview state.
    ///
    /// Returns after moving to the next token value, wrapping back to zero if the
    /// counter reaches `u64::MAX` so future preview changes still force redraws.
    fn bump_substitute_preview_revision(&mut self) {
        self.substitute_preview_revision = self.substitute_preview_revision.wrapping_add(1);
    }
}

impl SubstitutePreviewState {
    /// Return the first preview cursor used for viewport recentering.
    fn first_preview_cursor(&self) -> Option<Cursor> {
        match &self.mode {
            SubstitutePreviewMode::Search { .. } => None,
            SubstitutePreviewMode::Replace {
                buffer,
                replacement_matches,
                ..
            } => replacement_matches
                .first()
                .map(|search_match| Cursor::from_char_index(buffer, search_match.start)),
        }
    }
}

/// Build the transient replacement buffer and final replacement highlight spans.
fn build_preview_buffer_and_matches(
    buffer: &TextBuffer,
    plan: &SubstitutePlan,
) -> (TextBuffer, Vec<SearchMatch>) {
    let mut preview_buffer = buffer.clone();
    let mut replacement_matches = Vec::with_capacity(plan.edits().len());
    let mut char_delta = 0isize;

    // Apply the real preview buffer from the end to preserve original
    // coordinates, while tracking final highlighted replacement spans by
    // replaying the same edits in forward order on character counts.
    for edit in plan.edits().iter().rev() {
        preview_buffer.remove(edit.start_char, edit.end_char);
        if !edit.replacement.is_empty() {
            preview_buffer.insert(edit.start_char, &edit.replacement);
        }
    }
    for edit in plan.edits() {
        let preview_start = edit.start_char.saturating_add_signed(char_delta);
        let preview_end = preview_start + edit.replacement.chars().count();
        replacement_matches.push(SearchMatch {
            start: preview_start,
            end: preview_end,
        });
        char_delta +=
            edit.replacement.chars().count() as isize - (edit.end_char - edit.start_char) as isize;
    }

    (preview_buffer, replacement_matches)
}

/// Return the visible character range covered by the current viewport.
fn visible_char_range_for_viewport(viewport: &Viewport, buffer: &TextBuffer) -> (usize, usize) {
    let line_count = buffer.lines_count();
    if line_count == 0 || viewport.height() == 0 {
        return (0, 0);
    }
    let first_line = viewport
        .first_visible_line()
        .min(line_count.saturating_sub(1));
    let last_line = first_line
        .saturating_add(viewport.height().saturating_sub(1))
        .min(line_count.saturating_sub(1));
    let start_char = buffer.line_to_char(first_line);
    let end_char = if last_line + 1 < line_count {
        buffer.line_to_char(last_line + 1)
    } else {
        buffer.chars_count()
    };
    (start_char, end_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refreshing a valid substitute command should activate replacement preview.
    #[test]
    fn test_refresh_substitute_preview_activates_replace_preview() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor
            .buffer_mut()
            .insert(0, "zero\none\nfoo line\nthree\n");

        editor.enter_command_prompt("%s/foo/bar");

        assert!(editor.substitute_preview_uses_render_buffer());
        assert_eq!(
            editor.render_buffer().to_string(),
            "zero\none\nbar line\nthree\n"
        );
        assert_eq!(editor.substitute_preview_line_spans(2).len(), 1);
    }

    /// Typing only the pattern should highlight matches without switching buffers.
    #[test]
    fn test_refresh_substitute_preview_highlights_search_part() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor.buffer_mut().insert(0, "foo one\nbar\nfoo two\n");

        editor.enter_command_prompt("%s/foo");

        assert!(!editor.substitute_preview_uses_render_buffer());
        assert_eq!(editor.substitute_preview_line_spans(0).len(), 1);
        assert_eq!(editor.substitute_preview_line_spans(2).len(), 1);
    }

    /// Incomplete substitute preview should keep trailing spaces inside the pattern span.
    #[test]
    fn test_refresh_substitute_preview_preserves_trailing_pattern_space() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor.buffer_mut().insert(0, "foo bar\n");

        editor.enter_command_prompt("s/foo ");

        assert!(!editor.substitute_preview_uses_render_buffer());
        assert_eq!(
            editor.substitute_preview_line_spans(0),
            &[SearchHighlightSpan {
                start_col: 0,
                end_col: 4,
            }]
        );
    }

    /// Wrapping the preview revision should still produce a different redraw token.
    #[test]
    fn test_clear_substitute_preview_wraps_revision_token() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor.buffer_mut().insert(0, "foo\n");
        editor.enter_command_prompt("s/foo");
        editor.substitute_preview_revision = u64::MAX;

        editor.clear_substitute_preview(false);

        assert_eq!(editor.substitute_preview_revision(), 0);
    }

    /// Invalid substitute preview should clear the active preview and restore the viewport.
    #[test]
    fn test_refresh_substitute_preview_invalid_regex_restores_viewport() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor.buffer_mut().insert(0, "alpha\nbeta\n");
        editor.enter_command_prompt("%s/alpha/bravo");
        let previewed_line = editor.viewport.first_visible_line();

        editor.replace_active_prompt_text("%s/(?=beta)/x/".to_string());

        assert!(editor.substitute_preview.is_none());
        assert_eq!(editor.viewport.first_visible_line(), 0);
        assert!(previewed_line >= editor.viewport.first_visible_line());
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Invalid regex:"))
        );
    }

    /// Canceling command input should drop substitute preview and restore the saved viewport.
    #[test]
    fn test_cancel_prompt_input_restores_viewport_after_preview() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor
            .buffer_mut()
            .insert(0, "zero\none\nfoo line\nthree\n");
        editor.enter_command_prompt("%s/foo/bar");
        let previewed_line = editor.viewport.first_visible_line();

        editor.cancel_prompt_input();

        assert!(editor.substitute_preview.is_none());
        assert!(editor.mode.is_normal());
        assert_eq!(editor.viewport.first_visible_line(), 0);
        assert!(previewed_line >= editor.viewport.first_visible_line());
    }

    /// Executing a previewed substitute should commit edits and keep the preview viewport.
    #[test]
    fn test_execute_command_commits_preview_without_restoring_viewport() {
        let mut editor = EditorState::new(8);
        editor.viewport.set_soft_wrap(false);
        editor
            .buffer_mut()
            .insert(0, "zero\none\nfoo line\nthree\n");
        editor.enter_command_prompt("%s/foo/bar");
        let previewed_line = editor.viewport.first_visible_line();

        editor.execute_command();

        assert!(editor.substitute_preview.is_none());
        assert_eq!(editor.buffer.to_string(), "zero\none\nbar line\nthree\n");
        assert_eq!(editor.viewport.first_visible_line(), previewed_line);
    }
}
