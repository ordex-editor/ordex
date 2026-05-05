//! Repeat-last-change helpers for `EditorState`.

use super::*;

impl EditorState {
    /// Execute one resolved binding and update repeat capture state when eligible.
    pub(super) fn execute_bound_actions(&mut self, binding: &ActionBinding, count: Option<usize>) {
        let mode_before = self.mode.clone();
        // Snapshot the committed undo depth so capture can tell whether this
        // binding actually produced a new undoable change.
        let undo_depth_before = self.undo_stack.len();
        self.execute_actions_with_count(binding, count);
        self.capture_repeat_after_binding(binding, count, &mode_before, undo_depth_before);
    }

    /// Record one direct or insert-style change after a Normal-mode binding finishes.
    fn capture_repeat_after_binding(
        &mut self,
        binding: &ActionBinding,
        count: Option<usize>,
        mode_before: &Mode,
        undo_depth_before: usize,
    ) {
        if self.replaying_history || self.replaying_repeat || !mode_before.is_normal() {
            return;
        }

        // Insert-style repeats need to remember where insert input begins so the
        // final committed undo transaction can later be split into setup edits
        // and post-setup user edits.
        if matches!(self.mode, Mode::Insert) {
            let Some(active) = self.active_undo.as_ref() else {
                return;
            };
            self.active_insert_repeat = Some(ActiveInsertRepeatCapture {
                source: RepeatSource::Binding {
                    binding: binding.clone(),
                    count,
                },
                history_edit_start: active.edits.len(),
                session_start_char_idx: self.cursor.to_char_index(&self.buffer),
            });
            return;
        }

        self.active_insert_repeat = None;
        if !self.committed_new_undo_step(undo_depth_before)
            || !Self::binding_is_direct_repeat_source(binding)
        {
            // No new undo entry means the binding was a no-op, and excluded
            // bindings such as undo/redo must not replace the stored repeat target.
            return;
        }

        self.last_repeatable_change = Some(RepeatableChange::Direct(RepeatSource::Binding {
            binding: binding.clone(),
            count,
        }));
    }

    /// Record one completed operator command for Normal-mode `.` replay.
    pub(super) fn capture_repeat_after_operator(
        &mut self,
        command: ExecutedOperatorCommand,
        undo_depth_before: usize,
    ) {
        if self.replaying_history || self.replaying_repeat || !command.kind.is_repeatable_change() {
            return;
        }

        if matches!(self.mode, Mode::Insert) {
            let Some(active) = self.active_undo.as_ref() else {
                return;
            };
            self.active_insert_repeat = Some(ActiveInsertRepeatCapture {
                source: RepeatSource::Operator(command),
                history_edit_start: active.edits.len(),
                session_start_char_idx: self.cursor.to_char_index(&self.buffer),
            });
            return;
        }

        self.active_insert_repeat = None;
        if !self.committed_new_undo_step(undo_depth_before) {
            return;
        }

        self.last_repeatable_change =
            Some(RepeatableChange::Direct(RepeatSource::Operator(command)));
    }

    /// Finalize an insert-style repeat capture after Insert mode commits history.
    pub(super) fn capture_completed_insert_repeat(&mut self, undo_depth_before: usize) {
        // Skip replay capture while history/repeat playback is already rebuilding
        // editor state so synthetic undo entries do not overwrite the source change.
        if self.replaying_history || self.replaying_repeat {
            self.active_insert_repeat = None;
            return;
        }

        // Insert replay is only valid when a Normal-mode binding previously
        // marked the start of an insert-style session.
        let Some(capture) = self.active_insert_repeat.take() else {
            return;
        };
        if !self.committed_new_undo_step(undo_depth_before) {
            // Empty sessions such as `i<Esc>` commit no new undo step, so there
            // is no change worth storing for `.` replay.
            return;
        }
        // The just-finished insert session should have produced the newest
        // committed transaction because Escape closed the active history entry.
        let Some(transaction) = self.undo_stack.last() else {
            return;
        };
        if capture.history_edit_start > transaction.edits.len() {
            // Guard against malformed capture metadata before slicing the final
            // transaction into setup edits and post-setup insert edits.
            return;
        }

        // Setup actions such as `o` or `ciw` replay through their original
        // binding, so only the later insert-session edits need relative storage.
        let edits = transaction.edits[capture.history_edit_start..]
            .iter()
            .map(|edit| Self::relative_history_edit(edit, capture.session_start_char_idx))
            .collect();
        let final_cursor_offset =
            transaction.after_cursor_char_idx as isize - capture.session_start_char_idx as isize;
        self.last_repeatable_change = Some(RepeatableChange::InsertSession {
            source: capture.source,
            edits,
            final_cursor_offset,
        });
    }

