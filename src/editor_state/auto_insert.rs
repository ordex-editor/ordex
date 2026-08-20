//! Auto-insert and indentation helpers for `EditorState`.

use super::*;
use crate::indent::scope;
use crate::syntax::engine::{BracketFrame, LineLexMode};
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
            // Resolve the enclosing brackets once and carry them forward across
            // the range, so the scan cost is paid for the whole command instead
            // of being repeated for every line.
            let mut enclosing_brackets = editor
                .syntax
                .enclosing_bracket_stack(&editor.buffer, line_range.start_line);
            for line_idx in line_range.start_line..=line_range.end_line {
                changed_any |=
                    editor.reindent_one_line(line_idx, &enclosing_brackets, profile, config);
                // Reindenting rewrites leading whitespace only, so this line's
                // brackets still describe the state entering the next line.
                editor.syntax.advance_bracket_stack(
                    &editor.buffer,
                    line_idx,
                    &mut enclosing_brackets,
                );
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
        enclosing_brackets: &[BracketFrame],
        profile: &crate::syntax::profile::LanguageProfile,
        config: IndentationConfig,
    ) -> bool {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return false;
        };
        if line.trim().is_empty() {
            return false;
        }
        let spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        let entry_mode = self
            .syntax
            .exact_entry_mode_for_line(&self.buffer, line_idx);
        if crate::indent::skip_reindent_prefix_rewrite(&line, &spans, entry_mode) {
            return false;
        }

        let leader_offset = self.block_comment_reindent_leader_offset_columns(&line, &spans);
        // A line whose text continues a block comment carries the comment author's
        // own indentation, and reindent has no code anchor inside the comment to
        // align it against. Preserve such lines, except the comment leader and
        // closer lines, which reindent normalizes to a one-space offset.
        if matches!(entry_mode, LineLexMode::BlockComment { .. }) && leader_offset == 0 {
            return false;
        }

        let current_indent_chars = leading_indent_char_count(&line);
        let target_indent_columns = self
            .target_indent_columns(line_idx, enclosing_brackets, profile, config)
            .saturating_add(leader_offset);
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

    /// Return the extra indent columns required for block-comment leader lines.
    ///
    /// Returns `1` when the first non-whitespace token belongs to one block
    /// comment leader (`continue_with`) or one block comment closer (`close`);
    /// returns `0` for every other line shape.
    fn block_comment_reindent_leader_offset_columns(
        &self,
        line: &str,
        spans: &[HighlightSpan],
    ) -> usize {
        let token_column = first_non_whitespace_char_idx(line);
        // Reindent offset applies only when the visible token is classified as
        // comment syntax, so code tokens that happen to match marker text stay unchanged.
        if !spans
            .iter()
            .find(|span| span.covers(token_column))
            .is_some_and(|span| span.class == SyntaxClass::Comment)
        {
            return 0;
        }
        for style in self
            .syntax
            .active_comment_styles()
            .iter()
            .copied()
            .filter(|style| style.kind == CommentStyleKind::Block)
        {
            if style
                .continue_with
                .is_some_and(|leader| text_matches_at(line, token_column, leader))
                || style
                    .close
                    .is_some_and(|close| text_matches_at(line, token_column, close))
            {
                return 1;
            }
        }
        0
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
        // A single inserted line resolves its own enclosing brackets, since no
        // surrounding walk is available to carry them in.
        let enclosing_brackets = self.syntax.enclosing_bracket_stack(&self.buffer, line_idx);
        build_indent(
            self.target_indent_columns(line_idx, &enclosing_brackets, profile, config),
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

    /// Return whether the current insert-mode line already requests auto-dedent.
    ///
    /// Used to gate electric dedent so it fires only on the keystroke that
    /// completes a dedent trigger. When the line was already a closer/dedent
    /// header before the keystroke, later edits (such as inserting a space) must
    /// leave its indentation untouched.
    pub(super) fn current_line_requests_auto_dedent(&self) -> bool {
        let Some(profile) = self.active_indentation_profile() else {
            return false;
        };
        let Some(config) = profile.indentation() else {
            return false;
        };
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return false;
        };
        let spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        line_requests_auto_dedent(&line, &spans, profile, config)
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
        let spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        if !line_requests_auto_dedent(&line, &spans, profile, config) {
            return;
        }

        let current_indent_chars = leading_indent_char_count(&line);
        let current_indent_columns = indent_columns(&line, self.settings.indent_width);
        // One edited line resolves its own enclosing brackets, since no
        // surrounding walk is available to carry them in.
        let enclosing_brackets = self.syntax.enclosing_bracket_stack(&self.buffer, line_idx);
        let desired_columns =
            self.target_indent_columns(line_idx, &enclosing_brackets, profile, config);
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
    ///
    /// `enclosing_brackets` holds the brackets open at the start of `line_idx`.
    /// Callers walking a range in order carry that stack forward themselves so a
    /// whole-range reindent does not rebuild it for every line.
    fn target_indent_columns(
        &self,
        line_idx: usize,
        enclosing_brackets: &[BracketFrame],
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
        let current_spans = self.syntax.compute_spans_for_line(&self.buffer, line_idx);
        let mut target = previous_non_blank.as_ref().map_or(0, |(_, line)| {
            indent_columns(line, self.settings.indent_width)
        });

        // Each indentation family derives the base indent from the nearest
        // non-blank predecessor, then adjusts the current line relative to that
        // anchor according to the language's opening and closing cues.
        match config.style {
            IndentationStyle::CLike => self.c_like_target_indent(
                enclosing_brackets,
                &current_line,
                &current_spans,
                previous_non_blank.as_ref(),
                profile,
            ),
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

    /// Return the C-like indent column count for one line.
    ///
    /// The structural indent comes from the brackets enclosing the line: a
    /// leading closing delimiter returns to the indent of the line that opened
    /// its bracket, and every other line sits one step inside its innermost
    /// enclosing bracket. Because that result depends only on the enclosing
    /// scopes, a mis-indented line cannot propagate into the lines below it.
    ///
    /// One further step is added when the line continues an unterminated
    /// statement. That step is withheld for lines that head their own statement,
    /// and for predecessors that open the enclosing bracket, introduce the item
    /// below them, or merely close brackets, since none of those leave a
    /// statement hanging.
    fn c_like_target_indent(
        &self,
        frames: &[BracketFrame],
        current_line: &str,
        current_spans: &[HighlightSpan],
        previous_non_blank: Option<&(usize, String)>,
        profile: &crate::syntax::profile::LanguageProfile,
    ) -> usize {
        // Resolve every enclosing bracket to the indent of its opener's line,
        // which is the anchor all structural indentation is measured against.
        let enclosing_indents = frames
            .iter()
            .map(|frame| self.line_indent_columns(frame.opener_line))
            .collect::<Vec<_>>();
        let base = scope::base_indent(
            &enclosing_indents,
            current_line,
            current_spans,
            self.settings.indent_width,
        );

        // Heads such as `{`, `else`, and closing delimiters belong to the
        // statement above them, so they keep the structural indent unchanged.
        // Some languages add their own heads, such as Rust `where` clauses.
        if scope::starts_with_aligned_head(current_line, current_spans)
            || crate::indent::treat_c_like_line_as_aligned_head(
                current_line,
                current_spans,
                profile,
            )
        {
            return base;
        }
        let Some((previous_idx, previous_line)) = previous_non_blank else {
            return base;
        };
        // A predecessor that opens the innermost enclosing bracket already
        // contributed this line's indent step through the structural base.
        if frames
            .last()
            .is_some_and(|frame| frame.opener_line == *previous_idx)
        {
            return base;
        }
        let previous_spans = self
            .syntax
            .compute_spans_for_line(&self.buffer, *previous_idx);
        // Anchors such as attributes introduce the item below them instead of
        // continuing an expression, so they contribute no continuation indent.
        if crate::indent::treat_c_like_anchor_as_terminated(previous_line, &previous_spans, profile)
        {
            return base;
        }
        // A chain link belongs to the expression above it, so it is measured
        // from that line rather than from the enclosing block.
        if scope::starts_method_chain(current_line, current_spans) {
            let previous_indent = self.line_indent_columns(*previous_idx);
            // A predecessor that is itself an expression tail, such as an
            // earlier link or a line closing the brackets of one, already sits
            // at the level the chain resumes from. Anything else is the receiver
            // the chain hangs off, so the link steps one level inside it.
            if scope::starts_expression_tail(previous_line, &previous_spans) {
                return previous_indent;
            }
            return previous_indent.saturating_add(self.settings.indent_width);
        }

        // A `|` alternative written directly inside a block either lists the
        // next pattern of a match arm or adds the next operand of a bitwise-or.
        // Patterns line up with the pattern above, while an expression keeps
        // ordinary continuation indent; an assignment on that line separates the
        // two. Alternatives inside a call or macro argument list are plain
        // continuations and never reach this branch.
        if crate::indent::c_like_line_starts_pipe_alternative(current_line, current_spans, profile)
            && frames.last().is_some_and(|frame| frame.opener == '{')
        {
            let previous_indent = self.line_indent_columns(*previous_idx);
            if crate::indent::c_like_line_starts_pipe_alternative(
                previous_line,
                &previous_spans,
                profile,
            ) {
                return previous_indent;
            }
            if scope::line_contains_assignment(previous_line, &previous_spans) {
                return previous_indent.saturating_add(self.settings.indent_width);
            }
            return previous_indent;
        }

        // A predecessor made only of closing delimiters already returned to the
        // indent of the statement that owns it, so this line resumes there.
        if scope::line_only_closes_brackets(previous_line, &previous_spans) {
            return base;
        }

        if !scope::line_continues_statement(previous_line, &previous_spans) {
            return base;
        }

        // An operator opening a line continues the expression above it. Sibling
        // operators of equal binding strength share one level, a tighter-binding
        // operator nests inside the operand it splits, and a looser one returns
        // to the statement's own continuation level. This only applies while the
        // predecessor leaves an expression open, so a `||` closure passed as one
        // argument of a list is not mistaken for a boolean operator.
        if let Some(precedence) =
            scope::leading_binary_operator_precedence(current_line, current_spans)
            && let Some(previous_precedence) =
                scope::leading_binary_operator_precedence(previous_line, &previous_spans)
        {
            let previous_indent = self.line_indent_columns(*previous_idx);
            if precedence == previous_precedence {
                return previous_indent;
            }
            if precedence > previous_precedence {
                return previous_indent.saturating_add(self.settings.indent_width);
            }
        }

        base.saturating_add(self.settings.indent_width)
    }

    /// Return the indentation width of one buffer line.
    fn line_indent_columns(&self, line_idx: usize) -> usize {
        self.buffer
            .line_for_display_string(line_idx)
            .map_or(0, |line| indent_columns(&line, self.settings.indent_width))
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

/// Return the first non-whitespace character of `line` when it is code.
///
/// Returns the first non-whitespace character only when its column is a code
/// column (outside `Comment`/`String` spans); returns `None` when the line has
/// no non-whitespace character, or when that character lives inside a comment
/// or string region.
fn first_non_whitespace_code_char(line: &str, spans: &[HighlightSpan]) -> Option<char> {
    line.char_indices()
        .map(|(byte_offset, char)| (line[..byte_offset].chars().count(), char))
        .find(|(_, char)| !char.is_whitespace())
        .filter(|(column, _)| crate::indent::structural_token_is_code_column(spans, *column))
        .map(|(_, char)| char)
}

/// Return whether `line` begins with one closing brace-oriented delimiter.
///
/// Returns `true` when the first non-whitespace character is a code-column
/// `}`, `]`, or `)`; returns `false` otherwise, including when that character
/// lives inside a comment or string span.
fn starts_with_c_like_closer(line: &str, spans: &[HighlightSpan]) -> bool {
    first_non_whitespace_code_char(line, spans).is_some_and(|ch| matches!(ch, '}' | ']' | ')'))
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
    spans: &[HighlightSpan],
    profile: &crate::syntax::profile::LanguageProfile,
    config: IndentationConfig,
) -> bool {
    match config.style {
        IndentationStyle::CLike => starts_with_c_like_closer(line, spans),
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
