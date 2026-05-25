//! Indentation helpers for `EditorState`.

use super::*;
use crate::syntax::engine::LineLexMode;
use crate::syntax::profile::{CommentStyle, CommentStyleKind, IndentationConfig, IndentationStyle};
use crate::syntax::{HighlightSpan, SyntaxClass};

/// Inclusive logical-line range targeted by one indent command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndentLineRange {
    start_line: usize,
    end_line: usize,
}

/// Direction used by manual indentation-step commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndentDirection {
    Indent,
    Dedent,
}

impl IndentDirection {
    /// Return the target indentation width after one indentation step.
    fn apply(self, current_columns: usize, indent_width: usize) -> usize {
        match self {
            Self::Indent => current_columns.saturating_add(indent_width),
            Self::Dedent => current_columns.saturating_sub(indent_width),
        }
    }
}

/// Describe when one untouched auto-inserted prefix should be removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoInsertCleanupTrigger {
    Newline,
    Exit,
}

/// Prefix metadata used to continue one comment onto a newly inserted line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommentContinuation {
    target_column: usize,
    marker: String,
    spacing: String,
}

impl CommentContinuation {
    /// Build the exact text that should be inserted after `indent_column`.
    fn build_text(&self, indent_column: usize) -> String {
        format!(
            "{}{}{}",
            " ".repeat(self.target_column.saturating_sub(indent_column)),
            self.marker,
            self.spacing
        )
    }
}