    /// Replay the last repeatable change up to `count` times.
    pub(super) fn repeat_last_change(&mut self, count: usize) {
        let Some(change) = self.last_repeatable_change.clone() else {
            self.status_message = Some("Nothing to repeat".to_string());
            return;
        };

        let previous_replaying_repeat = std::mem::replace(&mut self.replaying_repeat, true);
        let repeats = count.clamp(1, Self::MAX_COUNT);

        // Keep the stored change stable while replay runs so `.` continues to
        // target the original command instead of capturing its own execution.
        match change {
            RepeatableChange::Direct(source) => {
                self.repeat_direct_change(&source, repeats);
            }
            RepeatableChange::InsertSession {
                source,
                edits,
                final_cursor_offset,
            } => self.repeat_insert_session(&source, &edits, final_cursor_offset, repeats),
        }

        self.replaying_repeat = previous_replaying_repeat;
    }

    /// Return whether this binding may become the source for repeatable direct changes.
    fn binding_is_direct_repeat_source(binding: &ActionBinding) -> bool {
        !Self::binding_actions(binding).iter().any(|action| {
            matches!(
                action,
                Action::RepeatLastChange | Action::Undo | Action::Redo
            )
        })
    }

    /// Repeat the just-finished insert-session edits for counted `i/a/I/A`.
    pub(super) fn apply_counted_insert_session_repeats(&mut self) {
        let Some(capture) = self.active_insert_repeat.clone() else {
            return;
        };
        let repeat_count = Self::insert_session_repeat_count(&capture.source);
        if self.mode != Mode::Insert || repeat_count <= 1 {
            return;
        }

        let Some(active) = self.active_undo.as_ref() else {
            return;
        };
        if capture.history_edit_start > active.edits.len() {
            // The snapshot was taken before later insert edits were recorded, so
            // a larger start index would mean the captured boundary no longer
            // points inside the current transaction and slicing would panic.
            return;
        }

        // Snapshot the original typed edit script before replay appends more
        // history entries to the active insert transaction.
        let edits = active.edits[capture.history_edit_start..]
            .iter()
            .map(|edit| Self::relative_history_edit(edit, capture.session_start_char_idx))
            .collect::<Vec<_>>();
        if edits.is_empty() {
            return;
        }
        let final_cursor_offset = self.cursor.to_char_index(&self.buffer) as isize
            - capture.session_start_char_idx as isize;

        // Each extra repeat starts at the current insert cursor so the typed
        // text lands contiguously the same way Vim-style counted insert does.
        for _ in 1..repeat_count {
            // The first copy is the user's original insert session, so the loop
            // only replays the remaining `count - 1` copies.
            let session_start_char_idx = self.cursor.to_char_index(&self.buffer);
            for edit in &edits {
                self.apply_relative_history_edit(session_start_char_idx, edit);
            }
            let target_char_idx =
                self.resolve_relative_char_idx(session_start_char_idx, final_cursor_offset);
            self.cursor = Cursor::from_char_index(&self.buffer, target_char_idx);
        }
    }

    /// Return a shared view over the actions contained in one binding.
    fn binding_actions(binding: &ActionBinding) -> &[Action] {
        match binding {
            ActionBinding::Single(action) => std::slice::from_ref(action),
            ActionBinding::Multiple(actions) => actions.as_slice(),
        }
    }

    /// Return the insert-session text repeat count stored in one repeat source.
    fn insert_session_repeat_count(source: &RepeatSource) -> usize {
        match source {
            RepeatSource::Binding { binding, count }
                if Self::binding_replays_counted_insert_text(binding) =>
            {
                count.unwrap_or(1).clamp(1, Self::MAX_COUNT)
            }
            RepeatSource::Binding { .. } | RepeatSource::Operator(_) => 1,
        }
    }

    /// Return whether a binding uses its count to replay one insert session's text.
    fn binding_replays_counted_insert_text(binding: &ActionBinding) -> bool {
        match binding {
            ActionBinding::Single(Action::EnterInsertMode | Action::InsertAfterCursor) => true,
            ActionBinding::Multiple(actions) => Self::binding_uses_insert_session_count(actions),
            ActionBinding::Single(_) => false,
        }
    }

    /// Return whether a command committed at least one new undo transaction.
    fn committed_new_undo_step(&self, undo_depth_before: usize) -> bool {
        // Undo depth only grows when the command finished with a committed edit,
        // so `<=` means the action was a no-op or stayed inside an unfinished session.
        self.undo_stack.len() > undo_depth_before
    }

