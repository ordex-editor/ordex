//! Direct editing helpers for `EditorState`.

use super::*;

impl EditorState {
    /// Mark the next frame as requiring a full redraw.
    pub(super) fn request_full_redraw(&mut self) {
        self.redraw_requested = true;
    }

    /// Arm one pending `r` command with the provided replacement count.
    pub(super) fn begin_replace_char(&mut self, count: usize) {
        self.pending_replace = Some(PendingReplace {
            count: count.max(1),
        });
    }

    /// Consume the next replacement target for one pending `r` command.
    ///
    /// Returns `true` when the pending replace state consumed the key, and
    /// `false` when no replace is waiting for input.
    pub(super) fn handle_pending_replace_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_replace.take() else {
            return false;
        };
        if key == Key::Esc {
            return true;
        }

        // Replacement accepts only insertable characters so `r` stays a
        // one-shot edit instead of recursively invoking other bindings.
        let Some(replacement) = KeyBindings::is_insertable_char(key) else {
            return true;
        };
        let undo_depth_before = self.undo_stack.len();
        self.replace_chars_under_cursor(replacement, pending.count);
        if !self.replaying_history
            && !self.replaying_repeat
            && self.undo_stack.len() > undo_depth_before
        {
            self.last_repeatable_change =
                Some(RepeatableChange::Direct(RepeatSource::ReplaceChar {
                    count: pending.count,
                    replacement,
                }));
        }
        true
    }

    /// Toggle the case of up to `count` characters on the current line.
    pub(super) fn toggle_case_at_cursor_count(&mut self, count: usize) {
        self.with_history_transaction(|editor| {
            let mut char_idx = editor.cursor.to_char_index(&editor.buffer);
            let mut toggled = 0usize;

            // Stay on the current line so Normal-mode `~` does not rewrite the
            // following line break or walk into the next line unexpectedly.
            for _ in 0..count {
                let line_start = editor.buffer.line_to_char(editor.cursor.line());
                let line_end = line_start + editor.buffer.line_len(editor.cursor.line());
                if char_idx >= line_end {
                    break;
                }

                let Some(ch) = editor.buffer.char_at(char_idx) else {
                    break;
                };
                let Some(replacement) = toggled_case(ch) else {
                    break;
                };

                editor.remove_buffer_range(char_idx, char_idx + 1);
                editor.insert_buffer_text(char_idx, &replacement.to_string());
                toggled += 1;
                if char_idx + 1 >= line_end {
                    break;
                }
                char_idx += 1;
            }

            if toggled > 0 {
                let cursor_idx = if toggled == 1 {
                    char_idx
                } else {
                    char_idx.min(editor.buffer.chars_count().saturating_sub(1))
                };
                editor.cursor = Cursor::from_char_index(&editor.buffer, cursor_idx);
            }
        });
    }

    /// Toggle the case of the active Visual selection and return to Normal mode.
    pub(super) fn toggle_case_visual_selection(&mut self) {
        let Some((selection, _kind)) = self.normalized_selection() else {
            return;
        };

        // Apply the transformation inside one undoable edit so Visual `~` behaves
        // like one change even when the selection spans multiple characters.
        self.with_history_transaction(|editor| {
            let toggled = editor
                .buffer
                .slice_string(selection.start, selection.end)
                .chars()
                .map(|ch| toggled_case(ch).unwrap_or(ch))
                .collect::<String>();
            editor.remove_buffer_range(selection.start, selection.end);
            editor.insert_buffer_text(selection.start, &toggled);
            editor.cursor = Cursor::from_char_index(&editor.buffer, selection.start);
        });
        self.exit_visual_mode();
    }

    /// Delete from the cursor through the end of the current line.
    pub(super) fn delete_to_line_end(&mut self) {
        self.with_history_transaction(|editor| {
            let Some(selection) = editor.cursor_to_line_end_selection() else {
                return;
            };
            editor.delete_range_into_yank_buffer(selection, YankKind::Character);
        });
    }

    /// Delete to the end of the current line and enter Insert mode.
    pub(super) fn change_to_line_end(&mut self) {
        self.begin_history_transaction();
        // `C` should still enter Insert mode at EOL even when there is nothing
        // left to delete, which mirrors the ordinary `c$` editing flow.
        if let Some(selection) = self.cursor_to_line_end_selection() {
            self.delete_range_into_yank_buffer(selection, YankKind::Character);
        }
        self.enter_insert_mode();
    }

    /// Add `delta` to the next decimal number on the current line.
    pub(super) fn offset_next_number(&mut self, delta: i64) {
        self.with_history_transaction(|editor| {
            let Some((start, end)) = editor.next_number_range_on_current_line() else {
                editor.show_status_message("No number on current line");
                return;
            };

            let current = editor.buffer.slice_string(start, end);
            let Ok(value) = current.parse::<i64>() else {
                editor.show_status_message("Number is out of range");
                return;
            };
            let Some(updated) = value.checked_add(delta) else {
                editor.show_status_message("Number is out of range");
                return;
            };

            // Replace only the located number span so `Ctrl-A`/`Ctrl-X` preserve
            // the rest of the line and stay inside one undoable change.
            editor.remove_buffer_range(start, end);
            editor.insert_buffer_text(start, &updated.to_string());
            editor.cursor = Cursor::from_char_index(&editor.buffer, start);
        });
    }

    /// Join the current line with up to `count` following lines.
    pub(super) fn join_lines_count(&mut self, count: usize) {
        self.with_history_transaction(|editor| {
            // Each iteration joins the current line with its immediate successor,
            // so repeated joins naturally collapse a whole block into one line.
            for _ in 0..count {
                if !editor.join_current_line_with_next() {
                    break;
                }
            }
        });
    }

    /// Replace up to `count` characters under the cursor with `replacement`.
    pub(super) fn replace_chars_under_cursor(&mut self, replacement: char, count: usize) {
        self.with_history_transaction(|editor| {
            let line_start = editor.buffer.line_to_char(editor.cursor.line());
            let line_end = line_start + editor.buffer.line_len(editor.cursor.line());
            let start = editor.cursor.to_char_index(&editor.buffer);
            if start >= line_end {
                return;
            }
            let end = start.saturating_add(count).min(line_end);
            let replacement_text = replacement.to_string().repeat(end - start);

            // `r` is a direct replacement, not a delete+yank command, so update
            // only the targeted span and leave the unnamed register untouched.
            editor.remove_buffer_range(start, end);
            editor.insert_buffer_text(start, &replacement_text);
            let final_idx = start + replacement_text.chars().count().saturating_sub(1);
            editor.cursor = Cursor::from_char_index(&editor.buffer, final_idx);
        });
    }

    /// Search forward for the next literal occurrence of the word under the cursor.
    pub(super) fn search_word_under_cursor(&mut self) {
        let Some(word) = self.word_under_cursor() else {
            self.show_status_message("No word under cursor");
            return;
        };
        let pattern = format!(r"\b{}\b", escape_regex_literal(&word));
        let Ok(search) = SearchQuery::compile(&pattern) else {
            self.show_status_message("Invalid search pattern");
            return;
        };
        self.last_search = Some(search);
        self.search_highlighting.reveal_committed();
        self.repeat_search(FindDirection::Forward);
        self.sync_search_highlights_for_viewport();
    }

    /// Return the current rename/search word under the Normal-mode cursor.
    fn word_under_cursor(&self) -> Option<String> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let ch = self.buffer.char_at(cursor_idx)?;
        if !self.is_identifier_char_in_current_buffer(ch) {
            return None;
        }

        // Expand from the cursor to the full identifier span so `*` searches for
        // the same token that rename prefill would recognize.
        let mut start = cursor_idx;
        while start > 0
            && self
                .buffer
                .char_at(start - 1)
                .is_some_and(|candidate| self.is_identifier_char_in_current_buffer(candidate))
        {
            start -= 1;
        }

        let mut end = cursor_idx + 1;
        while self
            .buffer
            .char_at(end)
            .is_some_and(|candidate| self.is_identifier_char_in_current_buffer(candidate))
        {
            end += 1;
        }
        Some(self.buffer.slice_string(start, end))
    }

    /// Return the character range from the cursor through the current line end.
    fn cursor_to_line_end_selection(&self) -> Option<SelectionRange> {
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let line_end = line_start + self.buffer.line_len(self.cursor.line());
        let start = self.cursor.to_char_index(&self.buffer);
        if start >= line_end {
            return None;
        }
        Some(SelectionRange {
            start,
            end: line_end,
        })
    }

    /// Return the next signed decimal number range on the current line.
    pub(super) fn next_number_range_on_current_line(&self) -> Option<(usize, usize)> {
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let line_end = line_start + self.buffer.line_len(self.cursor.line());
        let mut idx = self.cursor.to_char_index(&self.buffer);
        while idx < line_end {
            if !self
                .buffer
                .char_at(idx)
                .is_some_and(|ch| ch.is_ascii_digit())
            {
                idx += 1;
                continue;
            }

            // Accept one immediate unary sign when it belongs to this decimal
            // run instead of to an earlier number.
            let mut start = idx;
            let previous_char_is_sign = idx > line_start
                && self
                    .buffer
                    .char_at(idx - 1)
                    .is_some_and(|ch| matches!(ch, '+' | '-'));
            let sign_starts_number = idx == line_start + 1
                || self
                    .buffer
                    .char_at(idx - 2)
                    .is_none_or(|ch| !ch.is_ascii_digit());

            // Include the sign only when it sits directly before this digit run.
            // A sign at the beginning of the line is unary, and a sign preceded
            // by a non-digit is also unary. A sign after another digit belongs to
            // an earlier numeric token such as `1-23`, so it must stay separate.
            if previous_char_is_sign && sign_starts_number {
                start -= 1;
            }

            let mut end = idx + 1;
            while end < line_end
                && self
                    .buffer
                    .char_at(end)
                    .is_some_and(|ch| ch.is_ascii_digit())
            {
                end += 1;
            }
            return Some((start, end));
        }
        None
    }

    /// Join the current line with its immediate successor.
    ///
    /// Returns `true` when the buffer changed, and `false` when there was no
    /// following line to join.
    fn join_current_line_with_next(&mut self) -> bool {
        let current_line = self.cursor.line();
        if current_line + 1 >= self.buffer.lines_count() {
            return false;
        }

        let line_start = self.buffer.line_to_char(current_line);
        let line_end = line_start + self.buffer.line_len(current_line);
        let next_line_start = self.buffer.line_to_char(current_line + 1);
        let next_line = self
            .buffer
            .line_for_display_string(current_line + 1)
            .unwrap_or_default();
        let leading_ws = leading_horizontal_whitespace_chars(&next_line);
        let trimmed_next = next_line.trim_start_matches([' ', '\t']);
        let needs_separator = line_end > line_start
            && self
                .buffer
                .char_at(line_end.saturating_sub(1))
                .is_some_and(|ch| !ch.is_whitespace())
            && !trimmed_next.is_empty();
        let remove_end = next_line_start + leading_ws;

        self.remove_buffer_range(line_end, remove_end);
        if needs_separator {
            self.insert_buffer_text(line_end, " ");
        }
        self.cursor = Cursor::new(current_line, self.cursor.column());
        true
    }
}

/// Return the toggled case of one ASCII alphabetic character.
fn toggled_case(ch: char) -> Option<char> {
    if ch.is_ascii_lowercase() {
        return Some(ch.to_ascii_uppercase());
    }
    if ch.is_ascii_uppercase() {
        return Some(ch.to_ascii_lowercase());
    }
    None
}

/// Count the leading spaces and tabs in one display line.
fn leading_horizontal_whitespace_chars(line: &str) -> usize {
    line.chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count()
}

/// Escape one string for literal use inside a regex pattern.
fn escape_regex_literal(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        // Regex metacharacters must be escaped so `*` searches for the literal
        // identifier text under the cursor instead of interpreting it as syntax.
        if matches!(
            ch,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}