impl EditorState {
    /// Reindent the current Visual selection and return to Normal mode.
    pub(super) fn reindent_visual_selection(&mut self) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };

        self.prepare_visual_repeat(saved_selection, SelectionRepeatAction::Reindent);
        self.last_visual_selection = Some(saved_selection);
        self.reindent_visual_selection_shape(selection);
        self.clear_visual_mode(Mode::Normal);
    }

    /// Indent the current Visual selection by one configured indentation step.
    pub(super) fn indent_visual_selection(&mut self) {
        self.change_visual_selection_indentation(IndentDirection::Indent);
    }

    /// Dedent the current Visual selection by one configured indentation step.
    pub(super) fn dedent_visual_selection(&mut self) {
        self.change_visual_selection_indentation(IndentDirection::Dedent);
    }

    /// Reindent one operator-resolved selection range.
    pub(super) fn reindent_selection(&mut self, selection: SelectionRange) {
        let Some(profile) = self.active_indentation_profile() else {
            self.show_status_message("No manual indent rule for current language");
            return;
        };
        let config = profile
            .indentation()
            .expect("indentation profile should carry indentation metadata");
        let line_range = self.indent_line_range(selection);
        let mut changed_any = false;

        // Reindent line-by-line inside one undo transaction so the whole command
        // replays, undoes, and redraws the same way as other editing operators.
        self.with_history_transaction(|editor| {
            for line_idx in line_range.start_line..=line_range.end_line {
                changed_any |= editor.reindent_one_line(line_idx, profile, config);
            }
            editor.move_cursor_to_first_non_blank(line_range.start_line);
        });

        if changed_any {
            self.clear_status_message();
        }
    }

    /// Reindent one resolved Visual selection using the matching line span.
    pub(super) fn reindent_visual_selection_shape(&mut self, selection: VisualSelection) {
        match selection {
            VisualSelection::Character(selection) | VisualSelection::Line(selection) => {
                self.reindent_selection(selection);
            }
            VisualSelection::Block(selection) => {
                self.reindent_selection(selection.line_selection_range(&self.buffer));
            }
        }
    }

    /// Adjust one selection's touched lines by one configured indentation step.
    pub(super) fn adjust_selection_indentation(
        &mut self,
        selection: SelectionRange,
        direction: IndentDirection,
    ) {
        let line_range = self.indent_line_range(selection);
        let mut changed_any = false;

        // Run the whole indent adjustment as one history transaction so commands
        // such as `>>` and Visual indent/dedent undo and replay as a single edit.
        self.with_history_transaction(|editor| {
            for line_idx in line_range.start_line..=line_range.end_line {
                changed_any |= editor.adjust_one_line_indentation(line_idx, direction);
            }
            editor.move_cursor_to_first_non_blank(line_range.start_line);
        });

        if changed_any {
            self.clear_status_message();
        }
    }

    /// Adjust one resolved Visual selection by one configured indentation step.
    pub(super) fn adjust_visual_selection_indentation(
        &mut self,
        selection: VisualSelection,
        direction: IndentDirection,
    ) {
        match selection {
            VisualSelection::Character(selection) | VisualSelection::Line(selection) => {
                self.adjust_selection_indentation(selection, direction);
            }
            VisualSelection::Block(selection) => {
                self.adjust_selection_indentation(
                    selection.line_selection_range(&self.buffer),
                    direction,
                );
            }
        }
    }

    /// Return how many logical lines one indent-style selection touches.
    pub(super) fn indentation_line_count(&self, selection: SelectionRange) -> usize {
        let line_range = self.indent_line_range(selection);
        line_range.end_line.saturating_sub(line_range.start_line) + 1
    }

    /// Insert one newline at the cursor and auto-indent the new line when supported.
    pub(super) fn insert_newline_with_auto_indent(&mut self) {
        let continuation = self.comment_continuation_for_current_line();
        self.cleanup_pending_auto_insert_line(AutoInsertCleanupTrigger::Newline);
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_line_idx = self.cursor.line() + 1;
        self.insert_buffer_text(char_idx, "\n");
        self.apply_auto_prefix_to_line(char_idx + 1, new_line_idx, continuation);
    }

    /// Open one line below the cursor, auto-indent it, and enter Insert mode.
    pub(super) fn open_line_below_with_auto_indent(&mut self) {
        let continuation = self.comment_continuation_for_current_line();
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.insert_buffer_text(line_end, "\n");
        self.apply_auto_prefix_to_line(line_end + 1, line + 1, continuation);
        self.enter_insert_mode();
    }

    /// Open one line above the cursor, auto-indent it, and enter Insert mode.
    pub(super) fn open_line_above_with_auto_indent(&mut self) {
        let continuation = self.comment_continuation_for_current_line();
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_start = self.buffer.line_to_char(line);
        self.insert_buffer_text(line_start, "\n");
        self.apply_auto_prefix_to_line(line_start, line, continuation);
        self.enter_insert_mode();
    }

    /// Remove one untouched auto-indent prefix before Insert mode exits.
    pub(super) fn cleanup_pending_auto_insert_on_exit(&mut self) {
        self.cleanup_pending_auto_insert_line(AutoInsertCleanupTrigger::Exit);
        self.pending_auto_insert = None;
    }

    /// Mark the tracked auto-indented blank line as touched by user edits.
    pub(super) fn touch_pending_auto_insert(&mut self) {
        if let Some(pending) = self.pending_auto_insert.as_mut()
            && pending.line == self.cursor.line()
        {
            pending.touched = true;
        }
    }

    /// Drop auto-indent cleanup tracking when the insert cursor leaves that line.
    pub(super) fn clear_pending_auto_insert_if_cursor_left_line(&mut self) {
        let should_clear = self.mode != Mode::Insert
            || self
                .pending_auto_insert
                .as_ref()
                .is_some_and(|pending| pending.line != self.cursor.line());
        if should_clear {
            self.pending_auto_insert = None;
        }
    }

    /// Return the active language profile when it exposes indentation metadata.
    fn active_indentation_profile(
        &self,
    ) -> Option<&'static crate::syntax::profile::LanguageProfile> {
        detect_language_details(Some(self.file_path.as_path())).map(|(profile, _)| profile)
    }

    /// Convert one character range into the inclusive logical lines it touches.
    fn indent_line_range(&self, selection: SelectionRange) -> IndentLineRange {
        let start_line = self
            .buffer
            .char_to_line(selection.start.min(self.buffer.chars_count()));

        // End positions are exclusive, so convert them back to the last covered
        // character before asking the buffer for its containing logical line.
        let end_line = if selection.end > selection.start {
            let last_char = selection
                .end
                .saturating_sub(1)
                .min(self.buffer.chars_count().saturating_sub(1));
            self.buffer.char_to_line(last_char)
        } else {
            start_line
        };
        IndentLineRange {
            start_line,
            end_line,
        }
    }

    /// Reindent one logical line according to the active style family.
    ///
    /// Returns `true` when the line's leading indentation changed, and `false`
    /// when the line was blank or already matched the desired indentation.
    fn reindent_one_line(
        &mut self,
        line_idx: usize,
        profile: &crate::syntax::profile::LanguageProfile,
        config: IndentationConfig,
    ) -> bool {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return false;
        };
        if line.trim().is_empty() {
            return false;
        }

        let current_indent_chars = leading_indent_char_count(&line);
        let target_indent_columns = self.target_indent_columns(line_idx, profile, config);
        let desired_indent = build_indent(
            target_indent_columns,
            self.settings.indent_width,
            self.settings.indent_with_tabs,
        );
        if line.starts_with(&desired_indent)
            && current_indent_chars == desired_indent.chars().count()
        {
            return false;
        }

        // The replacement only touches the leading indentation span so line
        // contents stay byte-for-byte identical after the prefix is rewritten.
        let line_start = self.buffer.line_to_char(line_idx);
        self.remove_buffer_range(line_start, line_start + current_indent_chars);
        self.insert_buffer_text(line_start, &desired_indent);
        true
    }

    /// Apply one language-aware indent prefix and optional comment continuation.
    fn apply_auto_prefix_to_line(
        &mut self,
        insert_char_idx: usize,
        line_idx: usize,
        continuation: Option<CommentContinuation>,
    ) {
        self.pending_auto_insert = None;
        let indent = self.auto_indent_prefix_for_line(line_idx);
        let indent_columns = indent_columns(&indent, self.settings.indent_width);
        let continuation_text = continuation
            .as_ref()
            .map(|continuation| continuation.build_text(indent_columns))
            .unwrap_or_default();
        let prefix = format!("{indent}{continuation_text}");
        if prefix.is_empty() {
            self.cursor = Cursor::from_char_index(&self.buffer, insert_char_idx);
            return;
        }

        // Insert the combined prefix in one step so the cursor and undo history
        // see one contiguous auto-generated region at the start of the new line.
        self.insert_buffer_text(insert_char_idx, &prefix);
        self.cursor =
            Cursor::from_char_index(&self.buffer, insert_char_idx + prefix.chars().count());
        self.remember_pending_auto_insert_line(self.cursor.line(), prefix, continuation.is_none());
    }

    /// Return the indentation prefix automatically inserted for `line_idx`.
    fn auto_indent_prefix_for_line(&self, line_idx: usize) -> String {
        let Some(profile) = self.active_indentation_profile() else {
            return String::new();
        };
        let Some(config) = profile.indentation() else {
            return String::new();
        };
        build_indent(
            self.target_indent_columns(line_idx, profile, config),
            self.settings.indent_width,
            self.settings.indent_with_tabs,
        )
    }

    /// Record one untouched auto-inserted prefix for later cleanup.
    fn remember_pending_auto_insert_line(
        &mut self,
        line_idx: usize,
        prefix: String,
        cleanup_on_newline: bool,
    ) {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            self.pending_auto_insert = None;
            return;
        };
        self.pending_auto_insert = (line == prefix).then_some(PendingAutoInsertLine {
            line: line_idx,
            prefix,
            cleanup_on_newline,
            touched: false,
        });
    }

    /// Remove one tracked auto-inserted prefix when the line stayed untouched.
    fn cleanup_pending_auto_insert_line(&mut self, trigger: AutoInsertCleanupTrigger) {
        let Some(pending) = self.pending_auto_insert.clone() else {
            return;
        };
        if pending.touched
            || pending.line != self.cursor.line()
            || (trigger == AutoInsertCleanupTrigger::Newline && !pending.cleanup_on_newline)
        {
            return;
        }

        // Cleanup only applies when the line still consists of the inserted
        // prefix and no later edits changed its contents.
        let Some(line) = self.buffer.line_for_display_string(pending.line) else {
            self.pending_auto_insert = None;
            return;
        };
        if line != pending.prefix {
            self.pending_auto_insert = None;
            return;
        }

        let line_start = self.buffer.line_to_char(pending.line);
        let prefix_end = line_start + pending.prefix.chars().count();
        self.remove_buffer_range(line_start, prefix_end);
        self.cursor = Cursor::new(pending.line, 0);
        self.pending_auto_insert = None;
    }

    /// Return the comment prefix that should continue on the next inserted line.
    fn comment_continuation_for_current_line(&self) -> Option<CommentContinuation> {
        let line_idx = self.cursor.line();
        let line = self.buffer.line_for_display_string(line_idx)?;
        let cursor_column = self.cursor.column().min(line.chars().count());
        let spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        let entry_mode = self
            .syntax
            .exact_entry_mode_for_line(&self.buffer, line_idx);
        self.block_comment_continuation(&line, cursor_column, &spans, entry_mode)
            .or_else(|| self.line_comment_continuation(&line, cursor_column, &spans, entry_mode))
    }

    /// Return one line-comment continuation that matches the current cursor context.
    fn line_comment_continuation(
        &self,
        line: &str,
        cursor_column: usize,
        spans: &[HighlightSpan],
        entry_mode: LineLexMode,
    ) -> Option<CommentContinuation> {
        if matches!(entry_mode, LineLexMode::BlockComment { .. })
            || !cursor_is_in_comment_context(spans, cursor_column, line.chars().count())
        {
            return None;
        }

        let mut best = None;
        for style in self
            .syntax
            .active_comment_styles()
            .iter()
            .copied()
            .filter(|style| style.kind == CommentStyleKind::Line)
        {
            best = better_line_comment_candidate(
                best,
                find_comment_token(line, cursor_column, spans, style),
            );
        }
        let (start_column, style) = best?;
        Some(CommentContinuation {
            target_column: start_column,
            marker: style.open.to_string(),
            spacing: spacing_after_marker(line, start_column, style.open),
        })
    }

    /// Return one block-comment continuation that matches the current cursor context.
    fn block_comment_continuation(
        &self,
        line: &str,
        cursor_column: usize,
        spans: &[HighlightSpan],
        entry_mode: LineLexMode,
    ) -> Option<CommentContinuation> {
        let line_len = line.chars().count();
        let anchor = block_comment_anchor(
            self.syntax.active_comment_styles(),
            line,
            cursor_column,
            spans,
            entry_mode,
        )?;
        let leader = inferred_block_comment_leader(anchor.style)?;
        let indent = leading_indent_char_count(line);
        let trimmed_start = first_non_whitespace_char_idx(line);
        let close = anchor
            .style
            .close
            .expect("block comments must define a closing delimiter");
        if text_matches_at(line, trimmed_start, close) {
            return None;
        }

        // Reuse an explicit interior leader when the line already has one, fall
        // back to the opener alignment on opener lines, and otherwise synthesize
        // the default leader column for blank or free-form block-comment rows.
        if text_matches_at(line, trimmed_start, &leader) {
            let spacing = spacing_after_marker(line, trimmed_start, &leader);
            return Some(CommentContinuation {
                target_column: trimmed_start,
                marker: leader,
                spacing,
            });
        }
        if let Some(open_start) = anchor.open_start {
            return Some(CommentContinuation {
                target_column: open_start + anchor.style.open.chars().count()
                    - leader.chars().count(),
                marker: leader,
                spacing: spacing_after_marker(line, open_start, anchor.style.open),
            });
        }
        if !matches!(entry_mode, LineLexMode::BlockComment { .. })
            && !cursor_is_in_comment_context(spans, cursor_column, line_len)
        {
            return None;
        }
        Some(CommentContinuation {
            target_column: indent + anchor.style.open.chars().count() - leader.chars().count(),
            marker: leader,
            spacing: String::from(" "),
        })
    }

    /// Indent the current insert-mode line by one configured shift width.
    pub(super) fn indent_current_line_insert_mode(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        // Rebuild the leading indent from the configured shift width so tabs and
        // spaces follow the same settings used by auto-indent.
        self.touch_pending_auto_insert();
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return;
        };
        let (current_chars, desired) = self.adjusted_indent_prefix(&line, IndentDirection::Indent);
        self.replace_current_line_indent(line_idx, current_chars, desired);
    }

    /// Dedent the current insert-mode line by one configured shift width.
    pub(super) fn dedent_current_line_insert_mode(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        // Clamp the target width at zero so repeated `Ctrl-D` stops once the line
        // is flush-left instead of producing negative indentation.
        self.touch_pending_auto_insert();
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return;
        };
        let (current_chars, desired) = self.adjusted_indent_prefix(&line, IndentDirection::Dedent);
        self.replace_current_line_indent(line_idx, current_chars, desired);
    }

    /// Recompute one insert-mode line after typing a closer or dedent keyword.
    pub(super) fn auto_dedent_current_line_after_insert(&mut self) {
        let Some(profile) = self.active_indentation_profile() else {
            return;
        };
        let Some(config) = profile.indentation() else {
            return;
        };
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return;
        };
        if !line_requests_auto_dedent(&line, profile, config) {
            return;
        }

        let current_indent_chars = leading_indent_char_count(&line);
        let current_indent_columns = indent_columns(&line, self.settings.indent_width);
        let desired_columns = self.target_indent_columns(line_idx, profile, config);
        if desired_columns >= current_indent_columns {
            return;
        }

        // Only rewrite the leading prefix when the language syntax marks this
        // line as an outdent trigger, so extra user-typed indent stays intact.
        let desired_indent = build_indent(
            desired_columns,
            self.settings.indent_width,
            self.settings.indent_with_tabs,
        );
        self.replace_current_line_indent(line_idx, current_indent_chars, desired_indent);
    }

    /// Adjust the active Visual selection's indentation and return to Normal mode.
    fn change_visual_selection_indentation(&mut self, direction: IndentDirection) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };

        let action = match direction {
            IndentDirection::Indent => SelectionRepeatAction::Indent,
            IndentDirection::Dedent => SelectionRepeatAction::Dedent,
        };
        self.prepare_visual_repeat(saved_selection, action);
        self.last_visual_selection = Some(saved_selection);
        self.adjust_visual_selection_indentation(selection, direction);
        self.clear_visual_mode(Mode::Normal);
    }

    /// Adjust one line's leading whitespace by one configured indentation step.
    ///
    /// Returns `true` when the line's indent prefix changed, and `false` when the
    /// line already matched the requested indentation level.
    fn adjust_one_line_indentation(&mut self, line_idx: usize, direction: IndentDirection) -> bool {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return false;
        };
        let (current_indent_chars, desired_indent) = self.adjusted_indent_prefix(&line, direction);
        if line.starts_with(&desired_indent)
            && current_indent_chars == desired_indent.chars().count()
        {
            return false;
        }

        // The replacement rewrites only the indentation prefix so non-indent text
        // stays byte-for-byte identical after manual indent adjustment.
        let line_start = self.buffer.line_to_char(line_idx);
        self.remove_buffer_range(line_start, line_start + current_indent_chars);
        self.insert_buffer_text(line_start, &desired_indent);
        true
    }

    /// Return the current indent span and the prefix after one indent adjustment.
    fn adjusted_indent_prefix(&self, line: &str, direction: IndentDirection) -> (usize, String) {
        let current_chars = leading_indent_char_count(line);
        let current_columns = indent_columns(line, self.settings.indent_width);
        let desired_columns = direction.apply(current_columns, self.settings.indent_width);
        let desired_indent = build_indent(
            desired_columns,
            self.settings.indent_width,
            self.settings.indent_with_tabs,
        );
        (current_chars, desired_indent)
    }

    /// Replace the current line's indent prefix and keep the insert cursor aligned.
    fn replace_current_line_indent(
        &mut self,
        line_idx: usize,
        current_indent_chars: usize,
        desired_indent: String,
    ) {
        let line_start = self.buffer.line_to_char(line_idx);
        let old_cursor = self.cursor.column();
        let desired_chars = desired_indent.chars().count();

        // Adjust the cursor by the indent delta so typed text stays attached to
        // the same logical content after the prefix changes.
        self.remove_buffer_range(line_start, line_start + current_indent_chars);
        self.insert_buffer_text(line_start, &desired_indent);
        if old_cursor <= current_indent_chars {
            // A cursor inside the old indent stays attached to the end of the new
            // indent so repeated `Ctrl-T`/`Ctrl-D` keeps it on the indentation edge.
            self.cursor.set_column(desired_chars);
        } else if desired_chars >= current_indent_chars {
            // Growing the indent shifts later text right, so preserve the cursor's
            // offset from the first non-indent character.
            self.cursor
                .set_column(old_cursor + (desired_chars - current_indent_chars));
        } else {
            // Shrinking the indent pulls later text left by the removed width.
            self.cursor
                .set_column(old_cursor - (current_indent_chars - desired_chars));
        }
    }

    /// Compute the target indentation width for one line.
    fn target_indent_columns(
        &self,
        line_idx: usize,
        profile: &crate::syntax::profile::LanguageProfile,
        config: IndentationConfig,
    ) -> usize {
        let previous_non_blank = self.previous_non_blank_line(line_idx).and_then(|index| {
            self.buffer
                .line_for_display_string(index)
                .map(|text| (index, text))
        });
        let current_line = self
            .buffer
            .line_for_display_string(line_idx)
            .unwrap_or_default();
        let mut target = previous_non_blank.as_ref().map_or(0, |(_, line)| {
            indent_columns(line, self.settings.indent_width)
        });

        // Each indentation family derives the base indent from the nearest
        // non-blank predecessor, then adjusts the current line relative to that
        // anchor according to the language's opening and closing cues.
        match config.style {
            IndentationStyle::CLike => {
                if previous_non_blank
                    .as_ref()
                    .is_some_and(|(_, line)| opens_c_like_block(line))
                {
                    target = target.saturating_add(self.settings.indent_width);
                }
                if starts_with_c_like_closer(&current_line) {
                    target = target.saturating_sub(self.settings.indent_width);
                }
                target
            }
            IndentationStyle::PythonLike => {
                if previous_non_blank
                    .as_ref()
                    .is_some_and(|(_, line)| opens_python_like_block(line))
                {
                    target = target.saturating_add(self.settings.indent_width);
                }
                if starts_with_python_dedent_keyword(&current_line, profile, config) {
                    target = target.saturating_sub(self.settings.indent_width);
                }
                target
            }
            IndentationStyle::PreviousLine => target,
        }
    }

    /// Return the nearest earlier non-blank logical line, if any.
    fn previous_non_blank_line(&self, line_idx: usize) -> Option<usize> {
        // Blank lines do not carry indentation intent, so walk upward until one
        // line with visible content can anchor the current line's target indent.
        (0..line_idx).rev().find(|candidate| {
            self.buffer
                .line_for_display_string(*candidate)
                .is_some_and(|line| !line.trim().is_empty())
        })
    }

    /// Move the cursor to the first non-blank column of `line_idx`.
    fn move_cursor_to_first_non_blank(&mut self, line_idx: usize) {
        self.cursor = Cursor::new(line_idx, 0);
        self.move_first_non_blank();
    }
}

