//! Direct editing helpers for `EditorState`.

use super::*;

/// Captures the identifier selected to seed `*` or `<Space>*`.
#[derive(Clone)]
struct WordSearchSeed {
    word: String,
    start: usize,
    selected_next_on_line: bool,
}

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

    /// Arm Insert-mode literal insert for the next supported key.
    pub(super) fn begin_insert_literal(&mut self) {
        if self.mode == Mode::Insert {
            self.pending_insert_literal = true;
        }
    }

    /// Consume the next key for one pending Insert-mode literal insert.
    ///
    /// Returns `true` when the pending literal state consumed the key, and
    /// `false` when no literal insert is waiting for input.
    pub(super) fn handle_pending_insert_literal_key(&mut self, key: Key) -> bool {
        if !self.pending_insert_literal {
            return false;
        }
        if key == Key::Esc {
            self.pending_insert_literal = false;
            return false;
        }
        if let Some(ch) = KeyBindings::literal_insert_char_for_key(key) {
            self.insert_char(ch);
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
            self.refresh_completion_session();
            self.refresh_signature_help_session();
            self.clear_pending_auto_insert_if_cursor_left_line();
        }
        self.pending_insert_literal = false;
        true
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
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };

        self.prepare_visual_repeat(saved_selection, SelectionRepeatAction::ToggleCase);
        self.last_visual_selection = Some(saved_selection);
        self.apply_toggle_case_to_visual_selection(selection);
        self.exit_visual_mode();
    }

    /// Toggle the case of one explicit contiguous selection inside a single undoable change.
    pub(super) fn apply_toggle_case_to_selection(&mut self, selection: SelectionRange) {
        // Apply the transformation inside one undoable edit so the whole
        // selection stays one repeatable and undoable change.
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
    }

    /// Toggle the case of one explicit Visual selection inside a single undoable change.
    pub(super) fn apply_toggle_case_to_visual_selection(&mut self, selection: VisualSelection) {
        match selection {
            VisualSelection::Character(selection) | VisualSelection::Line(selection) => {
                self.apply_toggle_case_to_selection(selection);
            }
            VisualSelection::Block(selection) => {
                self.with_history_transaction(|editor| {
                    let segments = selection.segments(&editor.buffer);

                    // Apply block replacements from the end of the buffer toward the
                    // front so earlier indices stay valid while later rows change.
                    for segment in segments.iter().rev() {
                        let toggled = editor
                            .buffer
                            .slice_string(segment.start, segment.end)
                            .chars()
                            .map(|ch| toggled_case(ch).unwrap_or(ch))
                            .collect::<String>();
                        editor.remove_buffer_range(segment.start, segment.end);
                        editor.insert_buffer_text(segment.start, &toggled);
                    }

                    let mut cursor = Cursor::new(selection.start_line, selection.left_column);
                    cursor.clamp_to_buffer_normal(&editor.buffer);
                    editor.cursor = cursor;
                });
            }
        }
    }

    /// Begin one block-aligned Insert mode session from the active Visual selection.
    pub(super) fn begin_visual_insert(&mut self, kind: VisualInsertKind) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };
        let VisualSelection::Block(selection) = selection else {
            return;
        };

        let action = match kind {
            VisualInsertKind::BlockStart => SelectionRepeatAction::InsertBlockStart,
            VisualInsertKind::BlockEnd => SelectionRepeatAction::AppendBlockEnd,
        };
        self.prepare_visual_repeat(saved_selection, action);
        self.last_visual_selection = Some(saved_selection);
        self.begin_history_transaction();
        self.start_visual_insert(selection, kind);
    }

    /// Start one mirrored insert session over an explicit blockwise Visual selection.
    pub(super) fn start_visual_insert(
        &mut self,
        selection: BlockSelection,
        kind: VisualInsertKind,
    ) {
        let mut session = self.visual_insert_session_for_selection(selection, kind);
        session.primary_start_char_idx = self.visual_insert_char_idx_for_column(
            selection.start_line,
            session.target_column,
            kind,
        );
        self.clear_visual_mode(Mode::Insert);
        self.cursor = Cursor::from_char_index(&self.buffer, session.primary_start_char_idx);
        self.visual_insert_session = Some(session);
    }

    /// Build the mirrored insert-session metadata for one blockwise insert command.
    fn visual_insert_session_for_selection(
        &self,
        selection: BlockSelection,
        kind: VisualInsertKind,
    ) -> VisualInsertSession {
        let touched_lines = selection.start_line..=selection.end_line;
        let primary_line = *touched_lines.start();
        let target_column = match kind {
            VisualInsertKind::BlockStart => selection.left_column,
            VisualInsertKind::BlockEnd => selection.right_column.saturating_add(1),
        };

        // Keep one canonical cursor on the first touched line so every mirrored
        // edit reuses the same block-column anchor as the primary line.
        let primary_start_char_idx =
            self.visual_insert_target_for_line(selection, primary_line, kind);
        let mut secondary_lines = touched_lines.skip(1).collect::<Vec<_>>();
        secondary_lines.reverse();

        VisualInsertSession {
            primary_start_char_idx,
            target_column,
            kind,
            secondary_lines,
        }
    }

    /// Return the buffer character index where one blockwise insert should start on `line_idx`.
    ///
    /// The "insert target" is the per-line anchor used to translate the primary
    /// cursor's Insert-mode edit script onto every other row touched by the block.
    fn visual_insert_target_for_line(
        &self,
        selection: BlockSelection,
        line_idx: usize,
        kind: VisualInsertKind,
    ) -> usize {
        let line_start = self.buffer.line_to_char(line_idx);
        let line_len = self.buffer.line_len(line_idx);
        match kind {
            VisualInsertKind::BlockStart => line_start + selection.left_column.min(line_len),
            VisualInsertKind::BlockEnd => {
                line_start + selection.right_column.saturating_add(1).min(line_len)
            }
        }
    }

    /// Snapshot the active insert-history length when Visual mirroring is armed.
    pub(super) fn visual_insert_history_len(&self) -> Option<usize> {
        if self.mode != Mode::Insert {
            return None;
        }
        // Plain Insert mode should not mirror edits; only block insert sessions
        // populate this marker, so the early `?` skips ordinary typing.
        self.visual_insert_session.as_ref()?;
        let active = self.active_undo.as_ref()?;
        Some(active.edits.len())
    }

    /// Mirror the most recent primary-line insert edits onto the remaining selected lines.
    pub(super) fn mirror_visual_insert_edits(&mut self, history_start: Option<usize>) {
        let Some(history_start) = history_start else {
            return;
        };
        let Some(session) = self.visual_insert_session.clone() else {
            return;
        };
        if self.mode != Mode::Insert || session.secondary_lines.is_empty() {
            return;
        }

        let Some(active) = self.active_undo.as_ref() else {
            return;
        };
        let original_edit_count = active.edits.len();
        if history_start >= original_edit_count {
            // When the key produced no new primary-line history edits, there is
            // nothing to replay onto the remaining block rows.
            return;
        }

        // Secondary targets are processed from the bottom of the buffer upward so
        // inserts into lower rows cannot shift the stored indices for earlier rows.
        let mut primary_cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        for secondary_line in session.secondary_lines {
            let secondary_start_char_idx = self.visual_insert_char_idx_for_column(
                secondary_line,
                session.target_column,
                session.kind,
            );
            for edit_index in history_start..original_edit_count {
                let Some(edit) = self
                    .active_undo
                    .as_ref()
                    .and_then(|active| active.edits.get(edit_index))
                    .cloned()
                else {
                    return;
                };
                let translated = self.translate_visual_insert_history_edit(
                    &edit,
                    session.primary_start_char_idx,
                    secondary_start_char_idx,
                );
                primary_cursor_char_idx =
                    Self::shift_char_idx_for_history_edit(primary_cursor_char_idx, &translated);
                self.apply_forward_history_edit(&translated);
            }
        }
        self.cursor = Cursor::from_char_index(&self.buffer, primary_cursor_char_idx);
    }

    /// Resolve one block insert column into the current buffer character index for `line_idx`.
    fn visual_insert_char_idx_for_column(
        &mut self,
        line_idx: usize,
        target_column: usize,
        kind: VisualInsertKind,
    ) -> usize {
        let line_start = self.buffer.line_to_char(line_idx);
        let line_len = self.buffer.line_len(line_idx);
        if kind == VisualInsertKind::BlockEnd && line_len < target_column {
            // Blockwise append treats short rows as containing virtual cells up to
            // the block edge, so materialize those cells as spaces before replay.
            self.insert_buffer_text(line_start + line_len, &" ".repeat(target_column - line_len));
        }
        let line_len = self.buffer.line_len(line_idx);
        line_start + target_column.min(line_len)
    }

    /// Translate one primary-line history edit so it applies at a mirrored line target.
    fn translate_visual_insert_history_edit(
        &self,
        edit: &HistoryEdit,
        primary_start_char_idx: usize,
        secondary_start_char_idx: usize,
    ) -> HistoryEdit {
        let translated_char_idx = match edit {
            HistoryEdit::Insert { char_idx, .. } | HistoryEdit::Remove { char_idx, .. } => {
                let offset = *char_idx as isize - primary_start_char_idx as isize;
                self.resolve_relative_char_idx(secondary_start_char_idx, offset)
            }
        };
        match edit {
            HistoryEdit::Insert { text, .. } => HistoryEdit::Insert {
                char_idx: translated_char_idx,
                text: text.clone(),
            },
            HistoryEdit::Remove { text, .. } => HistoryEdit::Remove {
                char_idx: translated_char_idx,
                text: text.clone(),
            },
        }
    }

    /// Shift one saved primary cursor index after a mirrored edit changes earlier text.
    fn shift_char_idx_for_history_edit(index: usize, edit: &HistoryEdit) -> usize {
        match edit {
            HistoryEdit::Insert { char_idx, text } => {
                if *char_idx <= index {
                    index + text.chars().count()
                } else {
                    index
                }
            }
            HistoryEdit::Remove { char_idx, text } => {
                let end_char_idx = char_idx + text.chars().count();
                shift_selection_index_for_removal(index, *char_idx, end_char_idx)
            }
        }
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

    /// Yank from the cursor through the end of the current line into the unnamed register.
    ///
    /// This is the direct-action equivalent of `y$`, used by the `Y` binding.
    /// The buffer is not modified and the cursor stays in Normal mode.
    pub(super) fn yank_to_line_end(&mut self) {
        let Some(selection) = self.cursor_to_line_end_selection() else {
            return;
        };
        self.store_yank_range(selection, YankKind::Character);
    }

    /// Add `delta` to the next decimal number on the current line.
    pub(super) fn offset_next_number(&mut self, delta: i64) {
        self.with_history_transaction(|editor| {
            let Some((start, end)) = editor.next_number_range_on_current_line() else {
                editor.show_error_message("No number on current line");
                return;
            };

            let current = editor.buffer.slice_string(start, end);
            let Ok(value) = current.parse::<i64>() else {
                editor.show_error_message("Number is out of range");
                return;
            };
            let Some(updated) = value.checked_add(delta) else {
                editor.show_error_message("Number is out of range");
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
        let Some(seed) = self.word_under_cursor_or_next_on_line() else {
            self.show_error_message("No word under cursor");
            return;
        };
        let pattern = format!(r"\b{}\b", escape_regex_literal(&seed.word));
        let Ok(search) = SearchQuery::compile(&pattern) else {
            self.show_error_message("Invalid search pattern");
            return;
        };
        self.last_search = Some(search);
        self.search_highlighting.reveal_committed();

        // When the cursor starts on whitespace or punctuation, the helper picks
        // the next identifier on this line only to decide which pattern `*`
        // should search for. Move the cursor to that identifier's start first so
        // the repeated search begins after the seed word instead of landing on it.
        if seed.selected_next_on_line {
            self.cursor = Cursor::from_char_index(&self.buffer, seed.start);
        }

        self.repeat_search(FindDirection::Forward);
        self.sync_search_highlights_for_viewport();
    }

    /// Search project files for whole-word matches of the identifier under the cursor.
    pub(super) fn grep_word_under_cursor(&mut self) {
        let Some(pattern) = self.whole_word_pattern_under_cursor() else {
            self.show_error_message("No word under cursor");
            return;
        };
        self.execute_grep_pattern(pattern);
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

    /// Return the current word or the next same-line word when the cursor is on a separator.
    fn word_under_cursor_or_next_on_line(&self) -> Option<WordSearchSeed> {
        // Preserve the direct "word under cursor" path so searches triggered from
        // inside an identifier behave exactly like the original `*`/`<Space>*`
        // implementation and keep the user's true cursor position as the anchor.
        if let Some(word) = self.word_under_cursor() {
            return Some(WordSearchSeed {
                word,
                start: self.cursor.to_char_index(&self.buffer),
                selected_next_on_line: false,
            });
        }

        // Restrict the fallback scan to the current line so separator positions do
        // not accidentally borrow an identifier from the line above or below. The
        // fallback exists only to infer a search pattern from nearby visible text.
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let line_end = line_start + self.buffer.line_len(self.cursor.line());
        if line_start >= line_end {
            return None;
        }

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut start = cursor_idx.min(line_end.saturating_sub(1));
        // Walk right from the cursor until a buffer-specific identifier char
        // appears. This mirrors the requested "use the next word on the same
        // line" behavior for whitespace and punctuation positions.
        while start < line_end
            && !self
                .buffer
                .char_at(start)
                .is_some_and(|ch| self.is_identifier_char_in_current_buffer(ch))
        {
            start += 1;
        }
        if start >= line_end {
            return None;
        }

        // The scan stops at an interior identifier position when the cursor lands
        // inside punctuation immediately before a word. Expand left again so the
        // returned seed always covers the full identifier and not only its suffix.
        while start > line_start
            && self
                .buffer
                .char_at(start - 1)
                .is_some_and(|candidate| self.is_identifier_char_in_current_buffer(candidate))
        {
            start -= 1;
        }
        let mut end = start + 1;
        while end < line_end
            && self
                .buffer
                .char_at(end)
                .is_some_and(|candidate| self.is_identifier_char_in_current_buffer(candidate))
        {
            end += 1;
        }
        Some(WordSearchSeed {
            word: self.buffer.slice_string(start, end),
            start,
            selected_next_on_line: true,
        })
    }

    /// Return a whole-word regex for the identifier under the cursor.
    fn whole_word_pattern_under_cursor(&self) -> Option<String> {
        let seed = self.word_under_cursor_or_next_on_line()?;
        Some(format!(r"\b{}\b", escape_regex_literal(&seed.word)))
    }

    /// Return the character range from the cursor through the current line end.
    pub(super) fn cursor_to_line_end_selection(&self) -> Option<SelectionRange> {
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
