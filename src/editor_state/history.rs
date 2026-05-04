//! Undo and redo history helpers for `EditorState`.

use super::*;

impl EditorState {
    /// Drop all undo state and mark the freshly loaded buffer as unmodified.
    pub(super) fn reset_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.active_undo = None;
        self.saved_undo_depth = 0;
        self.replaying_history = false;
        self.sync_modified_from_history();
    }

    /// Synchronize the buffer dirty flag with the current undo/save position.
    pub(super) fn sync_modified_from_history(&mut self) {
        if self.undo_stack.len() == self.saved_undo_depth {
            self.buffer.clear_modified();
        } else {
            self.buffer.set_modified(true);
        }
    }

    /// Start capturing one undoable transaction from the current cursor position.
    pub(super) fn begin_history_transaction(&mut self) {
        if self.replaying_history || self.active_undo.is_some() {
            return;
        }
        self.active_undo = Some(ActiveUndoTransaction {
            before_cursor_char_idx: self.cursor.to_char_index(&self.buffer),
            edits: Vec::new(),
        });
    }

    /// Ensure insert-mode edits have one open transaction even in direct unit tests.
    pub(super) fn ensure_insert_history_transaction(&mut self) {
        if self.mode == Mode::Insert && self.active_undo.is_none() {
            self.begin_history_transaction();
        }
    }

    /// Record one inserted text segment in the currently active transaction.
    pub(super) fn record_history_insert(&mut self, char_idx: usize, text: &str) {
        let Some(active) = self.active_undo.as_mut() else {
            return;
        };
        active.edits.push(HistoryEdit::Insert {
            char_idx,
            text: text.to_string(),
        });
    }

    /// Record one removed text segment in the currently active transaction.
    pub(super) fn record_history_remove(&mut self, char_idx: usize, text: String) {
        let Some(active) = self.active_undo.as_mut() else {
            return;
        };
        active.edits.push(HistoryEdit::Remove { char_idx, text });
    }

    /// Commit the active transaction, clear redo, and refresh dirty-state tracking.
    pub(super) fn finish_history_transaction(&mut self) {
        let Some(active) = self.active_undo.take() else {
            return;
        };

        // Empty insert sessions like `i<Esc>` should not create synthetic undo steps.
        if active.edits.is_empty() {
            self.sync_modified_from_history();
            return;
        }

        let transaction = UndoTransaction {
            before_cursor_char_idx: active.before_cursor_char_idx.min(self.buffer.chars_count()),
            after_cursor_char_idx: self.cursor.to_char_index(&self.buffer),
            edits: active.edits,
        };
        self.undo_stack.push(transaction);
        self.redo_stack.clear();
        self.sync_modified_from_history();
    }

    /// Wrap one non-insert edit command in a single undoable transaction.
    pub(super) fn with_history_transaction<F>(&mut self, operation: F)
    where
        F: FnOnce(&mut Self),
    {
        // Nested edit helpers such as counted deletes may call into other edit
        // helpers that also use this wrapper. In that case, let the outermost
        // caller own the transaction boundaries so all sub-steps stay grouped
        // into one undo entry instead of fragmenting into many smaller ones.
        if self.replaying_history || self.active_undo.is_some() {
            operation(self);
            return;
        }

        // Start the transaction before running the closure so every shared
        // insert/remove helper it reaches records its edits into the same list.
        self.begin_history_transaction();
        operation(self);

        // Finish afterward so the final cursor position becomes the transaction's
        // redo target and empty no-op operations can be discarded cleanly.
        self.finish_history_transaction();
    }

    /// Clear pending modal-prefix state that should not survive a replay jump.
    pub(super) fn clear_pending_modal_state(&mut self) {
        self.pending_sequence.clear();
        self.pending_sequence_count = None;
        self.pending_sequence_motion_count = None;
        self.pending_operator = None;
        self.pending_macro = None;
        self.pending_find = None;
    }

    /// Apply one forward history edit while replay is suppressing capture.
    pub(super) fn apply_forward_history_edit(&mut self, edit: &HistoryEdit) {
        match edit {
            HistoryEdit::Insert { char_idx, text } => self.insert_buffer_text(*char_idx, text),
            HistoryEdit::Remove { char_idx, text } => {
                self.remove_buffer_range(*char_idx, *char_idx + text.chars().count());
            }
        }
    }

    /// Apply the inverse of one history edit while replay is suppressing capture.
    pub(super) fn apply_reverse_history_edit(&mut self, edit: &HistoryEdit) {
        match edit {
            HistoryEdit::Insert { char_idx, text } => {
                self.remove_buffer_range(*char_idx, *char_idx + text.chars().count());
            }
            HistoryEdit::Remove { char_idx, text } => self.insert_buffer_text(*char_idx, text),
        }
    }

    /// Replay one transaction for undo or redo and restore the recorded cursor endpoint.
    pub(super) fn replay_transaction(
        &mut self,
        transaction: &UndoTransaction,
        direction: ReplayDirection,
    ) {
        self.replaying_history = true;

        // Undo runs edits in reverse so later inserts/removals do not disturb
        // earlier character indices. Redo replays the original forward order.
        match direction {
            ReplayDirection::Undo => {
                for edit in transaction.edits.iter().rev() {
                    self.apply_reverse_history_edit(edit);
                }
            }
            ReplayDirection::Redo => {
                for edit in &transaction.edits {
                    self.apply_forward_history_edit(edit);
                }
            }
        }

        self.replaying_history = false;
        let target_char_idx = match direction {
            ReplayDirection::Undo => transaction.before_cursor_char_idx,
            ReplayDirection::Redo => transaction.after_cursor_char_idx,
        };
        self.cursor =
            Cursor::from_char_index(&self.buffer, target_char_idx.min(self.buffer.chars_count()));
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.desired_visual_column = None;
        self.clear_pending_modal_state();
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
    }

    /// Remove and return the next transaction from the stack used by this replay direction.
    pub(super) fn take_replay_transaction(
        &mut self,
        direction: ReplayDirection,
    ) -> Option<UndoTransaction> {
        match direction {
            ReplayDirection::Undo => self.undo_stack.pop(),
            ReplayDirection::Redo => self.redo_stack.pop(),
        }
    }

    /// Store one replayed transaction on the opposite history stack.
    pub(super) fn store_replayed_transaction(
        &mut self,
        direction: ReplayDirection,
        transaction: UndoTransaction,
    ) {
        match direction {
            // Undo removes a transaction from the undo stack, applies its inverse,
            // then makes that same transaction available for a future redo.
            ReplayDirection::Undo => self.redo_stack.push(transaction),
            // Redo consumes a previously undone transaction and restores it to the
            // undo stack as the newest committed change again.
            ReplayDirection::Redo => self.undo_stack.push(transaction),
        }
    }

    /// Move up to `count` transactions between history stacks and replay them.
    pub(super) fn step_history(
        &mut self,
        count: usize,
        direction: ReplayDirection,
        empty_message: &'static str,
    ) {
        let mut applied_any = false;

        // Pop before replay so the stack borrow ends before replay mutates other editor state.
        for _ in 0..count {
            let Some(transaction) = self.take_replay_transaction(direction) else {
                break;
            };
            self.replay_transaction(&transaction, direction);
            self.store_replayed_transaction(direction, transaction);
            applied_any = true;
        }

        self.sync_modified_from_history();
        self.status_message = if applied_any {
            None
        } else {
            Some(empty_message.to_string())
        };
    }

    /// Undo up to `count` committed transactions.
    pub(super) fn undo_changes(&mut self, count: usize) {
        self.step_history(count, ReplayDirection::Undo, "Already at oldest change");
    }

    /// Redo up to `count` transactions that were previously undone.
    pub(super) fn redo_changes(&mut self, count: usize) {
        self.step_history(count, ReplayDirection::Redo, "Already at newest change");
    }
}
