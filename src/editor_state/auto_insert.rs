//! Auto-insert and indentation helpers for `EditorState`.

use super::*;
use crate::indent::significant_last_char;
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
    ///
    /// Used by Normal and Visual mode operators (`>>`, `<<`), which shift by
    /// exactly `indent_width` regardless of current alignment.
    fn apply(self, current_columns: usize, indent_width: usize) -> usize {
        match self {
            Self::Indent => current_columns.saturating_add(indent_width),
            Self::Dedent => current_columns.saturating_sub(indent_width),
        }
    }

    /// Return the target indent column after one insert-mode step (Ctrl-T / Ctrl-D).
    ///
    /// Snaps to the nearest indent anchor (a multiple of `indent_width`) rather
    /// than shifting by a fixed amount:
    /// - `Indent`: advances to the next multiple of `indent_width` strictly
    ///   greater than `current_columns`. When `current_columns` is already a
    ///   multiple, advances by one full `indent_width`.
    /// - `Dedent`: retreats to the largest multiple of `indent_width` strictly
    ///   less than `current_columns`. When `current_columns` is already a
    ///   multiple, retreats by one full `indent_width`. Clamps at zero.
    fn apply_insert_mode(self, current_columns: usize, indent_width: usize) -> usize {
        match self {
            Self::Indent => {
                let remainder = current_columns % indent_width;
                // Snap to the next indent anchor by advancing to the next multiple
                // of indent_width strictly greater than current_columns.
                current_columns.saturating_add(indent_width - remainder)
            }
            Self::Dedent => {
                let remainder = current_columns % indent_width;
                // If perfectly aligned, step back a full stop; otherwise
                // round down to the nearest multiple by removing the overhang.
                if remainder == 0 {
                    current_columns.saturating_sub(indent_width)
                } else {
                    current_columns.saturating_sub(remainder)
                }
            }
        }
    }
}