    /// Convert one committed history edit into a session-relative replay edit.
    fn relative_history_edit(
        edit: &HistoryEdit,
        session_start_char_idx: usize,
    ) -> RelativeHistoryEdit {
        // Relative storage lets the same insert-session edit script replay from
        // a different cursor position without preserving absolute buffer indices.
        match edit {
            HistoryEdit::Insert { char_idx, text } => RelativeHistoryEdit::Insert {
                char_idx_offset: *char_idx as isize - session_start_char_idx as isize,
                text: text.clone(),
            },
            HistoryEdit::Remove { char_idx, text } => RelativeHistoryEdit::Remove {
                char_idx_offset: *char_idx as isize - session_start_char_idx as isize,
                text: text.clone(),
            },
        }
    }

    /// Replay one stored direct change by executing its original binding again.
    fn repeat_direct_change(&mut self, source: &RepeatSource, repeats: usize) {
        for _ in 0..repeats {
            let undo_depth_before = self.undo_stack.len();
            // Stop once the stored change can no longer apply at the current site.
            self.replay_repeat_source(source);
            if self.undo_stack.len() == undo_depth_before {
                break;
            }
        }
    }

    /// Replay one stored insert-style change by re-running setup and later edits.
    fn repeat_insert_session(
        &mut self,
        source: &RepeatSource,
        edits: &[RelativeHistoryEdit],
        final_cursor_offset: isize,
        repeats: usize,
    ) {
        for _ in 0..repeats {
            let undo_depth_before = self.undo_stack.len();
            self.replay_repeat_source(source);
            if !matches!(self.mode, Mode::Insert) || self.active_undo.is_none() {
                // Replay needs an active insert transaction so the relative edit
                // script can append to the same undo step as the setup binding.
                break;
            }

            let session_start_char_idx = self.cursor.to_char_index(&self.buffer);
            // Replay each edit relative to the new insert-session start so the
            // inserted text and cursor corrections stay local to the new site.
            for edit in edits {
                self.apply_relative_history_edit(session_start_char_idx, edit);
            }
            self.finish_replayed_insert_session(session_start_char_idx, final_cursor_offset);

            if self.undo_stack.len() == undo_depth_before {
                break;
            }
        }
    }

    /// Replay one stored source command without changing the saved repeat target.
    fn replay_repeat_source(&mut self, source: &RepeatSource) {
        match source {
            RepeatSource::Binding { binding, count } => {
                self.execute_actions_with_count(binding, *count);
            }
            RepeatSource::Operator(command) => {
                self.execute_operator_command(command.clone());
            }
        }
    }

    /// Apply one relative edit during insert-session replay.
    fn apply_relative_history_edit(
        &mut self,
        session_start_char_idx: usize,
        edit: &RelativeHistoryEdit,
    ) {
        match edit {
            RelativeHistoryEdit::Insert {
                char_idx_offset,
                text,
            } => {
                // Insert offsets stay anchored to the insert-session start so the
                // replayed text lands at the same relative location as before.
                let char_idx =
                    self.resolve_relative_char_idx(session_start_char_idx, *char_idx_offset);
                self.insert_buffer_text(char_idx, text);
            }
            RelativeHistoryEdit::Remove {
                char_idx_offset,
                text,
            } => {
                // Removal sizes come from the recorded deleted text, which keeps
                // repeat replay aligned with the original transaction boundaries.
                let start_char =
                    self.resolve_relative_char_idx(session_start_char_idx, *char_idx_offset);
                let end_char = start_char
                    .saturating_add(text.chars().count())
                    .min(self.buffer.chars_count());
                self.remove_buffer_range(start_char, end_char);
            }
        }
    }

    /// Resolve one signed character offset against the current buffer.
    fn resolve_relative_char_idx(&self, session_start_char_idx: usize, offset: isize) -> usize {
        let raw = if offset.is_negative() {
            session_start_char_idx.saturating_sub(offset.unsigned_abs())
        } else {
            session_start_char_idx.saturating_add(offset as usize)
        };
        raw.min(self.buffer.chars_count())
    }

    /// Finish one replayed insert session without applying a second cursor-left adjustment.
    fn finish_replayed_insert_session(
        &mut self,
        session_start_char_idx: usize,
        final_cursor_offset: isize,
    ) {
        // The stored cursor offset already reflects the original post-escape
        // Normal-mode position, so replay must restore it directly.
        let target_char_idx =
            self.resolve_relative_char_idx(session_start_char_idx, final_cursor_offset);
        self.cursor = Cursor::from_char_index(&self.buffer, target_char_idx);
        self.dismiss_completion_session(false);
        self.clear_visual_mode(Mode::Normal);
        self.finish_history_transaction();
        self.cursor.clamp_to_line_normal(&self.buffer);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
    }
}
