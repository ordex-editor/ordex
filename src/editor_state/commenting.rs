//! Comment-toggling helpers for `EditorState`.

use super::*;
use crate::syntax::profile::{CommentFlavor, CommentStyle, CommentStyleKind};

/// Inclusive logical-line range targeted by one linewise comment command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommentLineRange {
    start_line: usize,
    end_line: usize,
}

/// Return the cursor index after one insertion.
///
/// The result shifts forward by `inserted_len` when `insert_at` is at or
/// before `index`, and stays equal to `index` when the insertion happened
/// strictly after it.
fn adjust_char_idx_for_insert(index: usize, insert_at: usize, inserted_len: usize) -> usize {
    if insert_at <= index {
        index + inserted_len
    } else {
        index
    }
}

/// Return the cursor index after removing one character range.
///
/// The result moves backward by the removed width when `[start, end)` is fully
/// before `index`, becomes `start` when the removed range covered `index`, and
/// stays equal to `index` when the removal happened strictly after it.
fn adjust_char_idx_for_removal(index: usize, start: usize, end: usize) -> usize {
    if end <= index {
        index - (end - start)
    } else if start < index {
        start
    } else {
        index
    }
}

/// Return the leading whitespace width of one display line in characters.
fn leading_whitespace_char_count(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}

/// Return the byte index for one character boundary in a UTF-8 string slice.
fn str_char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(byte_idx, _)| byte_idx)
}

/// Return the first non-indented character index for one block-comment range.
fn block_comment_open_char_idx(buffer: &TextBuffer, start_char: usize) -> usize {
    let line_idx = buffer.char_to_line(start_char.min(buffer.chars_count()));
    if buffer.line_to_char(line_idx) != start_char {
        return start_char;
    }

    start_char
        + buffer
            .line_for_display(line_idx)
            .map(|line| line.chars().take_while(|ch| ch.is_whitespace()).count())
            .unwrap_or(0)
}

/// Return whether one line already carries the given line-comment prefix.
fn line_uses_line_comment_prefix(line: &str, open: &str) -> bool {
    let indent = leading_whitespace_char_count(line);
    let tail_start = str_char_to_byte_idx(line, indent);
    line[tail_start..].starts_with(open)
}

/// Return the removable bounds for one linewise block comment, if present.
fn linewise_block_comment_bounds(line: &str, open: &str, close: &str) -> Option<(usize, usize)> {
    let indent = leading_whitespace_char_count(line);
    let char_len = line.chars().count();
    let open_len = open.chars().count();
    let close_len = close.chars().count();
    let tail_start = str_char_to_byte_idx(line, indent);
    if !line[tail_start..].starts_with(open) || !line.ends_with(close) {
        return None;
    }

    // Trim the optional padding inserted just inside the block delimiters.
    let mut open_end = indent + open_len;
    if line.chars().nth(open_end) == Some(' ') {
        open_end += 1;
    }
    let mut close_start = char_len.saturating_sub(close_len);
    if close_start > open_end && line.chars().nth(close_start - 1) == Some(' ') {
        close_start -= 1;
    }
    (close_start >= open_end).then_some((open_end, close_start))
}

/// Return the removable bounds for one span-wrapped block comment, if present.
fn block_comment_bounds(
    buffer: &TextBuffer,
    range: SelectionRange,
    open: &str,
    close: &str,
) -> Option<(usize, usize, usize)> {
    let open_start = block_comment_open_char_idx(buffer, range.start);
    let close_len = close.chars().count();
    let close_start = range.end.saturating_sub(close_len);
    if close_start < open_start
        || !buffer.rope_slice_starts_with(open_start, range.end, open)
        || !buffer.rope_slice_starts_with(close_start, range.end, close)
    {
        return None;
    }

    // Remove the same optional inner padding that the wrapper inserts.
    let open_len = open.chars().count();
    let mut open_end = open_start + open_len;
    if buffer.char_at(open_end) == Some(' ') {
        open_end += 1;
    }
    let mut close_start = close_start;
    if close_start > open_end && buffer.char_at(close_start.saturating_sub(1)) == Some(' ') {
        close_start -= 1;
    }
    (close_start >= open_end).then_some((open_start, open_end, close_start))
}

impl EditorState {
    /// Toggle one linewise comment over the current line or counted line range.
    pub(super) fn toggle_line_comment_count(&mut self, count: usize) {
        let Some(style) = self.active_line_toggle_comment_style() else {
            self.show_status_message("No line or block comment style for current language");
            return;
        };
        self.apply_line_comment_range(self.line_comment_range_for_count(count), style);
    }