/// Describe which auto-insert entry point is creating a new line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoInsertOperation {
    Newline,
    OpenBelow,
    OpenAbove,
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
    marker: &'static str,
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
            self.show_error_message("No manual indent rule for current language");
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
        let continuation = self.comment_continuation_for_current_line(AutoInsertOperation::Newline);
        self.cleanup_pending_auto_insert_line(AutoInsertCleanupTrigger::Newline);
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_line_idx = self.cursor.line() + 1;
        self.insert_buffer_text(
            char_idx,
            self.newline_payload_for_break_at(char_idx, continuation.as_ref()),
        );
        self.apply_auto_prefix_to_line(char_idx + 1, new_line_idx, continuation);
    }

    /// Open one line below the cursor, auto-indent it, and enter Insert mode.
    pub(super) fn open_line_below_with_auto_indent(&mut self) {
        let continuation =
            self.comment_continuation_for_current_line(AutoInsertOperation::OpenBelow);
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.insert_buffer_text(
            line_end,
            self.newline_payload_for_break_at(line_end, continuation.as_ref()),
        );
        self.apply_auto_prefix_to_line(line_end + 1, line + 1, continuation);
        self.enter_insert_mode();
    }

    /// Open one line above the cursor, auto-indent it, and enter Insert mode.
    pub(super) fn open_line_above_with_auto_indent(&mut self) {
        let continuation =
            self.comment_continuation_for_current_line(AutoInsertOperation::OpenAbove);
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_start = self.buffer.line_to_char(line);
        self.insert_buffer_text(
            line_start,
            self.newline_payload_for_break_at(line_start, continuation.as_ref()),
        );
        self.apply_auto_prefix_to_line(line_start, line, continuation);
        self.enter_insert_mode();
    }

    /// Return the exact newline payload needed for one Enter-style line break.
    fn newline_payload_for_break_at(
        &self,
        char_idx: usize,
        continuation: Option<&CommentContinuation>,
    ) -> &'static str {
        let last_char = self
            .buffer
            .char_at(self.buffer.chars_count().saturating_sub(1));

        // EOF-only breaks need one extra trailing newline only when no comment
        // continuation will populate the opened line. That preserves a visible
        // blank line after Escape without adding an extra empty line behind
        // continued comments or other non-empty auto-insert content.
        if char_idx == self.buffer.chars_count()
            && continuation.is_none()
            && !last_char.is_some_and(|ch| matches!(ch, '\n' | '\r'))
        {
            "\n\n"
        } else {
            "\n"
        }
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

    /// Return the insertion index after any block-comment closer spacing adjustment.
    pub(super) fn adjusted_insert_char_idx(&mut self, c: char) -> usize {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        if !c.is_ascii() || char_idx == 0 {
            return char_idx;
        }
        let Some(line) = self.buffer.line_for_display_string(self.cursor.line()) else {
            return char_idx;
        };
        if self.cursor.column() != line.chars().count() {
            return char_idx;
        }
        let trimmed = line.trim_start_matches([' ', '\t']);
        let entry_mode = self
            .syntax
            .exact_entry_mode_for_line(&self.buffer, self.cursor.line());
        let spans = self
            .syntax
            .compute_spans_for_line(&self.buffer, self.cursor.line());
        let Some(anchor) = block_comment_anchor(
            self.syntax.active_comment_styles(),
            &line,
            self.cursor.column(),
            &spans,
            entry_mode,
        ) else {
            return char_idx;
        };
        let Some(leader) = anchor.style.continue_with else {
            return char_idx;
        };
        let Some(close) = anchor.style.close else {
            return char_idx;
        };
        if close.as_bytes().last().copied() != Some(c as u8) || trimmed != format!("{leader} ") {
            return char_idx;
        }

        // Compact `* ` into `*` before typing the closing delimiter so ` */`
        // lands in one step instead of leaving the user with ` * /`.
        self.cursor = Cursor::from_char_index(&self.buffer, char_idx - 1);
        self.remove_buffer_range(char_idx - 1, char_idx);
        char_idx - 1
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
            self.cursor = Cursor::new(line_idx, 0);
            return;
        }

        // Insert the combined prefix in one step so the cursor and undo history
        // see one contiguous auto-generated region at the start of the new line.
        self.insert_buffer_text(insert_char_idx, &prefix);
        self.cursor =
            Cursor::from_char_index(&self.buffer, insert_char_idx + prefix.chars().count());
        self.remember_pending_auto_insert_line(self.cursor.line(), prefix);
    }

    /// Apply the language-aware indent prefix for `line_idx` without a comment
    /// continuation, positioning the cursor after the inserted whitespace.
    pub(super) fn apply_indent_prefix_to_line(&mut self, insert_char_idx: usize, line_idx: usize) {
        self.apply_auto_prefix_to_line(insert_char_idx, line_idx, None);
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
    fn remember_pending_auto_insert_line(&mut self, line_idx: usize, prefix: String) {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            self.pending_auto_insert = None;
            return;
        };
        self.pending_auto_insert = (line == prefix).then_some(PendingAutoInsertLine {
            line: line_idx,
            prefix,
            cleanup_on_exit: true,
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
            || (trigger == AutoInsertCleanupTrigger::Exit && !pending.cleanup_on_exit)
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
        let trimmed_prefix = pending.prefix.trim_end();
        let trimmed_prefix_end = line_start + trimmed_prefix.chars().count();
        self.remove_buffer_range(trimmed_prefix_end, prefix_end);
        self.cursor = Cursor::from_char_index(&self.buffer, trimmed_prefix_end);
        self.pending_auto_insert = None;
    }

    /// Return the comment prefix that should continue on the next inserted line.
    fn comment_continuation_for_current_line(
        &self,
        operation: AutoInsertOperation,
    ) -> Option<CommentContinuation> {
        let line_idx = self.cursor.line();
        let line = self.buffer.line_for_display_string(line_idx)?;
        let cursor_column = self.cursor.column().min(line.chars().count());
        let spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        let entry_mode = self
            .syntax
            .exact_entry_mode_for_line(&self.buffer, line_idx);
        self.block_comment_continuation(&line, cursor_column, &spans, entry_mode, operation)
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
            best = better_comment_candidate(
                best,
                find_comment_token(line, cursor_column, spans, style),
            );
        }
        let best = best?;
        Some(CommentContinuation {
            target_column: best.start_column,
            marker: best.style.open,
            spacing: spacing_after_marker(line, best.start_byte, best.style.open),
        })
    }

    /// Return one block-comment continuation that matches the current cursor context.
    fn block_comment_continuation(
        &self,
        line: &str,
        cursor_column: usize,
        spans: &[HighlightSpan],
        entry_mode: LineLexMode,
        operation: AutoInsertOperation,
    ) -> Option<CommentContinuation> {
        if operation == AutoInsertOperation::OpenAbove {
            return None;
        }
        let line_len = line.chars().count();
        let anchor = block_comment_anchor(
            self.syntax.active_comment_styles(),
            line,
            cursor_column,
            spans,
            entry_mode,
        )?;
        let leader = anchor.style.continue_with?;
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
        if text_matches_at(line, trimmed_start, leader) {
            let spacing =
                spacing_after_marker(line, leading_ascii_whitespace_byte_count(line), leader);
            return Some(CommentContinuation {
                target_column: trimmed_start,
                marker: leader,
                spacing,
            });
        }
        if let Some(open_start) = anchor.open_start {
            // When the closing delimiter also appears on this line after the opener,
            // the block comment is self-contained on a single line.
            let after_open_byte = open_start.start_byte + anchor.style.open.len();
            if let Some(close_byte_offset) = line[after_open_byte..].find(close) {
                // `o` opens a line after the current one, which is always outside
                // the now-closed comment regardless of where the cursor sits.
                if operation == AutoInsertOperation::OpenBelow {
                    return None;
                }
                // For Enter (splitting the line), only continue the comment when
                // the cursor is at or before the start of the closing delimiter.
                // In Insert mode the cursor is a bar that sits *before* the
                // character at cursor_column, so a cursor at close_start_column
                // inserts the newline before `*/`, leaving the left half without
                // a closing delimiter — continuation is appropriate.
                // Only a cursor strictly past close_start_column (i.e. between
                // or after the characters of `*/`) must not produce a continuation.
                // Likewise, a cursor on or before the opener is outside the body.
                let open_end_column = open_start.start_column + anchor.style.open.chars().count();
                let close_start_column =
                    line[..after_open_byte + close_byte_offset].chars().count();
                if cursor_column < open_end_column || cursor_column > close_start_column {
                    return None;
                }
            }
            return Some(CommentContinuation {
                target_column: open_start.start_column + anchor.style.open.chars().count()
                    - leader.chars().count(),
                marker: leader,
                spacing: spacing_after_marker(line, open_start.start_byte, anchor.style.open),
            });
        }
        // Reaching the fallback means the line does not expose an opener token or
        // a visible interior leader. Comment highlighting alone is not enough at
        // that point because a closing line such as ` */` is still highlighted as
        // comment, yet continuing it would leak the block leader outside the
        // comment. Only an inherited BlockComment entry mode proves the cursor is
        // still in the carried body of the block, so keep this guard at the last
        // fallback step after the opener/leader cases above have had a chance.
        if !matches!(entry_mode, LineLexMode::BlockComment { .. })
            && !cursor_is_in_comment_context(spans, cursor_column, line_len)
        {
            return None;
        }
        Some(CommentContinuation {
            target_column: trimmed_start + anchor.style.open.chars().count()
                - leader.chars().count(),
            marker: leader,
            spacing: String::from(" "),
        })
    }

    /// Indent the current insert-mode line by one configured shift width.
    pub(super) fn indent_current_line_insert_mode(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        // Snap to the next indent anchor so the resulting column is always a
        // multiple of indent_width, matching Vim's Ctrl-T behaviour.
        self.touch_pending_auto_insert();
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return;
        };
        let (current_chars, desired) =
            self.adjusted_insert_mode_prefix(&line, IndentDirection::Indent);
        self.replace_current_line_indent(line_idx, current_chars, desired);
    }

    /// Dedent the current insert-mode line by one configured shift width.
    pub(super) fn dedent_current_line_insert_mode(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        // Snap to the previous indent anchor so the resulting column is always a
        // multiple of indent_width, clamping at zero to avoid negative indentation.
        self.touch_pending_auto_insert();
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return;
        };
        let (current_chars, desired) =
            self.adjusted_insert_mode_prefix(&line, IndentDirection::Dedent);
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
        // Blank lines stay untouched so manual indent operators never insert
        // whitespace on empty rows, matching Vim's behaviour.
        if line.trim().is_empty() {
            return false;
        }
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
    ///
    /// Used by Normal and Visual mode operators that shift by exactly
    /// `indent_width` regardless of current column alignment.
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

    /// Return the current indent span and the snapped prefix for Ctrl-T or Ctrl-D.
    ///
    /// Advances/retreats to the next/previous indent anchor (next multiple of `indent_width`
    /// strictly greater than the current column count, or largest multiple of `indent_width`
    /// strictly less than the current column count, clamped at zero).
    fn adjusted_insert_mode_prefix(
        &self,
        line: &str,
        direction: IndentDirection,
    ) -> (usize, String) {
        let current_chars = leading_indent_char_count(line);
        let current_columns = indent_columns(line, self.settings.indent_width);
        let desired_columns =
            direction.apply_insert_mode(current_columns, self.settings.indent_width);
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
                if let Some((anchor_idx, ref anchor_line)) = previous_non_blank {
                    let anchor_spans = self.syntax.compute_spans_for_line(&self.buffer, anchor_idx);
                    let anchor_indent = indent_columns(anchor_line, self.settings.indent_width);
                    let mut previous_same_indent_anchor_storage = Vec::new();
                    let mut enclosing_less_indent_anchor_storage = None;
                    let mut search_idx = anchor_idx;
                    // Capture up to two earlier same-indentation anchors while
                    // skipping deeper nested body lines above the current anchor.
                    while let Some(prev_idx) = self.previous_non_blank_line(search_idx) {
                        let Some(prev_line) = self.buffer.line_for_display_string(prev_idx) else {
                            break;
                        };
                        let prev_indent = indent_columns(&prev_line, self.settings.indent_width);
                        if prev_indent > anchor_indent {
                            search_idx = prev_idx;
                            continue;
                        }
                        if prev_indent < anchor_indent {
                            let prev_spans =
                                self.syntax.compute_spans_for_line(&self.buffer, prev_idx);
                            enclosing_less_indent_anchor_storage = Some((prev_line, prev_spans));
                            break;
                        }
                        let prev_spans = self.syntax.compute_spans_for_line(&self.buffer, prev_idx);
                        previous_same_indent_anchor_storage.push((prev_line, prev_spans));
                        if previous_same_indent_anchor_storage.len() == 2 {
                            break;
                        }
                        search_idx = prev_idx;
                    }
                    let previous_same_indent_anchors = previous_same_indent_anchor_storage
                        .iter()
                        .map(|(line, spans)| (line.as_str(), spans.as_slice()))
                        .collect::<Vec<_>>();
                    let enclosing_less_indent_anchor = enclosing_less_indent_anchor_storage
                        .as_ref()
                        .map(|(line, spans)| (line.as_str(), spans.as_slice()));

                    if opens_c_like_block(anchor_line, &anchor_spans)
                        || line_has_unmatched_open_delimiter(anchor_line, &anchor_spans)
                    {
                        // The anchor line opens a block or has an unmatched `(`/`[`:
                        // the current line is the first indented body line.
                        target = target.saturating_add(self.settings.indent_width);
                    } else if line_is_continuation(anchor_line, &anchor_spans)
                        && !starts_with_c_like_closer(&current_line)
                        && !crate::indent::skip_c_like_continuation_indent_after_trailing_comma(
                            anchor_line,
                            &anchor_spans,
                            &previous_same_indent_anchors,
                            enclosing_less_indent_anchor,
                            profile,
                        )
                    {
                        // The anchor is an unterminated (continuation) statement.
                        // Add one extra level only when the anchor is not already
                        // itself at continuation-indent level, to prevent stacking.
                        let anchor_already_continuation = self
                            .previous_non_blank_line(anchor_idx)
                            .is_some_and(|prev_idx| {
                                self.buffer.line_for_display_string(prev_idx).is_some_and(
                                    |prev_line| {
                                        let prev_spans = self
                                            .syntax
                                            .compute_spans_for_line(&self.buffer, prev_idx);
                                        line_is_continuation(&prev_line, &prev_spans)
                                    },
                                )
                            });
                        if !anchor_already_continuation {
                            target = target.saturating_add(self.settings.indent_width);
                        }
                    } else if line_is_terminated(anchor_line, &anchor_spans) {
                        if line_is_block_closer_terminated(anchor_line, &anchor_spans) {
                            if let Some(head_indent) = self
                                .continuation_head_indent_for_block_closer_anchor(
                                    anchor_idx, target,
                                )
                            {
                                target = head_indent;
                            }
                        } else {
                            // The anchor is a terminated statement.  Walk further back
                            // through any unterminated (continuation) lines to find the
                            // head of the statement, mirroring the upward
                            // terminated-line lookup used by this engine:
                            // scan: each unterminated predecessor overwrites `target`
                            // with its own indent until a terminated line is reached.
                            let mut search_idx = anchor_idx;
                            while let Some(prev_idx) = self.previous_non_blank_line(search_idx) {
                                let Some(prev_line) = self.buffer.line_for_display_string(prev_idx)
                                else {
                                    break;
                                };
                                let prev_spans =
                                    self.syntax.compute_spans_for_line(&self.buffer, prev_idx);
                                if line_is_continuation(&prev_line, &prev_spans) {
                                    // Unterminated predecessor: adopt its indent and
                                    // keep scanning upward for an earlier head.
                                    target = indent_columns(&prev_line, self.settings.indent_width);
                                    search_idx = prev_idx;
                                } else {
                                    // Another terminated line or a block opener: the
                                    // head of the continuation is already captured.
                                    break;
                                }
                            }
                        }
                    }
                }
                // A closing delimiter on the current line removes one level of
                // indent relative to the computed target.  This is applied after
                // the anchor-based adjustments so that a `}` on the body line
                // of a block opener correctly cancels out the one level added
                // by the opener check above.
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

    /// Return one continuation-head indent for a block-closer terminated anchor.
    fn continuation_head_indent_for_block_closer_anchor(
        &self,
        anchor_idx: usize,
        anchor_indent: usize,
    ) -> Option<usize> {
        let mut search_idx = anchor_idx;
        let mut skipped_same_indent_opener = false;
        while let Some(prev_idx) = self.previous_non_blank_line(search_idx) {
            let Some(prev_line) = self.buffer.line_for_display_string(prev_idx) else {
                break;
            };
            let prev_indent = indent_columns(&prev_line, self.settings.indent_width);
            let prev_spans = self.syntax.compute_spans_for_line(&self.buffer, prev_idx);

            // Ignore lines indented more deeply than the closer. Those lines
            // belong to the body that has already been closed by the anchor.
            if prev_indent > anchor_indent {
                search_idx = prev_idx;
                continue;
            }

            // Skip the block opener at the same indentation level so a closer
            // such as `};` can resolve to an earlier continuation head.
            if prev_indent == anchor_indent && opens_c_like_block(&prev_line, &prev_spans) {
                skipped_same_indent_opener = true;
                search_idx = prev_idx;
                continue;
            }

            // The first continuation line at-or-left of the closer is the head
            // that should own the next-line indentation after `};` / `});`.
            if line_is_continuation(&prev_line, &prev_spans) {
                // When the closed block uses a standalone `{` at the same
                // indentation level, the immediately preceding continuation line
                // (for example `match value`) belongs to that just-closed block.
                // Keep scanning to find the outer owning head for the next line.
                if skipped_same_indent_opener && prev_indent == anchor_indent {
                    search_idx = prev_idx;
                    continue;
                }
                return Some(prev_indent);
            }

            // A terminated predecessor ends the scan boundary.
            if line_is_terminated(&prev_line, &prev_spans) {
                break;
            }

            search_idx = prev_idx;
        }
        None
    }

    /// Return the nearest earlier non-blank logical line, if any.
    fn previous_non_blank_line(&self, line_idx: usize) -> Option<usize> {
        // Blank lines do not carry indentation intent, so walk upward until one
        // non-comment line with visible content can anchor the current line's
        // target indent. Pure comment lines are skipped so a block-comment body
        // does not push the cursor deeper once insertion continues after `*/`.
        (0..line_idx).rev().find(|candidate| {
            self.buffer
                .line_for_display_string(*candidate)
                .is_some_and(|line| !line.trim().is_empty())
                && !self.line_is_comment_only(*candidate)
        })
    }

    /// Move the cursor to the first non-blank column of `line_idx`.
    fn move_cursor_to_first_non_blank(&mut self, line_idx: usize) {
        self.cursor = Cursor::new(line_idx, 0);
        self.move_first_non_blank();
    }

    /// Return whether `line_idx` contains only comment text apart from whitespace.
    fn line_is_comment_only(&self, line_idx: usize) -> bool {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return false;
        };
        if line.trim().is_empty() {
            return false;
        }
        let spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        // Comment-only lines should not become indentation anchors for the next
        // inserted code line after the cursor leaves the comment block.
        line.chars().enumerate().all(|(column, ch)| {
            ch.is_whitespace()
                || spans
                    .iter()
                    .find(|span| span.covers(column))
                    .is_some_and(|span| span.class == SyntaxClass::Comment)
        })
    }
}