/// One block-comment anchor found on the current line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockCommentAnchor {
    style: CommentStyle,
    open_start: Option<usize>,
}

/// Return the block-comment anchor relevant to the current line and cursor.
fn block_comment_anchor(
    styles: &[CommentStyle],
    line: &str,
    cursor_column: usize,
    spans: &[HighlightSpan],
    entry_mode: LineLexMode,
) -> Option<BlockCommentAnchor> {
    if let LineLexMode::BlockComment { style, .. } = entry_mode {
        return Some(BlockCommentAnchor {
            style,
            open_start: None,
        });
    }
    if !cursor_is_in_comment_context(spans, cursor_column, line.chars().count()) {
        return None;
    }

    let mut best = None;
    for style in styles
        .iter()
        .copied()
        .filter(|style| style.kind == CommentStyleKind::Block)
    {
        best = better_block_comment_candidate(
            best,
            find_comment_token(line, cursor_column, spans, style),
        );
    }
    best.map(|(open_start, style)| BlockCommentAnchor {
        style,
        open_start: Some(open_start),
    })
}

/// Return whether the cursor is inside a comment span or positioned at its end.
fn cursor_is_in_comment_context(
    spans: &[HighlightSpan],
    cursor_column: usize,
    line_len: usize,
) -> bool {
    if line_len == 0 {
        return false;
    }
    let target_column = cursor_column.min(line_len.saturating_sub(1));
    spans
        .iter()
        .find(|span| span.covers(target_column))
        .is_some_and(|span| span.class == SyntaxClass::Comment)
}

