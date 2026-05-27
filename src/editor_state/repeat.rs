//! Repeat-last-change helpers for `EditorState`.

use super::auto_insert::IndentDirection;
use super::*;

impl EditorState {
    /// Execute one resolved binding and update repeat capture state when eligible.
    pub(super) fn execute_bound_binding(&mut self, binding: &Binding, count: Option<usize>) {
        let mode_before = self.mode.clone();
        // Snapshot the committed undo depth so capture can tell whether this
        // binding actually produced a new undoable change.
        let undo_depth_before = self.undo_stack.len();
        self.execute_binding_with_count(binding, count);
        self.capture_repeat_after_binding(binding, count, None, &mode_before, undo_depth_before);
    }

    /// Record one direct or insert-style change after a Normal-mode binding finishes.
    pub(super) fn capture_repeat_after_binding(
        &mut self,
        binding: &Binding,
        count: Option<usize>,
        register: Option<ClipboardRegister>,
        mode_before: &Mode,
        undo_depth_before: usize,
    ) {
        if self.replaying_history || self.replaying_repeat {
            return;
        }
        if mode_before.is_visual() {
            self.capture_repeat_after_visual_binding(undo_depth_before);
            return;
        }
        if !mode_before.is_normal() {
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
                    register,
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
            register,
        }));
    }

    /// Record one completed operator command for Normal-mode `.` replay.
    pub(super) fn capture_repeat_after_operator(
        &mut self,
        command: ExecutedOperatorCommand,
        selection_source: Option<SelectionRepeatCommand>,
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

        let source =
            selection_source.map_or(RepeatSource::Operator(command), RepeatSource::Selection);
        self.last_repeatable_change = Some(RepeatableChange::Direct(source));
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
            self.show_status_message("Nothing to repeat");
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
    fn binding_is_direct_repeat_source(binding: &Binding) -> bool {
        match binding {
            Binding::Actions(actions) => {
                !Self::action_binding_actions(actions).iter().any(|action| {
                    matches!(
                        action,
                        Action::RepeatLastChange | Action::Undo | Action::Redo
                    )
                })
            }
            Binding::Replay(_) => true,
        }
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
    fn action_binding_actions(binding: &ActionBinding) -> &[Action] {
        match binding {
            ActionBinding::Single(action) => std::slice::from_ref(action),
            ActionBinding::Multiple(actions) => actions.as_slice(),
        }
    }

    /// Return the insert-session text repeat count stored in one repeat source.
    fn insert_session_repeat_count(source: &RepeatSource) -> usize {
        match source {
            RepeatSource::Binding {
                binding,
                count,
                register: _,
            } if Self::binding_replays_counted_insert_text(binding) => {
                count.unwrap_or(1).clamp(1, Self::MAX_COUNT)
            }
            RepeatSource::Binding { .. }
            | RepeatSource::Operator(_)
            | RepeatSource::Selection(_)
            | RepeatSource::ReplaceChar { .. } => 1,
        }
    }

    /// Return whether a binding uses its count to replay one insert session's text.
    fn binding_replays_counted_insert_text(binding: &Binding) -> bool {
        match binding {
            Binding::Actions(ActionBinding::Single(
                Action::EnterInsertMode | Action::InsertAfterCursor,
            )) => true,
            Binding::Actions(ActionBinding::Multiple(actions)) => {
                Self::binding_uses_insert_session_count(actions)
            }
            Binding::Actions(ActionBinding::Single(_)) | Binding::Replay(_) => false,
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
            RepeatSource::Binding {
                binding,
                count,
                register,
            } => {
                self.replay_registered_binding(binding, *count, *register);
            }
            RepeatSource::Operator(command) => {
                self.execute_operator_command(command.clone());
            }
            RepeatSource::Selection(command) => {
                self.replay_selection_repeat(command);
            }
            RepeatSource::ReplaceChar { count, replacement } => {
                self.replace_chars_under_cursor(*replacement, *count);
            }
        }
    }

    /// Record one completed Visual-mode change so `.` can rebuild its selection shape.
    fn capture_repeat_after_visual_binding(&mut self, undo_depth_before: usize) {
        let Some(command) = self.pending_visual_repeat.take() else {
            return;
        };
        // Blockwise `I`/`A` keeps editing in one mirrored Insert session whose
        // setup depends on the live block shape, so `.` intentionally does not
        // capture or replay it as a standalone repeatable change.
        if matches!(
            command.action,
            SelectionRepeatAction::InsertBlockStart | SelectionRepeatAction::AppendBlockEnd
        ) {
            self.active_insert_repeat = None;
            self.last_repeatable_change = None;
            return;
        }

        if matches!(self.mode, Mode::Insert) {
            // Visual `c` keeps the delete phase and inserted text inside one undo
            // transaction, so repeat capture must wait for Insert mode to finish.
            let Some(active) = self.active_undo.as_ref() else {
                return;
            };
            self.active_insert_repeat = Some(ActiveInsertRepeatCapture {
                source: RepeatSource::Selection(command),
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
            Some(RepeatableChange::Direct(RepeatSource::Selection(command)));
    }

    /// Convert one stored Visual selection into the shape used for later repeats.
    fn selection_repeat_target_for_visual_change(
        &self,
        selection: LastVisualSelection,
        action: SelectionRepeatAction,
    ) -> SelectionRepeatTarget {
        let anchor = Cursor::from_char_index(
            &self.buffer,
            selection
                .anchor_char_idx
                .min(self.buffer.chars_count().saturating_sub(1)),
        );
        let cursor = Cursor::from_char_index(
            &self.buffer,
            selection
                .cursor_char_idx
                .min(self.buffer.chars_count().saturating_sub(1)),
        );
        let line_count = anchor.line().abs_diff(cursor.line()) + 1;
        match action {
            SelectionRepeatAction::Indent
            | SelectionRepeatAction::Dedent
            | SelectionRepeatAction::Reindent
            | SelectionRepeatAction::ToggleLineComment
            | SelectionRepeatAction::InsertBlockStart
            | SelectionRepeatAction::AppendBlockEnd => SelectionRepeatTarget::Lines {
                // Indent-style commands act on touched lines, so repeats should
                // rebuild the same line span regardless of original char columns.
                line_count: line_count.max(1),
            },
            SelectionRepeatAction::Delete
            | SelectionRepeatAction::Change
            | SelectionRepeatAction::ToggleCase => match selection.kind {
                VisualKind::Character => SelectionRepeatTarget::Character {
                    // Characterwise Visual changes replay over the same number of
                    // selected characters starting at the new cursor position.
                    char_count: selection
                        .end_char_idx
                        .saturating_sub(selection.start_char_idx)
                        .max(1),
                },
                VisualKind::Line => SelectionRepeatTarget::Lines {
                    line_count: selection.line_count.max(1),
                },
                VisualKind::Block => SelectionRepeatTarget::Block {
                    line_count: line_count.max(1),
                    column_count: anchor.column().abs_diff(cursor.column()) + 1,
                },
            },
            SelectionRepeatAction::ToggleBlockComment => match selection.kind {
                VisualKind::Character => SelectionRepeatTarget::Character {
                    char_count: selection
                        .end_char_idx
                        .saturating_sub(selection.start_char_idx)
                        .max(1),
                },
                VisualKind::Line | VisualKind::Block => SelectionRepeatTarget::Lines {
                    line_count: line_count.max(1),
                },
            },
        }
    }

    /// Store the selection shape for one Visual change before it mutates the buffer.
    pub(super) fn prepare_visual_repeat(
        &mut self,
        selection: LastVisualSelection,
        action: SelectionRepeatAction,
    ) {
        self.pending_visual_repeat = Some(SelectionRepeatCommand {
            action,
            target: self.selection_repeat_target_for_visual_change(selection, action),
            register: None,
        });
    }

    /// Attach one explicit clipboard register to the pending visual repeat command.
    pub(super) fn set_pending_visual_register(&mut self, register: ClipboardRegister) {
        if let Some(command) = self.pending_visual_repeat.as_mut() {
            command.register = Some(register);
        }
    }

    /// Replay one stored selection-shaped change from the current cursor.
    fn replay_selection_repeat(&mut self, command: &SelectionRepeatCommand) {
        let Some(selection) = self.selection_for_repeat_target(command.target) else {
            return;
        };

        // Reapply the stored edit using the same helpers as the original command
        // so history, cursor placement, and side effects stay aligned.
        match command.action {
            SelectionRepeatAction::Delete => {
                self.apply_delete_visual_selection(selection, false, true);
                self.queue_clipboard_write_from_yank_buffer(command.register);
            }
            SelectionRepeatAction::Change => {
                self.apply_delete_visual_selection(selection, true, true);
                self.queue_clipboard_write_from_yank_buffer(command.register);
            }
            SelectionRepeatAction::ToggleCase => {
                self.apply_toggle_case_to_visual_selection(selection);
            }
            SelectionRepeatAction::ToggleLineComment => {
                if let Some(style) = self.active_line_toggle_comment_style() {
                    self.apply_toggle_line_comment_to_visual_selection(selection, style);
                }
            }
            SelectionRepeatAction::ToggleBlockComment => {
                if let Some(style) = self.active_block_comment_style() {
                    self.apply_toggle_block_comment_to_visual_selection(selection, style);
                }
            }
            SelectionRepeatAction::Reindent => {
                self.reindent_visual_selection_shape(selection);
            }
            SelectionRepeatAction::Indent => {
                self.adjust_visual_selection_indentation(selection, IndentDirection::Indent);
            }
            SelectionRepeatAction::Dedent => {
                self.adjust_visual_selection_indentation(selection, IndentDirection::Dedent);
            }
            SelectionRepeatAction::InsertBlockStart => {
                if let VisualSelection::Block(selection) = selection {
                    self.start_visual_insert(selection, VisualInsertKind::BlockStart);
                }
            }
            SelectionRepeatAction::AppendBlockEnd => {
                if let VisualSelection::Block(selection) = selection {
                    self.start_visual_insert(selection, VisualInsertKind::BlockEnd);
                }
            }
        }
    }

    /// Replay one stored binding, optionally against an explicit clipboard register.
    fn replay_registered_binding(
        &mut self,
        binding: &Binding,
        count: Option<usize>,
        register: Option<ClipboardRegister>,
    ) {
        let Some(register) = register else {
            self.execute_binding_with_count(binding, count);
            return;
        };

        // Repeat reuses the same register-targeted execution path so explicit
        // `\"+` and `\"*` bindings preserve their original side effects. A
        // missing trigger is sufficient here because operator-based register
        // changes are captured as `RepeatSource::Operator`, so this path only
        // replays direct bindings whose register-aware actions do not need one.
        self.execute_registered_binding(binding, count, register, None);
    }

    /// Resolve one stored repeat target into a concrete selection.
    fn selection_for_repeat_target(
        &self,
        target: SelectionRepeatTarget,
    ) -> Option<VisualSelection> {
        match target {
            SelectionRepeatTarget::Character { char_count } => {
                // Characterwise repeats consume the next stored width of text from
                // the cursor instead of jumping back to the old Visual span.
                let start = self.cursor.to_char_index(&self.buffer);
                let end = start
                    .saturating_add(char_count.max(1))
                    .min(self.buffer.chars_count());
                (end > start).then_some(VisualSelection::Character(SelectionRange { start, end }))
            }
            SelectionRepeatTarget::Lines { line_count } => {
                // Linewise repeats use the current line as their anchor and expand
                // downward by the recorded number of touched logical lines.
                Some(VisualSelection::Line(
                    self.current_line_range(line_count.max(1)),
                ))
            }
            SelectionRepeatTarget::Block {
                line_count,
                column_count,
            } => {
                let start_line = self.cursor.line();
                let end_line = start_line
                    .saturating_add(line_count.max(1).saturating_sub(1))
                    .min(self.buffer.lines_count().saturating_sub(1));
                Some(VisualSelection::Block(BlockSelection {
                    start_line,
                    end_line,
                    left_column: self.cursor.column(),
                    right_column: self
                        .cursor
                        .column()
                        .saturating_add(column_count.max(1).saturating_sub(1)),
                }))
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
    pub(super) fn resolve_relative_char_idx(
        &self,
        session_start_char_idx: usize,
        offset: isize,
    ) -> usize {
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
        self.visual_insert_session = None;
        self.finish_history_transaction();
        self.cursor.clamp_to_line_normal(&self.buffer);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
    }
}