/// One block-comment anchor found on the current line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockCommentAnchor {
    style: CommentStyle,
    open_start: Option<CommentTokenMatch>,
}

/// One matched comment token on the current line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommentTokenMatch {
    start_column: usize,
    start_byte: usize,
    style: CommentStyle,
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
        best =
            better_comment_candidate(best, find_comment_token(line, cursor_column, spans, style));
    }
    best.map(|open_start| BlockCommentAnchor {
        style: open_start.style,
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

/// Return the better of two comment-token candidates.
fn better_comment_candidate(
    current: Option<CommentTokenMatch>,
    candidate: Option<CommentTokenMatch>,
) -> Option<CommentTokenMatch> {
    match (current, candidate) {
        (None, candidate) => candidate,
        (current, None) => current,
        (Some(current), Some(candidate)) => {
            if candidate.start_column < current.start_column
                || (candidate.start_column == current.start_column
                    && candidate.style.open.chars().count() > current.style.open.chars().count())
            {
                Some(candidate)
            } else {
                Some(current)
            }
        }
    }
}

/// Return the comment token on `line` that matches `style` before `cursor_column`.
fn find_comment_token(
    line: &str,
    cursor_column: usize,
    spans: &[HighlightSpan],
    style: CommentStyle,
) -> Option<CommentTokenMatch> {
    let token = style.open.as_bytes();
    if token.is_empty() || line.len() < token.len() {
        return None;
    }
    let cursor_byte = byte_idx_for_column(line, cursor_column);

    // Scan every start position up to the cursor so inline comment continuations
    // can align with the token that actually owns the cursor's comment region.
    for start_byte in 0..=cursor_byte.min(line.len().saturating_sub(token.len())) {
        if line.as_bytes()[start_byte..].starts_with(token)
            && is_char_boundary_or_eof(line, start_byte)
            && is_char_boundary_or_eof(line, start_byte + token.len())
        {
            let start_column = line[..start_byte].chars().count();
            if cursor_column < start_column {
                continue;
            }
            if spans
                .iter()
                .find(|span| span.covers(start_column))
                .is_some_and(|span| span.class == SyntaxClass::Comment)
            {
                return Some(CommentTokenMatch {
                    start_column,
                    start_byte,
                    style,
                });
            }
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

/// Return the first non-whitespace byte index in `line`.
fn leading_ascii_whitespace_byte_count(line: &str) -> usize {
    line.as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count()
}

/// Return the exact whitespace that follows `marker`, or one space when absent.
fn spacing_after_marker(line: &str, start_byte: usize, marker: &str) -> String {
    let spacing_start = start_byte + marker.len();
    let bytes = line.as_bytes();
    let mut spacing_end = spacing_start;
    while spacing_end < bytes.len() && bytes[spacing_end].is_ascii_whitespace() {
        spacing_end += 1;
    }
    if spacing_end == spacing_start {
        return String::from(" ");
    }
    line[spacing_start..spacing_end].to_string()
}

/// Return whether `byte_idx` sits on a character boundary or at EOF.
fn is_char_boundary_or_eof(text: &str, byte_idx: usize) -> bool {
    byte_idx == text.len() || text.is_char_boundary(byte_idx)
}

/// Convert one display column into its UTF-8 byte index inside `text`.
fn byte_idx_for_column(text: &str, column: usize) -> usize {
    text.char_indices()
        .nth(column)
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

/// Return whether `line` opens one `{`-delimited block for the following line.
///
/// Returns `true` when the last significant character of the line is `{`;
/// returns `false` otherwise.
fn opens_c_like_block(line: &str, spans: &[HighlightSpan]) -> bool {
    significant_last_char(line, spans) == Some('{')
}

/// Return whether `line` has more unmatched opening `(` or `[` than closing
/// ones, considering only characters outside strings and comments.
///
/// Returns `true` when at least one `(` or `[` is left unmatched after
/// scanning the full line; returns `false` when every opener is paired with a
/// closer or there are no openers at all.
fn line_has_unmatched_open_delimiter(line: &str, spans: &[HighlightSpan]) -> bool {
    let mut depth: i32 = 0;
    for (byte_off, ch) in line.char_indices() {
        let col = line[..byte_off].chars().count();
        // Skip characters inside strings or comments.
        let is_code = spans
            .iter()
            .find(|span| span.covers(col))
            .is_none_or(|span| !matches!(span.class, SyntaxClass::Comment | SyntaxClass::String));
        if !is_code {
            continue;
        }
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// Return whether `line` is a terminated statement in C-like syntax.
///
/// Trailing line-comment text is stripped first so that a comment after a
/// terminator (e.g. `let x = 1; // note`) does not mask the terminator.
///
/// A line is considered terminated when its last significant character is
/// `;` or `}`.  Lines ending with `{` are block-openers handled separately.
///
/// Returns `true` for terminated lines; returns `false` for unterminated
/// (continuation) lines.
fn line_is_terminated(line: &str, spans: &[HighlightSpan]) -> bool {
    matches!(significant_last_char(line, spans), Some(';' | '}'))
}

/// Return whether `line` is terminated by a closing block brace.
///
/// Returns `true` when the right edge of the significant text resolves to a
/// `}` block closer, optionally followed by suffix closers/terminators such as
/// `)`, `]`, or `;`; returns `false` for every other terminator or non-terminator.
fn line_is_block_closer_terminated(line: &str, spans: &[HighlightSpan]) -> bool {
    for (byte_off, ch) in line.char_indices().rev() {
        let col = line[..byte_off].chars().count();
        // Ignore whitespace and trailing comment text so suffix checks only
        // inspect significant code characters.
        let is_significant = !ch.is_whitespace()
            && !spans
                .iter()
                .any(|span| span.class == SyntaxClass::Comment && span.covers(col));
        if !is_significant {
            continue;
        }
        // Suffix closers/terminators can trail a block closer without changing
        // the underlying "this line closes a block" intent.
        if matches!(ch, ';' | ')' | ']') {
            continue;
        }
        return ch == '}';
    }
    false
}

/// Return whether `line` is an unterminated (continuation) statement.
///
/// A line is terminated only when it ends with one explicit terminator:
/// ends with `;`, `}`, or `{`.  Everything else — identifiers, closing
/// delimiters `)` `]`, operators, commas — is unterminated and continues
/// on the next line.  Unmatched-delimiter cases (e.g. `[10,` or `call(10,`)
/// are handled separately by `line_has_unmatched_open_delimiter`.
///
/// Returns `true` when the next line should receive an extra continuation
/// indent level; returns `false` otherwise.
fn line_is_continuation(line: &str, spans: &[HighlightSpan]) -> bool {
    !matches!(
        significant_last_char(line, spans),
        None | Some(';' | '}' | '{')
    )
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

    let pattern = profile.identifier;
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

#[cfg(test)]
mod tests {
    use super::IndentDirection;

    /// Helper to run one indent-mode case with a readable failure message.
    #[track_caller]
    fn check(direction: IndentDirection, current: usize, width: usize, expected: usize) {
        let got = direction.apply_insert_mode(current, width);
        assert_eq!(
            got, expected,
            "{direction:?} from column {current} with indent_width={width}: \
             expected {expected}, got {got}"
        );
    }

    // --- indent_width = 4, Indent direction ---

    /// Ctrl-T from column 0 (aligned) advances to the first indent stop.
    #[test]
    fn insert_mode_indent_from_zero_aligned() {
        check(IndentDirection::Indent, 0, 4, 4);
    }

    /// Ctrl-T from column 4 (aligned) advances to the next full stop.
    #[test]
    fn insert_mode_indent_from_four_aligned() {
        check(IndentDirection::Indent, 4, 4, 8);
    }

    /// Ctrl-T from column 8 (aligned) advances to the next full stop.
    #[test]
    fn insert_mode_indent_from_eight_aligned() {
        check(IndentDirection::Indent, 8, 4, 12);
    }

    /// Ctrl-T from column 1 (misaligned) snaps to the next anchor at 4.
    #[test]
    fn insert_mode_indent_from_one_misaligned() {
        check(IndentDirection::Indent, 1, 4, 4);
    }

    /// Ctrl-T from column 5 (misaligned) snaps to the next anchor at 8.
    #[test]
    fn insert_mode_indent_from_five_misaligned() {
        check(IndentDirection::Indent, 5, 4, 8);
    }

    /// Ctrl-T from column 7 (misaligned) snaps to the next anchor at 8.
    #[test]
    fn insert_mode_indent_from_seven_misaligned() {
        check(IndentDirection::Indent, 7, 4, 8);
    }

    // --- indent_width = 4, Dedent direction ---

    /// Ctrl-D from column 4 (aligned) retreats to 0.
    #[test]
    fn insert_mode_dedent_from_four_aligned() {
        check(IndentDirection::Dedent, 4, 4, 0);
    }

    /// Ctrl-D from column 8 (aligned) retreats by one full stop.
    #[test]
    fn insert_mode_dedent_from_eight_aligned() {
        check(IndentDirection::Dedent, 8, 4, 4);
    }

    /// Ctrl-D from column 1 (misaligned) snaps down to 0.
    #[test]
    fn insert_mode_dedent_from_one_misaligned() {
        check(IndentDirection::Dedent, 1, 4, 0);
    }

    /// Ctrl-D from column 5 (misaligned) snaps down to the previous anchor at 4.
    #[test]
    fn insert_mode_dedent_from_five_misaligned() {
        check(IndentDirection::Dedent, 5, 4, 4);
    }

    /// Ctrl-D from column 7 (misaligned) snaps down to the previous anchor at 4.
    #[test]
    fn insert_mode_dedent_from_seven_misaligned() {
        check(IndentDirection::Dedent, 7, 4, 4);
    }

    /// Ctrl-D from column 0 stays at 0 and does not wrap around.
    #[test]
    fn insert_mode_dedent_from_zero_clamps() {
        check(IndentDirection::Dedent, 0, 4, 0);
    }

    // --- indent_width = 2 ---

    /// Ctrl-T from column 3 (misaligned, width=2) snaps up to 4.
    #[test]
    fn insert_mode_indent_width_two_from_three_misaligned() {
        check(IndentDirection::Indent, 3, 2, 4);
    }

    /// Ctrl-D from column 3 (misaligned, width=2) snaps down to 2.
    #[test]
    fn insert_mode_dedent_width_two_from_three_misaligned() {
        check(IndentDirection::Dedent, 3, 2, 2);
    }

    /// Ctrl-D from column 2 (aligned, width=2) retreats to 0.
    #[test]
    fn insert_mode_dedent_width_two_from_two_aligned() {
        check(IndentDirection::Dedent, 2, 2, 0);
    }

    // --- indent_width = 3 ---

    /// Ctrl-T from column 5 (misaligned, width=3) snaps up to 6.
    #[test]
    fn insert_mode_indent_width_three_from_five_misaligned() {
        check(IndentDirection::Indent, 5, 3, 6);
    }

    /// Ctrl-D from column 5 (misaligned, width=3) snaps down to 3.
    #[test]
    fn insert_mode_dedent_width_three_from_five_misaligned() {
        check(IndentDirection::Dedent, 5, 3, 3);
    }
}