/// Return the better of two line-comment candidates.
fn better_line_comment_candidate(
    current: Option<(usize, CommentStyle)>,
    candidate: Option<(usize, CommentStyle)>,
) -> Option<(usize, CommentStyle)> {
    match (current, candidate) {
        (None, candidate) => candidate,
        (current, None) => current,
        (Some((current_start, current_style)), Some((candidate_start, candidate_style))) => {
            if candidate_start < current_start
                || (candidate_start == current_start
                    && candidate_style.open.chars().count() > current_style.open.chars().count())
            {
                Some((candidate_start, candidate_style))
            } else {
                Some((current_start, current_style))
            }
        }
    }
}

/// Return the better of two block-comment candidates.
fn better_block_comment_candidate(
    current: Option<(usize, CommentStyle)>,
    candidate: Option<(usize, CommentStyle)>,
) -> Option<(usize, CommentStyle)> {
    better_line_comment_candidate(current, candidate)
}

/// Return the comment token on `line` that matches `style` before `cursor_column`.
fn find_comment_token(
    line: &str,
    cursor_column: usize,
    spans: &[HighlightSpan],
    style: CommentStyle,
) -> Option<(usize, CommentStyle)> {
    let token_len = style.open.chars().count();
    let line_len = line.chars().count();
    if token_len == 0 || line_len < token_len {
        return None;
    }

    // Scan every start position up to the cursor so inline comment continuations
    // can align with the token that actually owns the cursor's comment region.
    for start_column in 0..=cursor_column.min(line_len.saturating_sub(token_len)) {
        if text_matches_at(line, start_column, style.open)
            && cursor_column >= start_column
            && spans
                .iter()
                .find(|span| span.covers(start_column))
                .is_some_and(|span| span.class == SyntaxClass::Comment)
        {
            return Some((start_column, style));
        }
    }
    None
}