    /// Toggle one linewise comment over the active Visual selection.
    pub(super) fn toggle_line_comment_visual_selection(&mut self) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };
        let Some(style) = self.active_line_toggle_comment_style() else {
            self.show_status_message("No line or block comment style for current language");
            return;
        };

        self.prepare_visual_repeat(saved_selection, SelectionRepeatAction::ToggleLineComment);
        self.last_visual_selection = Some(saved_selection);
        self.apply_toggle_line_comment_to_visual_selection(selection, style);
        self.exit_visual_mode();
    }

    /// Toggle one block comment over the current line or counted line range.
    pub(super) fn toggle_block_comment_count(&mut self, count: usize) {
        let Some(style) = self.active_block_comment_style() else {
            self.show_status_message("No block comment style for current language");
            return;
        };
        self.apply_block_comment_range(self.block_comment_range_for_count(count), style);
    }

    /// Toggle one block comment over the active Visual selection.
    pub(super) fn toggle_block_comment_visual_selection(&mut self) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };
        let Some(style) = self.active_block_comment_style() else {
            self.show_status_message("No block comment style for current language");
            return;
        };

        self.prepare_visual_repeat(saved_selection, SelectionRepeatAction::ToggleBlockComment);
        self.last_visual_selection = Some(saved_selection);
        self.apply_toggle_block_comment_to_visual_selection(selection, style);
        self.exit_visual_mode();
    }

    /// Reapply one linewise comment toggle over an explicit Visual selection.
    pub(super) fn apply_toggle_line_comment_to_visual_selection(
        &mut self,
        selection: VisualSelection,
        style: CommentStyle,
    ) {
        self.apply_line_comment_range(
            self.line_comment_range_for_visual_selection(selection),
            style,
        );
    }

    /// Reapply one block-comment toggle over an explicit Visual selection.
    pub(super) fn apply_toggle_block_comment_to_visual_selection(
        &mut self,
        selection: VisualSelection,
        style: CommentStyle,
    ) {
        self.apply_block_comment_range(
            self.block_comment_range_for_visual_selection(selection),
            style,
        );
    }

    /// Return the active ordinary comment style for one requested structural kind.
    fn active_ordinary_comment_style(&self, kind: CommentStyleKind) -> Option<CommentStyle> {
        let styles = self.syntax.active_comment_styles();

        // Prefer the profile-marked default when one exists for the requested kind.
        styles
            .iter()
            .copied()
            .find(|style| {
                style.flavor == CommentFlavor::Ordinary
                    && style.kind == kind
                    && style.preferred_default
            })
            .or_else(|| {
                styles
                    .iter()
                    .copied()
                    .find(|style| style.flavor == CommentFlavor::Ordinary && style.kind == kind)
            })
    }

    /// Return the comment style used by the linewise toggle command.
    pub(super) fn active_line_toggle_comment_style(&self) -> Option<CommentStyle> {
        self.active_ordinary_comment_style(CommentStyleKind::Line)
            .or_else(|| self.active_ordinary_comment_style(CommentStyleKind::Block))
    }

    /// Return the comment style used by the explicit block-comment command.
    pub(super) fn active_block_comment_style(&self) -> Option<CommentStyle> {
        self.active_ordinary_comment_style(CommentStyleKind::Block)
    }

    /// Return the counted logical-line range targeted by one linewise toggle.
    fn line_comment_range_for_count(&self, count: usize) -> CommentLineRange {
        let start_line = self.cursor.line();
        let end_line =
            (start_line + count.max(1) - 1).min(self.buffer.lines_count().saturating_sub(1));
        CommentLineRange {
            start_line,
            end_line,
        }
    }

    /// Return the touched logical-line range for one explicit selection.
    fn line_comment_range_for_visual_selection(
        &self,
        selection: VisualSelection,
    ) -> CommentLineRange {
        match selection {
            VisualSelection::Character(selection) | VisualSelection::Line(selection) => {
                self.selection_line_range(selection)
            }
            VisualSelection::Block(selection) => CommentLineRange {
                start_line: selection.start_line,
                end_line: selection.end_line,
            },
        }
    }

    /// Return the counted full-line span targeted by one explicit block toggle.
    fn block_comment_range_for_count(&self, count: usize) -> SelectionRange {
        let lines = self.line_comment_range_for_count(count);
        self.full_line_range(lines.start_line, lines.end_line)
    }

    /// Return the span targeted by one block toggle over an explicit selection.
    fn block_comment_range_for_visual_selection(
        &self,
        selection: VisualSelection,
    ) -> SelectionRange {
        match selection {
            VisualSelection::Character(selection) => selection,
            VisualSelection::Line(selection) => self.full_line_selection_range(selection),
            VisualSelection::Block(selection) => {
                self.full_line_range(selection.start_line, selection.end_line)
            }
        }
    }

    /// Return the inclusive line range touched by one contiguous selection.
    fn selection_line_range(&self, selection: SelectionRange) -> CommentLineRange {
        let start_line = self
            .buffer
            .char_to_line(selection.start.min(self.buffer.chars_count()));

        // End indices are exclusive, so convert them back to the last covered
        // character before mapping to one logical line.
        let end_line = if selection.end > selection.start {
            let last_char = selection
                .end
                .saturating_sub(1)
                .min(self.buffer.chars_count().saturating_sub(1));
            self.buffer.char_to_line(last_char)
        } else {
            start_line
        };
        CommentLineRange {
            start_line,
            end_line,
        }
    }

    /// Return one contiguous range covering full logical lines.
    fn full_line_range(&self, start_line: usize, end_line: usize) -> SelectionRange {
        let start = self.buffer.line_to_char(start_line);
        let end = self.buffer.line_to_char(end_line) + self.buffer.line_len(end_line);
        SelectionRange { start, end }
    }

    /// Expand one contiguous selection to the full logical lines it touches.
    fn full_line_selection_range(&self, selection: SelectionRange) -> SelectionRange {
        let lines = self.selection_line_range(selection);
        self.full_line_range(lines.start_line, lines.end_line)
    }

    /// Apply one linewise comment toggle over the requested logical lines.
    fn apply_line_comment_range(&mut self, lines: CommentLineRange, style: CommentStyle) {
        match style.kind {
            CommentStyleKind::Line => self.apply_line_prefix_comment_range(lines, style),
            CommentStyleKind::Block => self.apply_linewise_block_comment_range(lines, style),
        }
        self.status_message = None;
    }

    /// Toggle one ordinary prefix-style comment over the given logical lines.
    fn apply_line_prefix_comment_range(&mut self, lines: CommentLineRange, style: CommentStyle) {
        let open = style.open;
        let should_uncomment = (lines.start_line..=lines.end_line).all(|line_idx| {
            self.buffer
                .line_for_display_string(line_idx)
                .is_some_and(|line| line_uses_line_comment_prefix(&line, open))
        });

        self.with_history_transaction(|editor| {
            let mut cursor_char_idx = editor.cursor.to_char_index(&editor.buffer);

            // Apply from the bottom upward so earlier line offsets remain valid.
            for line_idx in (lines.start_line..=lines.end_line).rev() {
                let Some(line) = editor.buffer.line_for_display_string(line_idx) else {
                    continue;
                };
                let line_start = editor.buffer.line_to_char(line_idx);
                let indent = leading_whitespace_char_count(&line);
                let has_content = line.chars().nth(indent).is_some();

                if should_uncomment {
                    let mut remove_end = line_start + indent + open.chars().count();
                    if editor.buffer.char_at(remove_end) == Some(' ') {
                        remove_end += 1;
                    }

                    // Prefix removal uses the same optional padding inserted on comment.
                    let remove_start = line_start + indent;
                    editor.remove_buffer_range(remove_start, remove_end);
                    cursor_char_idx =
                        adjust_char_idx_for_removal(cursor_char_idx, remove_start, remove_end);
                } else {
                    let mut insert_text = open.to_string();
                    if has_content {
                        insert_text.push(' ');
                    }

                    // Inserting after indentation preserves each line's existing layout.
                    let insert_at = line_start + indent;
                    editor.insert_buffer_text(insert_at, &insert_text);
                    cursor_char_idx = adjust_char_idx_for_insert(
                        cursor_char_idx,
                        insert_at,
                        insert_text.chars().count(),
                    );
                }
            }

            editor.cursor = Cursor::from_char_index(&editor.buffer, cursor_char_idx);
        });
    }

    /// Toggle one line-oriented block comment over the given logical lines.
    fn apply_linewise_block_comment_range(&mut self, lines: CommentLineRange, style: CommentStyle) {
        let open = style.open;
        let close = style
            .close
            .expect("block comment styles must define a closing delimiter");
        let should_uncomment = (lines.start_line..=lines.end_line).all(|line_idx| {
            self.buffer
                .line_for_display_string(line_idx)
                .and_then(|line| linewise_block_comment_bounds(&line, open, close))
                .is_some()
        });

        self.with_history_transaction(|editor| {
            let mut cursor_char_idx = editor.cursor.to_char_index(&editor.buffer);

            // Suffix edits happen before prefixes so same-line indices stay stable.
            for line_idx in (lines.start_line..=lines.end_line).rev() {
                let Some(line) = editor.buffer.line_for_display_string(line_idx) else {
                    continue;
                };
                let line_start = editor.buffer.line_to_char(line_idx);
                let line_end = line_start + editor.buffer.line_len(line_idx);
                let indent = leading_whitespace_char_count(&line);
                let has_content = line.chars().nth(indent).is_some();

                if should_uncomment {
                    let Some((open_end, close_start)) =
                        linewise_block_comment_bounds(&line, open, close)
                    else {
                        continue;
                    };

                    // Remove the closing edge first so the opening edge keeps its coordinates.
                    let close_start_char = line_start + close_start;
                    editor.remove_buffer_range(close_start_char, line_end);
                    cursor_char_idx =
                        adjust_char_idx_for_removal(cursor_char_idx, close_start_char, line_end);

                    let open_start_char = line_start + indent;
                    let open_end_char = line_start + open_end;
                    editor.remove_buffer_range(open_start_char, open_end_char);
                    cursor_char_idx = adjust_char_idx_for_removal(
                        cursor_char_idx,
                        open_start_char,
                        open_end_char,
                    );
                } else {
                    let close_text = if has_content {
                        format!(" {close}")
                    } else {
                        close.to_string()
                    };
                    editor.insert_buffer_text(line_end, &close_text);
                    cursor_char_idx = adjust_char_idx_for_insert(
                        cursor_char_idx,
                        line_end,
                        close_text.chars().count(),
                    );

                    let mut open_text = open.to_string();
                    if has_content {
                        open_text.push(' ');
                    }
                    let open_start_char = line_start + indent;
                    editor.insert_buffer_text(open_start_char, &open_text);
                    cursor_char_idx = adjust_char_idx_for_insert(
                        cursor_char_idx,
                        open_start_char,
                        open_text.chars().count(),
                    );
                }
            }

            editor.cursor = Cursor::from_char_index(&editor.buffer, cursor_char_idx);
        });
    }

    /// Toggle one block comment around the requested contiguous span.
    fn apply_block_comment_range(&mut self, range: SelectionRange, style: CommentStyle) {
        let open = style.open;
        let close = style
            .close
            .expect("block comment styles must define a closing delimiter");
        let uncomment_bounds = block_comment_bounds(&self.buffer, range, open, close);

        self.with_history_transaction(|editor| {
            let mut cursor_char_idx = editor.cursor.to_char_index(&editor.buffer);

            if let Some((open_start, open_end, close_start)) = uncomment_bounds {
                // Remove the trailing edge first so the leading range stays anchored.
                editor.remove_buffer_range(close_start, range.end);
                cursor_char_idx =
                    adjust_char_idx_for_removal(cursor_char_idx, close_start, range.end);
                editor.remove_buffer_range(open_start, open_end);
                cursor_char_idx =
                    adjust_char_idx_for_removal(cursor_char_idx, open_start, open_end);
            } else {
                let has_content = range.start < range.end;
                let close_text = if has_content {
                    format!(" {close}")
                } else {
                    close.to_string()
                };
                editor.insert_buffer_text(range.end, &close_text);
                cursor_char_idx = adjust_char_idx_for_insert(
                    cursor_char_idx,
                    range.end,
                    close_text.chars().count(),
                );

                let mut open_text = open.to_string();
                if has_content {
                    open_text.push(' ');
                }
                let open_start = block_comment_open_char_idx(&editor.buffer, range.start);
                editor.insert_buffer_text(open_start, &open_text);
                cursor_char_idx = adjust_char_idx_for_insert(
                    cursor_char_idx,
                    open_start,
                    open_text.chars().count(),
                );
            }

            editor.cursor = Cursor::from_char_index(&editor.buffer, cursor_char_idx);
        });
        self.status_message = None;
    }
}
