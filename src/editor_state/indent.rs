//! Indentation helpers for `EditorState`.

use super::*;
use crate::syntax::profile::{IndentationConfig, IndentationStyle};

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

impl EditorState {
    /// Reindent the current Visual selection and return to Normal mode.
    pub(super) fn reindent_visual_selection(&mut self) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some((selection, _kind)) = self.normalized_selection() else {
            return;
        };

        self.prepare_visual_repeat(saved_selection, SelectionRepeatAction::Reindent);
        self.last_visual_selection = Some(saved_selection);
        self.reindent_selection(selection);
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
    ///
    /// Returns `true` when the current language exposes a built-in indentation rule,
    /// and `false` when indentation is unsupported for the active file.
    pub(super) fn reindent_selection(&mut self, selection: SelectionRange) -> bool {
        let Some(profile) = self.active_indentation_profile() else {
            self.show_status_message("No manual indent rule for current language");
            return false;
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
            self.status_message = None;
        }
        true
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
            self.status_message = None;
        }
    }

    /// Return how many logical lines one indent-style selection touches.
    pub(super) fn indentation_line_count(&self, selection: SelectionRange) -> usize {
        let line_range = self.indent_line_range(selection);
        line_range.end_line.saturating_sub(line_range.start_line) + 1
    }

    /// Insert one newline at the cursor and auto-indent the new line when supported.
    pub(super) fn insert_newline_with_auto_indent(&mut self) {
        self.cleanup_pending_auto_indent_line();
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.insert_buffer_text(char_idx, "\n");
        self.cursor.move_down(&self.buffer);
        self.cursor.set_column(0);
        self.apply_auto_indent_to_current_line();
    }

    /// Open one line below the cursor, auto-indent it, and enter Insert mode.
    pub(super) fn open_line_below_with_auto_indent(&mut self) {
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.insert_buffer_text(line_end, "\n");
        self.cursor = Cursor::new(line + 1, 0);
        self.apply_auto_indent_to_current_line();
        self.enter_insert_mode();
    }

    /// Open one line above the cursor, auto-indent it, and enter Insert mode.
    pub(super) fn open_line_above_with_auto_indent(&mut self) {
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_start = self.buffer.line_to_char(line);
        self.insert_buffer_text(line_start, "\n");
        self.cursor = Cursor::new(line, 0);
        self.apply_auto_indent_to_current_line();
        self.enter_insert_mode();
    }

    /// Remove one untouched auto-indent prefix before Insert mode exits.
    pub(super) fn cleanup_pending_auto_indent_on_exit(&mut self) {
        self.cleanup_pending_auto_indent_line();
        self.pending_auto_indent = None;
    }

    /// Mark the tracked auto-indented blank line as touched by user edits.
    pub(super) fn touch_pending_auto_indent(&mut self) {
        if let Some(pending) = self.pending_auto_indent.as_mut()
            && pending.line == self.cursor.line()
        {
            pending.touched = true;
        }
    }

    /// Drop auto-indent cleanup tracking when the insert cursor leaves that line.
    pub(super) fn clear_pending_auto_indent_if_cursor_left_line(&mut self) {
        let should_clear = self.mode != Mode::Insert
            || self
                .pending_auto_indent
                .as_ref()
                .is_some_and(|pending| pending.line != self.cursor.line());
        if should_clear {
            self.pending_auto_indent = None;
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

    /// Apply one language-aware indentation prefix to the current line, when supported.
    fn apply_auto_indent_to_current_line(&mut self) {
        self.pending_auto_indent = None;
        let Some(profile) = self.active_indentation_profile() else {
            return;
        };
        let Some(config) = profile.indentation() else {
            return;
        };
        let line_idx = self.cursor.line();
        let desired_indent = build_indent(
            self.target_indent_columns(line_idx, profile, config),
            self.settings.indent_width,
            self.settings.indent_with_tabs,
        );
        if desired_indent.is_empty() {
            return;
        }

        // Insert the computed prefix at column zero so both blank lines and
        // split-line newlines reuse the same indentation calculation.
        let line_start = self.buffer.line_to_char(line_idx);
        self.insert_buffer_text(line_start, &desired_indent);
        self.cursor.set_column(desired_indent.chars().count());
        self.remember_pending_auto_indent_line(line_idx, desired_indent);
    }

    /// Record one untouched auto-indented blank line for later cleanup.
    fn remember_pending_auto_indent_line(&mut self, line_idx: usize, indent: String) {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            self.pending_auto_indent = None;
            return;
        };
        self.pending_auto_indent = (line == indent).then_some(PendingAutoIndentLine {
            line: line_idx,
            indent,
            touched: false,
        });
    }

    /// Remove one tracked auto-indent prefix when the line stayed untouched and blank.
    fn cleanup_pending_auto_indent_line(&mut self) {
        let Some(pending) = self.pending_auto_indent.clone() else {
            return;
        };
        if pending.touched || pending.line != self.cursor.line() {
            return;
        }

        // Cleanup only applies when the line still consists of the inserted
        // prefix and no later edits changed its contents.
        let Some(line) = self.buffer.line_for_display_string(pending.line) else {
            self.pending_auto_indent = None;
            return;
        };
        if line != pending.indent {
            self.pending_auto_indent = None;
            return;
        }

        let line_start = self.buffer.line_to_char(pending.line);
        let indent_end = line_start + pending.indent.chars().count();
        self.remove_buffer_range(line_start, indent_end);
        self.cursor = Cursor::new(pending.line, 0);
        self.pending_auto_indent = None;
    }

    /// Indent the current insert-mode line by one configured shift width.
    pub(super) fn indent_current_line_insert_mode(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        // Rebuild the leading indent from the configured shift width so tabs and
        // spaces follow the same settings used by auto-indent.
        self.touch_pending_auto_indent();
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
        self.touch_pending_auto_indent();
        let line_idx = self.cursor.line();
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return;
        };
        let (current_chars, desired) = self.adjusted_indent_prefix(&line, IndentDirection::Dedent);
        self.replace_current_line_indent(line_idx, current_chars, desired);
    }

    /// Adjust the active Visual selection's indentation and return to Normal mode.
    fn change_visual_selection_indentation(&mut self, direction: IndentDirection) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some((selection, _kind)) = self.normalized_selection() else {
            return;
        };

        let action = match direction {
            IndentDirection::Indent => SelectionRepeatAction::Indent,
            IndentDirection::Dedent => SelectionRepeatAction::Dedent,
        };
        self.prepare_visual_repeat(saved_selection, action);
        self.last_visual_selection = Some(saved_selection);
        self.adjust_selection_indentation(selection, direction);
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
        .any(|keyword| starts_with_keyword(trimmed, keyword, profile))
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