/// Return whether `token` starts at `column` inside `line`.
fn text_matches_at(line: &str, column: usize, token: &str) -> bool {
    let mut suffix = line.chars().skip(column);
    token
        .chars()
        .all(|token_ch| suffix.next() == Some(token_ch))
}

/// Return the first non-whitespace character index in `line`.
fn first_non_whitespace_char_idx(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}

/// Return the exact whitespace that follows `marker`, or one space when absent.
fn spacing_after_marker(line: &str, start_column: usize, marker: &str) -> String {
    let spacing_start = start_column + marker.chars().count();
    let spacing_byte = char_to_byte_idx(line, spacing_start);
    let spacing_len = line[spacing_byte..]
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .count();
    if spacing_len == 0 {
        return String::from(" ");
    }
    let spacing_end = char_to_byte_idx(line, spacing_start + spacing_len);
    line[spacing_byte..spacing_end].to_string()
}

/// Return the shared overlap between the block open suffix and close prefix.
fn inferred_block_comment_leader(style: CommentStyle) -> Option<String> {
    let close = style.close?;
    let open_chars = style.open.chars().collect::<Vec<_>>();
    let close_chars = close.chars().collect::<Vec<_>>();
    let overlap_len = open_chars.len().min(close_chars.len());

    // Prefer the longest overlap so `<!-- -->` yields `--` while `/* */` yields `*`.
    for candidate_len in (1..=overlap_len).rev() {
        if open_chars[open_chars.len() - candidate_len..] == close_chars[..candidate_len] {
            return Some(
                open_chars[open_chars.len() - candidate_len..]
                    .iter()
                    .collect(),
            );
        }
    }
    None
}

/// Convert one character index inside `text` into its byte index.
fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(byte_idx, _)| byte_idx)
}

/// Return the number of leading indentation characters in `line`.
fn leading_indent_char_count(line: &str) -> usize {
    line.chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count()
}

/// Return the visual indentation width of the leading whitespace in `line`.
fn indent_columns(line: &str, indent_width: usize) -> usize {
    let mut columns = 0;

    // Leading tabs advance to the next configured indentation stop while spaces
    // advance by exactly one column.
    for ch in line.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => {
                let remainder = columns % indent_width;
                columns += if remainder == 0 {
                    indent_width
                } else {
                    indent_width - remainder
                };
            }
            _ => break,
        }
    }
    columns
}

/// Build one normalized indentation prefix for the configured output policy.
fn build_indent(columns: usize, indent_width: usize, indent_with_tabs: bool) -> String {
    if indent_with_tabs {
        let tabs = columns / indent_width;
        let spaces = columns % indent_width;
        return format!("{}{}", "\t".repeat(tabs), " ".repeat(spaces));
    }
    " ".repeat(columns)
}

/// Return whether `line` opens one brace-oriented block for the following line.
fn opens_c_like_block(line: &str) -> bool {
    line.trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '{' | '[' | '('))
}

/// Return whether `line` begins with one closing brace-oriented delimiter.
fn starts_with_c_like_closer(line: &str) -> bool {
    line.trim_start_matches([' ', '\t'])
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '}' | ']' | ')'))
}

/// Return whether `line` opens one colon-oriented block for the following line.
fn opens_python_like_block(line: &str) -> bool {
    line.trim_end().ends_with(':')
}

/// Return whether `line` should outdent relative to the preceding Python block.
fn starts_with_python_dedent_keyword(
    line: &str,
    profile: &crate::syntax::profile::LanguageProfile,
    config: IndentationConfig,
) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    config
        .dedent_keywords
        .iter()
        .any(|keyword| starts_with_complete_python_dedent_header(trimmed, keyword, profile))
}

/// Return whether `line` starts with `keyword` as a standalone token.
fn starts_with_keyword(
    line: &str,
    keyword: &str,
    profile: &crate::syntax::profile::LanguageProfile,
) -> bool {
    let Some(remainder) = line.strip_prefix(keyword) else {
        return false;
    };

    let Some(pattern) = profile.identifier else {
        return false;
    };
    remainder
        .chars()
        .next()
        .is_none_or(|ch| !identifier_can_continue(pattern, ch))
}

/// Return whether `line` starts with one complete Python dedent header.
fn starts_with_complete_python_dedent_header(
    line: &str,
    keyword: &str,
    profile: &crate::syntax::profile::LanguageProfile,
) -> bool {
    if !starts_with_keyword(line, keyword, profile) {
        return false;
    }

    // Python dedent headers become structurally complete only after their `:`,
    // so insert-mode auto-dedent waits for that terminator before rewriting indent.
    line.strip_prefix(keyword)
        .is_some_and(|remainder| remainder.contains(':'))
}

/// Return whether `line` is one insert-mode trigger that should auto-dedent.
fn line_requests_auto_dedent(
    line: &str,
    profile: &crate::syntax::profile::LanguageProfile,
    config: IndentationConfig,
) -> bool {
    match config.style {
        IndentationStyle::CLike => starts_with_c_like_closer(line),
        IndentationStyle::PythonLike => starts_with_python_dedent_keyword(line, profile, config),
        IndentationStyle::PreviousLine => false,
    }
}
