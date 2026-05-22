//! Input, action, motion, and modal-state helpers for `EditorState`.

use super::*;
use crate::dialogs::{
    CodeActionPickerState, LocationPickerState, PickerItem, PickerState, SearchPickerState,
};
use crate::navigation::WordStyle;

/// Describe one list-navigation command for a modal picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerMotion {
    Up,
    Down,
    PageUp,
    PageDown,
}

impl EditorState {
    /// Handle one normalized key input and route it through pending states and bindings.
    ///
    /// When a blockwise Visual `I` or `A` insert session is active, this also
    /// mirrors the primary cursor's Insert-mode edits onto the remaining block rows.
    pub(crate) fn handle_key(&mut self, key: Key) {
        let key = Self::normalize_key(key);
        let visual_insert_history_start = self.visual_insert_history_len();
        self.handle_key_inner(key);
        self.mirror_visual_insert_edits(visual_insert_history_start);
    }

    /// Handle one already-normalized key input and route it through editor state.
    fn handle_key_inner(&mut self, key: Key) {
        self.capture_macro_key(key);
        self.clear_status_overlay();
        self.dismiss_hover();
        if matches!(self.mode, Mode::Command(_) | Mode::Search(_)) {
            if key == Key::Esc {
                if self
                    .ignore_input_escape_cancel_until
                    .is_some_and(|until| Instant::now() <= until)
                {
                    return;
                }
                self.ignore_input_escape_cancel_until = None;
            } else {
                self.ignore_input_escape_cancel_until = None;
            }
        } else {
            self.ignore_input_escape_cancel_until = None;
        }

        // Picker dialogs own their entire key stream so they can keep query text
        // and list navigation isolated from normal-mode bindings.
        if self.handle_picker_key(key) {
            return;
        }

        // Recovery prompts block normal editing until the stale swap is either
        // restored or discarded for the currently shown file.
        if self.handle_pending_swap_recovery_key(key) {
            return;
        }

        // Soft read-only saves ask once more before a write targets the current
        // file path while the buffer remains intentionally marked read-only.
        if self.handle_pending_soft_read_only_save_key(key) {
            return;
        }

        // Highest priority: overwrite confirmation must consume input first so
        // destructive write prompts cannot be bypassed by other pending states.
        if self.handle_pending_overwrite_key(key) {
            return;
        }

        // Next: quit confirmation prompt takes precedence over navigation/editing.
        if self.handle_pending_quit_key(key) {
            return;
        }

        // Session-open confirmation mirrors quit confirmation because both flows
        // may need to save or discard dirty buffers before continuing.
        if self.handle_pending_session_open_key(key) {
            return;
        }

        // Dirty buffer-close confirmation is separate from quit confirmation so
        // `:bd` can reuse the same y/n/c flow without entangling quit state.
        if self.handle_pending_buffer_close_key(key) {
            return;
        }

        // While waiting for find/till target, consume every key until resolved/cancelled.
        if self.handle_pending_find_key(key) {
            return;
        }

        // Pending replace owns exactly one following key, similar to find/till.
        if self.handle_pending_replace_key(key) {
            return;
        }

        // Generic operators own the next key stream until one motion/object resolves.
        if self.handle_pending_operator_key(key) {
            return;
        }

        // Macro commands own the next key once `q` or `@` has requested a register.
        if self.handle_pending_macro_key(key) {
            return;
        }

        // Clipboard register prefixes mirror Vim's `"+` / `"*` flow and own the
        // next key stream until the register and command have both resolved.
        if self.handle_pending_register_key(key) {
            return;
        }

        // Then process multi-key normal-mode sequences (g*, diw/ciw/da().
        if self.handle_pending_sequence_key(key) {
            return;
        }

        // Finally, parse a fresh numeric count prefix if applicable.
        if self.handle_pending_count_key(key) {
            return;
        }

        if self.mode_uses_modal_bindings() && key == Key::Char('"') {
            self.begin_register_prefix();
            return;
        }

        // First resolve exact bindings for the current mode. This must run before
        // sequence-prefix detection so explicit single-key remaps like
        // `z = "move-right"` keep winning even when built-in `z*` sequences
        // exist. If we flipped the order, typing `z` would get stuck in a
        // pending prefix instead of executing the configured direct action.
        let binding = self.keybindings.get_binding(key, &self.mode).cloned();
        if let Some(actions) = binding.as_ref() {
            let count = self.pending_count.take();
            if let ActionBinding::Single(action) = actions
                && let Some(kind) = Self::operator_kind_for_action(*action)
            {
                let count = count.map(|value| value.clamp(1, Self::MAX_COUNT));
                self.begin_operator(kind, Some(KeyInput::from(key)), count, None);
                return;
            }
            self.execute_bound_actions(actions, count);
            return;
        }

        // Only after exact bindings fail do we consider multi-key prefixes. This
        // makes sequences an opt-in fallback instead of shadowing direct keymaps.
        if self.mode_uses_modal_bindings() {
            let key_input = KeyInput::from(key);
            if self
                .keybindings
                .starts_sequence_prefix(&self.mode, &key_input)
            {
                self.pending_sequence.clear();
                self.pending_sequence.push(key_input);
                // Exact single-key bindings win over prefixes. This keeps custom
                // remaps like `z = "move-right"` usable even after built-in `z*`
                // sequences are added.
                self.pending_sequence_count = self.pending_count.take();
                self.pending_sequence_motion_count = None;
                return;
            }
        }

        // Handle insertable characters for insert/command/search modes
        if let Some(c) = KeyBindings::is_insertable_char(key) {
            if self.mode_uses_modal_bindings() {
                // Unbound key in normal mode - ignore
                self.pending_count = None;
                return;
            }

            if self.mode == Mode::Insert {
                self.insert_char(c);
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
                self.refresh_completion_session();
                self.refresh_signature_help_session();
                self.clear_pending_auto_indent_if_cursor_left_line();
            } else {
                self.append_prompt_char(c);
            }
        }

        if self.mode_uses_modal_bindings() {
            self.pending_count = None;
        }
    }

    /// Execute one or more actions with an optional Normal-mode count prefix.
    /// Execute a borrowed action binding, repeating whole multi-action sequences for counts.
    pub(super) fn execute_actions_with_count(
        &mut self,
        actions: &ActionBinding,
        count: Option<usize>,
    ) {
        match actions {
            ActionBinding::Single(action) => {
                self.execute_action_with_count(*action, count);
            }
            ActionBinding::Multiple(actions) => {
                // Insert-style multi-action bindings such as `I` and `A` should
                // enter one insert session and let Escape replay the typed text.
                if Self::binding_uses_insert_session_count(actions) {
                    for action in actions.iter().copied() {
                        self.execute_action(action);
                    }
                    return;
                }
                let repeats = count.map_or(1, |value| value.clamp(1, Self::MAX_COUNT));
                for _ in 0..repeats {
                    for action in actions.iter().copied() {
                        self.execute_action(action);
                    }
                }
            }
        }
    }

    /// Execute one action with an optional Normal-mode count prefix.
    ///
    /// Repeat-oriented actions use capped counts, while line-targeting `G`/`gg`
    /// use the raw parsed line number (no `MAX_COUNT` cap).
    pub(super) fn execute_action_with_count(&mut self, action: Action, count: Option<usize>) {
        let Some(count) = count else {
            self.execute_action(action);
            return;
        };
        self.reset_wrapped_goal_if_needed(action);
        let raw_count = count.max(1);
        let count = raw_count.min(Self::MAX_COUNT);
        match action {
            Action::MoveLeft => {
                self.cursor.move_left_normal_by(count);
                self.finish_counted_normal_action();
            }
            Action::MoveRight => {
                self.cursor.move_right_normal_by(&self.buffer, count);
                self.finish_counted_normal_action();
            }
            Action::MoveUp => {
                self.move_up_for_current_wrap_mode_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveDown => {
                self.move_down_for_current_wrap_mode_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveDownFirstNonBlank => {
                self.move_down_first_non_blank_count(raw_count);
                self.finish_counted_normal_action();
            }
            Action::MoveWordForward => {
                self.move_word_forward_count(count, WordStyle::Small);
                self.finish_counted_normal_action();
            }
            Action::MoveBigWordForward => {
                self.move_word_forward_count(count, WordStyle::Big);
                self.finish_counted_normal_action();
            }
            Action::MoveWordBackward => {
                self.move_word_backward_count(count, WordStyle::Small);
                self.finish_counted_normal_action();
            }
            Action::MoveBigWordBackward => {
                self.move_word_backward_count(count, WordStyle::Big);
                self.finish_counted_normal_action();
            }
            Action::MoveWordEnd => {
                self.move_word_end_count(count, WordStyle::Small);
                self.finish_counted_normal_action();
            }
            Action::MoveBigWordEnd => {
                self.move_word_end_count(count, WordStyle::Big);
                self.finish_counted_normal_action();
            }
            Action::MoveWordEndBackward => {
                self.move_word_end_backward_count(count, WordStyle::Small);
                self.finish_counted_normal_action();
            }
            Action::MoveBigWordEndBackward => {
                self.move_word_end_backward_count(count, WordStyle::Big);
                self.finish_counted_normal_action();
            }
            Action::MoveParagraphForward => {
                self.move_paragraph_forward_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveParagraphBackward => {
                self.move_paragraph_backward_count(count);
                self.finish_counted_normal_action();
            }
            Action::DeleteCharAtCursor => {
                self.delete_char_at_cursor_count(count);
                self.finish_counted_normal_action();
            }
            Action::ToggleCaseAtCursor => {
                if self.mode.is_visual() {
                    self.toggle_case_visual_selection();
                } else {
                    self.toggle_case_at_cursor_count(count);
                }
                self.finish_counted_normal_action();
            }
            Action::IncrementNextNumber => {
                self.offset_next_number(count as i64);
                self.finish_counted_normal_action();
            }
            Action::DecrementNextNumber => {
                self.offset_next_number(-(count as i64));
                self.finish_counted_normal_action();
            }
            Action::JoinLines => {
                self.join_lines_count(count);
                self.finish_counted_normal_action();
            }
            Action::BeginReplaceChar => self.begin_replace_char(count),
            Action::VisualInsertBlockStart => {
                self.begin_visual_insert(VisualInsertKind::BlockStart);
            }
            Action::VisualAppendBlockEnd => self.begin_visual_insert(VisualInsertKind::BlockEnd),
            Action::YankCurrentLine => {
                self.yank_current_line_count(count);
                self.finish_counted_normal_action();
            }
            Action::BeginDeleteOperator => {
                self.begin_operator(OperatorKind::Delete, None, Some(count), None);
            }
            Action::BeginChangeOperator => {
                self.begin_operator(OperatorKind::Change, None, Some(count), None);
            }
            Action::BeginYankOperator => {
                self.begin_operator(OperatorKind::Yank, None, Some(count), None);
            }
            Action::BeginIndentOperator => {
                self.begin_operator(OperatorKind::Indent, None, Some(count), None);
            }
            Action::BeginReindentOperator => {
                self.begin_operator(OperatorKind::Reindent, None, Some(count), None);
            }
            Action::BeginDedentOperator => {
                self.begin_operator(OperatorKind::Dedent, None, Some(count), None);
            }
            Action::PasteAfterCursor => {
                self.paste_from_yank_buffer_count(PastePosition::After, count);
                self.finish_counted_normal_action();
            }
            Action::PasteBeforeCursor => {
                self.paste_from_yank_buffer_count(PastePosition::Before, count);
                self.finish_counted_normal_action();
            }
            Action::PasteClipboardAfterCursor => {
                self.request_clipboard_paste(
                    ClipboardRegister::Clipboard,
                    PastePosition::After,
                    count,
                );
                self.finish_counted_normal_action();
            }
            Action::PasteClipboardBeforeCursor => {
                self.request_clipboard_paste(
                    ClipboardRegister::Clipboard,
                    PastePosition::Before,
                    count,
                );
                self.finish_counted_normal_action();
            }
            Action::JumpOlder => {
                self.jump_backward_count(count);
                self.finish_counted_normal_action();
            }
            Action::JumpNewer => {
                self.jump_forward_count(count);
                self.finish_counted_normal_action();
            }
            Action::BeginMacroRecord => self.begin_macro_recording_action(),
            Action::BeginMacroPlayback => self.begin_macro_playback_action(count),
            Action::PageUp => {
                self.viewport
                    .page_up_by(&mut self.cursor, &self.buffer, count);
            }
            Action::ScrollLineUp => self.scroll_viewport_lines(count, MotionDirection::Up),
            Action::ScrollLineDown => self.scroll_viewport_lines(count, MotionDirection::Down),
            Action::PageDown => {
                self.viewport
                    .page_down_by(&mut self.cursor, &self.buffer, count);
            }
            Action::HalfPageUp => {
                self.viewport
                    .half_page_up_by(&mut self.cursor, &self.buffer, count);
            }
            Action::HalfPageDown => {
                self.viewport
                    .half_page_down_by(&mut self.cursor, &self.buffer, count);
            }
            Action::SearchNext => self.repeat_search_count(FindDirection::Forward, count),
            Action::SearchPrevious => self.repeat_search_count(FindDirection::Backward, count),
            Action::Undo => {
                self.undo_changes(count);
                self.finish_counted_normal_action();
            }
            Action::Redo => {
                self.redo_changes(count);
                self.finish_counted_normal_action();
            }
            Action::MoveToLastLine | Action::MoveToFirstLine => {
                self.goto_line(raw_count);
                self.cursor.clamp_to_line_normal(&self.buffer);
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::FindForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Forward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::FindBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Backward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::TillForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Forward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::TillBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Backward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::RepeatFindForward => self.repeat_find(FindRepeatDirection::Same, count),
            Action::RepeatFindBackward => self.repeat_find(FindRepeatDirection::Reversed, count),
            Action::RepeatLastChange => self.repeat_last_change(count),
            Action::MatchBracket => {
                self.goto_percent_of_file(raw_count);
                self.finish_counted_normal_action();
            }
            Action::EnterCommandMode => {
                self.pending_search_count = None;
                self.enter_command_prompt(raw_count.to_string());
            }
            Action::EnterSearchMode => {
                self.pending_search_count = Some(count);
                self.enter_search_prompt();
            }
            _ => {
                // Non-repeatable actions with a count execute once and clear the count.
                self.execute_action(action);
            }
        }
    }

    /// Normalize cursor and viewport once after count-aware normal-mode actions.
    pub(super) fn finish_counted_normal_action(&mut self) {
        self.cursor.clamp_to_line_normal(&self.buffer);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
    }

    /// Return whether the current mode uses normal-style motion and count handling.
    ///
    /// Returns `true` in Normal and Visual modes, and `false` in every mode
    /// that should skip normal-command motion/count interpretation.
    pub(crate) fn mode_uses_modal_bindings(&self) -> bool {
        self.mode.is_normal() || self.mode.is_visual()
    }

    /// Return whether an action should preserve wrapped-row column intent.
    ///
    /// Returns `true` for wrapped vertical motions that should keep the visual
    /// goal column, and `false` for every action that should clear that goal.
    pub(super) fn preserves_wrapped_goal(action: Action) -> bool {
        matches!(action, Action::MoveUp | Action::MoveDown)
    }

    /// Clear wrapped-row column intent when a different action takes over.
    pub(super) fn reset_wrapped_goal_if_needed(&mut self, action: Action) {
        if !self.soft_wrap_enabled() || !Self::preserves_wrapped_goal(action) {
            self.desired_visual_column = None;
        }
    }

    /// Return whether this action needs the generic post-action visibility sync.
    ///
    /// Returns `true` when the shared cursor/viewport visibility pass should run
    /// after the action, and `false` for actions that already handled it directly.
    pub(super) fn action_needs_visibility_sync(action: Action) -> bool {
        !matches!(
            action,
            Action::PageUp
                | Action::PageDown
                | Action::HalfPageUp
                | Action::HalfPageDown
                | Action::ScrollLineUp
                | Action::ScrollLineDown
                | Action::AlignViewportTop
                | Action::AlignViewportCenter
                | Action::AlignViewportBottom
                | Action::ExecuteCommand
        )
    }

    /// Return whether one multi-action binding should count by replaying insert text on Escape.
    ///
    /// Returns `true` for insert-entry bindings whose numeric prefix belongs to
    /// the later insert session, and `false` for every binding that should use
    /// the ordinary whole-sequence repeater.
    pub(super) fn binding_uses_insert_session_count(actions: &[Action]) -> bool {
        matches!(
            actions,
            [Action::MoveFirstNonBlank, Action::EnterInsertMode]
                | [Action::MoveLineEnd, Action::InsertAfterCursor]
        )
    }

    /// Scroll the viewport and nudge the cursor only when margin enforcement requires it.
    pub(super) fn scroll_viewport_lines(&mut self, count: usize, direction: MotionDirection) {
        match direction {
            MotionDirection::Up => self.viewport.scroll_up(count),
            MotionDirection::Down => self.viewport.scroll_down(count, &self.buffer),
        }

        // Viewport-only motions should preserve the scroll delta, so adjust the
        // cursor into the safe band instead of running a generic visibility pass.
        if self.soft_wrap_enabled() {
            self.clamp_cursor_to_wrapped_margin();
        } else {
            self.clamp_cursor_to_line_margin();
        }
    }

    /// Nudge an unwrapped cursor back inside the current scroll-margin-safe band.
    pub(super) fn clamp_cursor_to_line_margin(&mut self) {
        let (top_line, bottom_line) = self.viewport.line_margin_limits();
        if self.cursor.line() < top_line {
            self.cursor
                .move_down_normal_by(&self.buffer, top_line - self.cursor.line());
        } else if self.cursor.line() > bottom_line {
            self.cursor
                .move_up_normal_by(&self.buffer, self.cursor.line() - bottom_line);
        }
    }

    /// Nudge a wrapped cursor back inside the current scroll-margin-safe band.
    pub(super) fn clamp_cursor_to_wrapped_margin(&mut self) {
        let width = self.viewport.width().max(1);
        let line_len = self.buffer.line_len(self.cursor.line());
        let current_visual = soft_wrap::visual_cursor(
            self.cursor.column(),
            line_len,
            width,
            self.mode_uses_modal_bindings(),
            self.cursor.line(),
        );
        let (top_limit, bottom_limit) = self.viewport.wrapped_margin_limits(&self.buffer);

        // Wrapped motions adjust by rendered rows so the cursor lands exactly on
        // the nearest allowed row while preserving its visual column intent.
        if current_visual.position < top_limit {
            let rows = soft_wrap::visual_rows_between(
                current_visual.position,
                top_limit,
                &self.buffer,
                width,
            );
            self.move_wrapped_rows(rows, MotionDirection::Down);
        } else if current_visual.position > bottom_limit {
            let rows = soft_wrap::visual_rows_between(
                bottom_limit,
                current_visual.position,
                &self.buffer,
                width,
            );
            self.move_wrapped_rows(rows, MotionDirection::Up);
        }
    }

    /// Execute one upward movement using the active wrap mode.
    pub(super) fn move_up_for_current_wrap_mode(&mut self) {
        if self.soft_wrap_enabled() {
            self.move_up_wrapped();
        } else if self.mode_uses_modal_bindings() {
            self.cursor.move_up_normal(&self.buffer);
        } else {
            self.cursor.move_up(&self.buffer);
        }
    }

    /// Execute one downward movement using the active wrap mode.
    pub(super) fn move_down_for_current_wrap_mode(&mut self) {
        if self.soft_wrap_enabled() {
            self.move_down_wrapped();
        } else if self.mode_uses_modal_bindings() {
            self.cursor.move_down_normal(&self.buffer);
        } else {
            self.cursor.move_down(&self.buffer);
        }
    }

    /// Execute an upward counted movement using the active wrap mode.
    pub(super) fn move_up_for_current_wrap_mode_count(&mut self, count: usize) {
        if self.soft_wrap_enabled() {
            self.move_up_wrapped_count(count);
        } else {
            self.cursor.move_up_normal_by(&self.buffer, count);
        }
    }

    /// Execute a downward counted movement using the active wrap mode.
    pub(super) fn move_down_for_current_wrap_mode_count(&mut self, count: usize) {
        if self.soft_wrap_enabled() {
            self.move_down_wrapped_count(count);
        } else {
            self.cursor.move_down_normal_by(&self.buffer, count);
        }
    }

    /// Execute one logical action without a count prefix.
    ///
    /// NOTE: when adding or changing action behavior, verify whether
    /// `execute_action_with_count` needs the same update for counted execution.
    pub(super) fn execute_action(&mut self, action: Action) {
        self.reset_wrapped_goal_if_needed(action);
        match action {
            // Navigation
            Action::MoveLeft => {
                if self.mode_uses_modal_bindings() {
                    self.cursor.move_left_normal();
                } else {
                    self.cursor.move_left(&self.buffer);
                }
            }
            Action::MoveRight => {
                if self.mode_uses_modal_bindings() {
                    self.cursor.move_right_normal(&self.buffer);
                } else {
                    self.cursor.move_right(&self.buffer);
                }
            }
            Action::MoveUp => {
                self.move_up_for_current_wrap_mode();
            }
            Action::MoveDown => {
                self.move_down_for_current_wrap_mode();
            }
            Action::MoveDownFirstNonBlank => self.move_down_first_non_blank_count(1),
            Action::MoveWordForward => self.move_word_forward(WordStyle::Small),
            Action::MoveBigWordForward => self.move_word_forward(WordStyle::Big),
            Action::MoveWordBackward => self.move_word_backward(WordStyle::Small),
            Action::MoveBigWordBackward => self.move_word_backward(WordStyle::Big),
            Action::MoveWordEnd => self.move_word_end(WordStyle::Small),
            Action::MoveBigWordEnd => self.move_word_end(WordStyle::Big),
            Action::MoveWordEndBackward => self.move_word_end_backward(WordStyle::Small),
            Action::MoveBigWordEndBackward => self.move_word_end_backward(WordStyle::Big),
            Action::MoveParagraphForward => self.move_paragraph_forward(),
            Action::MoveParagraphBackward => self.move_paragraph_backward(),
            Action::MoveLineStart => self.cursor.move_to_line_start(),
            Action::MoveLineEnd => self.cursor.move_to_line_end(&self.buffer),
            Action::MovePastLineEnd => self.cursor.move_past_line_end(&self.buffer),
            Action::MoveFirstNonBlank => self.move_first_non_blank(),
            Action::MoveToFirstLine => self.move_to_first_line(),
            Action::MoveToLastLine => self.move_to_last_line(),
            Action::AlignViewportTop => self.align_viewport_top(),
            Action::AlignViewportCenter => self.align_viewport_center(),
            Action::AlignViewportBottom => self.align_viewport_bottom(),
            Action::ScrollLineUp => self.scroll_viewport_lines(1, MotionDirection::Up),
            Action::ScrollLineDown => self.scroll_viewport_lines(1, MotionDirection::Down),
            Action::PageUp => self.viewport.page_up(&mut self.cursor, &self.buffer),
            Action::PageDown => self.viewport.page_down(&mut self.cursor, &self.buffer),
            Action::HalfPageUp => self.viewport.half_page_up(&mut self.cursor, &self.buffer),
            Action::HalfPageDown => self.viewport.half_page_down(&mut self.cursor, &self.buffer),
            Action::FindForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Forward,
                    count: 1,
                });
            }
            Action::FindBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Backward,
                    count: 1,
                });
            }
            Action::TillForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Forward,
                    count: 1,
                });
            }
            Action::TillBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Backward,
                    count: 1,
                });
            }
            Action::RepeatFindForward => self.repeat_find(FindRepeatDirection::Same, 1),
            Action::RepeatFindBackward => self.repeat_find(FindRepeatDirection::Reversed, 1),
            Action::RepeatLastChange => self.repeat_last_change(1),
            Action::JumpOlder => self.jump_backward(),
            Action::JumpNewer => self.jump_forward(),
            Action::MatchBracket => self.jump_to_matching_delimiter(),
            Action::GotoDefinition => self.request_navigation(NavigationKind::Definition),
            Action::GotoReferences => self.request_navigation(NavigationKind::References),
            Action::GotoFileUnderCursor => self.goto_file_under_cursor(),
            Action::GotoFileUnderCursorAtPosition => self.goto_file_under_cursor_at_position(),
            Action::GotoAlternateFile => self.goto_alternate_file(),
            Action::GotoLastModification => self.goto_last_modification(),
            Action::ShowHover => self.request_hover(),
            Action::OpenCodeActions => self.request_code_actions(),
            Action::OpenDiagnosticsPicker => self.open_diagnostics_picker(),
            Action::NextDiagnostic => self.goto_next_diagnostic(),
            Action::PrevDiagnostic => self.goto_prev_diagnostic(),
            Action::PromptRenameSymbol => {
                self.enter_command_prompt(self.prefilled_rename_command());
            }

            // Mode switching
            Action::EnterInsertMode => {
                self.begin_history_transaction();
                self.enter_insert_mode();
            }
            Action::EnterVisualMode => self.enter_visual_mode(VisualKind::Character),
            Action::EnterVisualLineMode => self.enter_visual_mode(VisualKind::Line),
            Action::EnterVisualBlockMode => self.enter_visual_mode(VisualKind::Block),
            Action::SwapVisualAnchor => self.swap_visual_anchor(),
            Action::RecreateLastSelection => self.recreate_last_selection(),
            Action::VisualInsertBlockStart => {
                self.begin_visual_insert(VisualInsertKind::BlockStart);
            }
            Action::VisualAppendBlockEnd => self.begin_visual_insert(VisualInsertKind::BlockEnd),
            Action::InsertAfterCursor => self.insert_after_cursor(),
            Action::OpenLineBelow => self.open_line_below(),
            Action::OpenLineAbove => self.open_line_above(),
            Action::EnterCommandMode => {
                self.pending_search_count = None;
                self.enter_command_prompt(String::new());
            }
            Action::PromptGrep => {
                self.pending_search_count = None;
                self.enter_command_prompt("grep ".to_string());
            }
            Action::GrepWordUnderCursor => {
                self.pending_search_count = None;
                self.grep_word_under_cursor();
            }
            Action::EnterSearchMode => {
                self.pending_search_count = None;
                self.enter_search_prompt();
            }
            Action::OpenBufferSwitcher => self.open_buffer_switcher(),
            Action::OpenFilePicker => self.open_file_picker(),
            Action::ExitToNormalMode => self.exit_to_normal_mode(),
            Action::HideSearchHighlighting => self.hide_search_highlighting(),
            Action::SearchNext => self.repeat_search(FindDirection::Forward),
            Action::SearchPrevious => self.repeat_search(FindDirection::Backward),
            Action::Undo => self.undo_changes(1),
            Action::Redo => self.redo_changes(1),
            Action::SaveCurrentFile => self.request_save_current(
                OverwriteBehavior::ConfirmIfDifferentPath,
                PostSaveAction::StayOpen,
            ),
            Action::SaveCurrentFileAndQuit => self.request_save_current(
                OverwriteBehavior::ConfirmIfDifferentPath,
                PostSaveAction::QuitOnSuccess,
            ),
            Action::UpdateCurrentFileAndQuit => {
                self.update_current_file(PostSaveAction::QuitOnSuccess)
            }
            Action::RequestFullRedraw => self.request_full_redraw(),

            // Insert mode
            Action::ToggleCaseAtCursor => {
                if self.mode.is_visual() {
                    self.toggle_case_visual_selection();
                } else {
                    self.toggle_case_at_cursor_count(1);
                }
            }
            Action::DeleteToLineEnd => self.delete_to_line_end(),
            Action::ChangeToLineEnd => self.change_to_line_end(),
            Action::IncrementNextNumber => self.offset_next_number(1),
            Action::DecrementNextNumber => self.offset_next_number(-1),
            Action::JoinLines => self.join_lines_count(1),
            Action::BeginReplaceChar => self.begin_replace_char(1),
            Action::SearchWordUnderCursor => self.search_word_under_cursor(),
            Action::DeleteCharBackward => self.delete_char_backward(),
            Action::DeleteCharForward => self.delete_char_forward(),
            Action::CompletionSelectUp => {
                // When no completion popup is active, keep the insert-mode Up key as
                // ordinary cursor motion instead of swallowing the navigation key.
                if !self.move_completion_selection(CompletionDirection::Up) {
                    self.move_up_for_current_wrap_mode();
                }
            }
            Action::CompletionSelectDown => {
                // This mirrors the Up-key fallback so Ctrl+N/Down still move the
                // cursor normally outside an active completion session.
                if !self.move_completion_selection(CompletionDirection::Down) {
                    self.move_down_for_current_wrap_mode();
                }
            }
            Action::DeleteCharAtCursor => self.delete_char_at_cursor(),
            Action::DeleteWordBackward => self.delete_word_backward(),
            Action::DeleteToLineStart => self.delete_to_line_start(),
            Action::InsertNewline => self.insert_newline(),
            Action::DeleteSelection => self.delete_visual_selection(false),
            Action::IndentSelection => self.indent_visual_selection(),
            Action::ReindentSelection => self.reindent_visual_selection(),
            Action::DedentSelection => self.dedent_visual_selection(),
            Action::ChangeSelection => self.delete_visual_selection(true),
            Action::YankSelection => self.yank_visual_selection(),
            Action::YankCurrentLine => self.yank_current_line(),
            Action::PasteAfterCursor => self.paste_from_yank_buffer(PastePosition::After),
            Action::PasteBeforeCursor => self.paste_from_yank_buffer(PastePosition::Before),
            Action::PasteClipboardAfterCursor => {
                self.request_clipboard_paste(ClipboardRegister::Clipboard, PastePosition::After, 1)
            }
            Action::PasteClipboardBeforeCursor => {
                self.request_clipboard_paste(ClipboardRegister::Clipboard, PastePosition::Before, 1)
            }
            Action::BeginDeleteOperator => {
                self.begin_operator(OperatorKind::Delete, None, None, None)
            }
            Action::BeginChangeOperator => {
                self.begin_operator(OperatorKind::Change, None, None, None)
            }
            Action::BeginYankOperator => self.begin_operator(OperatorKind::Yank, None, None, None),
            Action::BeginIndentOperator => {
                self.begin_operator(OperatorKind::Indent, None, None, None)
            }
            Action::BeginReindentOperator => {
                self.begin_operator(OperatorKind::Reindent, None, None, None)
            }
            Action::BeginDedentOperator => {
                self.begin_operator(OperatorKind::Dedent, None, None, None)
            }
            Action::IndentCurrentLine => self.indent_current_line_insert_mode(),
            Action::DedentCurrentLine => self.dedent_current_line_insert_mode(),
            Action::BeginMacroRecord => self.begin_macro_recording_action(),
            Action::BeginMacroPlayback => self.begin_macro_playback_action(1),

            // Command/Search mode
            Action::ExecuteCommand => self.execute_command(),
            Action::CancelCommand => self.cancel_prompt_input(),
            Action::PromptHistoryPrev => {
                self.recall_prompt_history_previous(PromptHistoryScope::MatchingPrefix);
            }
            Action::PromptHistoryNext => {
                self.recall_prompt_history_next(PromptHistoryScope::MatchingPrefix);
            }
            Action::PromptHistoryPrevFull => {
                self.recall_prompt_history_previous(PromptHistoryScope::Full);
            }
            Action::PromptHistoryNextFull => {
                self.recall_prompt_history_next(PromptHistoryScope::Full);
            }
            Action::DeleteInputChar => self.delete_input_char(),
            Action::DeleteInputCharForward => self.delete_input_char_forward(),
            Action::DeleteInputWordBackward => self.delete_input_word_backward(),
            Action::DeleteInputToStart => self.delete_input_to_start(),
            Action::DeleteInputToEnd => self.delete_input_to_end(),
            Action::MoveInputStart => self.move_input_start(),
            Action::MoveInputEnd => self.move_input_end(),
            Action::MoveInputLeft => self.move_input_left(),
            Action::MoveInputRight => self.move_input_right(),
            Action::MoveInputWordLeft => self.move_input_word_left(),
            Action::MoveInputWordRight => self.move_input_word_right(),
        }

        // In normal mode, cursor must stay on a real character for non-empty lines.
        if self.mode_uses_modal_bindings() {
            self.cursor.clamp_to_line_normal(&self.buffer);
        } else {
            self.pending_sequence.clear();
            self.pending_sequence_count = None;
            self.pending_sequence_motion_count = None;
            self.pending_operator = None;
            self.pending_macro = None;
            self.pending_register = None;
            self.pending_find = None;
            self.pending_replace = None;
        }

        // Page-style motions already compute their own viewport placement, so a
        // second generic visibility pass would shrink the intended scroll delta.
        if Self::action_needs_visibility_sync(action) {
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        }
        self.sync_completion_after_action(action);
        self.sync_signature_help_after_action(action);
        self.sync_visible_match_for_viewport();
        self.clear_pending_auto_indent_if_cursor_left_line();
    }
}

impl EditorState {
    /// Handle keys while either picker dialog is active.
    ///
    /// Returns `true` when the picker consumed the key and normal editor
    /// keybinding dispatch should stop, or `false` when no picker is active.
    fn handle_picker_key(&mut self, key: Key) -> bool {
        let Some(picker) = self.active_picker_kind() else {
            return false;
        };

        match key {
            Key::Esc => self.close_picker(picker),
            Key::Char('\n') => self.confirm_picker_selection(picker),
            Key::Up | Key::Ctrl('p') => self.move_picker_selection(picker, PickerMotion::Up),
            Key::Down | Key::Ctrl('n') => {
                self.move_picker_selection(picker, PickerMotion::Down);
            }
            Key::PageUp => self.move_picker_selection(picker, PickerMotion::PageUp),
            Key::PageDown => self.move_picker_selection(picker, PickerMotion::PageDown),
            // Query-editing keys reuse the shared input buffer and then resync matches.
            Key::Backspace | Key::Ctrl('h') => {
                self.delete_input_char();
                self.refresh_picker_matches(picker);
            }
            Key::Delete | Key::Ctrl('d') => {
                self.delete_input_char_forward();
                self.refresh_picker_matches(picker);
            }
            Key::Ctrl('w') => {
                self.delete_input_word_backward();
                self.refresh_picker_matches(picker);
            }
            Key::Ctrl('u') => {
                self.delete_input_to_start();
                self.refresh_picker_matches(picker);
            }
            Key::Ctrl('k') => {
                self.delete_input_to_end();
                self.refresh_picker_matches(picker);
            }
            Key::Ctrl('a') | Key::Home => self.move_input_start(),
            Key::Ctrl('e') | Key::End => self.move_input_end(),
            Key::Ctrl('b') | Key::Left => self.move_input_left(),
            Key::Ctrl('f') | Key::Right => self.move_input_right(),
            Key::Alt('b') => self.move_input_word_left(),
            Key::Alt('d') => {
                self.delete_input_word_forward();
                self.refresh_picker_matches(picker);
            }
            Key::Alt('f') => self.move_input_word_right(),
            _ => {
                if let Some(c) = KeyBindings::is_insertable_char(key) {
                    self.mode.append_char(c);
                    self.refresh_picker_matches(picker);
                }
            }
        }

        true
    }

    /// Close the active picker without applying a selection.
    fn close_picker(&mut self, picker: PickerKind) {
        match picker {
            PickerKind::BufferSwitch => self.close_buffer_switcher(),
            PickerKind::FilePicker => self.close_file_picker(),
            PickerKind::SearchPicker => self.close_search_picker(),
            PickerKind::LocationPicker => self.close_location_picker(),
            PickerKind::DiagnosticPicker => self.close_diagnostics_picker(),
            PickerKind::CodeActionPicker => self.close_code_action_picker(),
        }
    }

    /// Confirm the active picker selection, if one is available.
    fn confirm_picker_selection(&mut self, picker: PickerKind) {
        match picker {
            PickerKind::BufferSwitch => self.confirm_buffer_switcher_selection(),
            PickerKind::FilePicker => self.confirm_file_picker_selection(),
            PickerKind::SearchPicker => self.confirm_search_picker_selection(),
            PickerKind::LocationPicker => self.confirm_location_picker_selection(),
            PickerKind::DiagnosticPicker => self.confirm_diagnostics_picker_selection(),
            PickerKind::CodeActionPicker => self.confirm_code_action_picker_selection(),
        }
    }

    /// Move one shared picker list according to one navigation command.
    fn move_picker_state<T: PickerItem>(
        picker: Option<&mut PickerState<T>>,
        motion: PickerMotion,
        page_step: usize,
    ) {
        let Some(picker) = picker else {
            return;
        };
        match motion {
            PickerMotion::Up => picker.move_up(),
            PickerMotion::Down => picker.move_down(),
            PickerMotion::PageUp => picker.move_page_up(page_step),
            PickerMotion::PageDown => picker.move_page_down(page_step),
        }
    }

    /// Move the active picker selection according to one navigation command.
    fn move_picker_selection(&mut self, picker: PickerKind, motion: PickerMotion) {
        // Page motions need the popup height so both pickers keep the same viewport step.
        let page_step = crate::render::picker_popup_page_step(self.viewport.height());
        match picker {
            PickerKind::BufferSwitch => Self::move_picker_state(
                self.buffer_switch
                    .as_mut()
                    .map(BufferSwitchState::picker_mut),
                motion,
                page_step,
            ),
            PickerKind::FilePicker => Self::move_picker_state(
                self.file_picker.as_mut().map(FilePickerState::picker_mut),
                motion,
                page_step,
            ),
            PickerKind::SearchPicker => Self::move_picker_state(
                self.search_picker
                    .as_mut()
                    .map(SearchPickerState::picker_mut),
                motion,
                page_step,
            ),
            PickerKind::LocationPicker => Self::move_picker_state(
                self.location_picker
                    .as_mut()
                    .map(LocationPickerState::picker_mut),
                motion,
                page_step,
            ),
            PickerKind::DiagnosticPicker => Self::move_picker_state(
                self.diagnostic_picker
                    .as_mut()
                    .map(DiagnosticPickerState::picker_mut),
                motion,
                page_step,
            ),
            PickerKind::CodeActionPicker => Self::move_picker_state(
                self.code_action_picker
                    .as_mut()
                    .map(CodeActionPickerState::picker_mut),
                motion,
                page_step,
            ),
        }
    }

    /// Refresh the current picker matches after the query text changes.
    fn refresh_picker_matches(&mut self, picker: PickerKind) {
        match (picker, &self.mode) {
            (PickerKind::BufferSwitch, Mode::BufferSwitch(input)) => {
                if let Some(picker) = &mut self.buffer_switch {
                    picker.sync_query(input.text());
                }
            }
            (PickerKind::FilePicker, Mode::FilePicker(input)) => {
                if let Some(picker) = &mut self.file_picker {
                    picker.sync_query(input.text());
                }
            }
            (PickerKind::SearchPicker, Mode::SearchPicker(input)) => {
                if let Some(picker) = &mut self.search_picker {
                    picker.sync_query(input.text());
                }
            }
            (PickerKind::LocationPicker, Mode::LocationPicker(input)) => {
                if let Some(picker) = &mut self.location_picker {
                    picker.sync_query(input.text());
                }
            }
            (PickerKind::DiagnosticPicker, Mode::DiagnosticPicker(input)) => {
                if let Some(picker) = &mut self.diagnostic_picker {
                    picker.sync_query(input.text());
                }
            }
            (PickerKind::CodeActionPicker, Mode::CodeActionPicker(input)) => {
                if let Some(picker) = &mut self.code_action_picker {
                    picker.sync_query(input.text());
                }
            }
            _ => {}
        }
    }

    /// Move the cursor by wrapped screen rows instead of buffer lines.
    pub(super) fn move_wrapped_rows(&mut self, count: usize, direction: MotionDirection) {
        let width = self.viewport.width().max(1);
        let normal_mode = self.mode_uses_modal_bindings();
        let line_len = self.buffer.line_len(self.cursor.line());
        let current_visual = soft_wrap::visual_cursor(
            self.cursor.column(),
            line_len,
            width,
            normal_mode,
            self.cursor.line(),
        );
        let desired_visual_column = self.desired_visual_column.unwrap_or(current_visual.column);
        let mut target_position = current_visual.position;

        // Wrapped-row movement is bounded by the requested count and shares the
        // same stepping primitives as wrapped rendering and viewport scrolling.
        match direction {
            MotionDirection::Down => {
                target_position =
                    soft_wrap::advance_visual_position(target_position, &self.buffer, width, count);
            }
            MotionDirection::Up => {
                target_position =
                    soft_wrap::retreat_visual_position(target_position, &self.buffer, width, count);
            }
        }

        let target_len = self.buffer.line_len(target_position.line);
        let target_column = soft_wrap::buffer_column_for_visual_column(
            target_position.row,
            desired_visual_column,
            target_len,
            width,
            normal_mode,
        );
        self.cursor = Cursor::new(target_position.line, target_column);
        self.desired_visual_column = Some(desired_visual_column);
    }

    /// Move up by one wrapped screen row.
    pub(super) fn move_up_wrapped(&mut self) {
        self.move_wrapped_rows(1, MotionDirection::Up);
    }

    /// Move down by one wrapped screen row.
    pub(super) fn move_down_wrapped(&mut self) {
        self.move_wrapped_rows(1, MotionDirection::Down);
    }

    /// Move up by `count` wrapped screen rows.
    pub(super) fn move_up_wrapped_count(&mut self, count: usize) {
        self.move_wrapped_rows(count, MotionDirection::Up);
    }

    /// Move down by `count` wrapped screen rows.
    pub(super) fn move_down_wrapped_count(&mut self, count: usize) {
        self.move_wrapped_rows(count, MotionDirection::Down);
    }

    /// Move to the next word or WORD start using `style`.
    pub(super) fn move_word_forward(&mut self, style: WordStyle) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_next_word_start_with_style(&self.buffer, char_idx, style);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `w`/`W`-style motion repeatedly while avoiding per-step viewport work.
    pub(super) fn move_word_forward_count(&mut self, count: usize, style: WordStyle) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_forward(style);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    /// Move to the previous word or WORD start using `style`.
    pub(super) fn move_word_backward(&mut self, style: WordStyle) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_prev_word_start_with_style(&self.buffer, char_idx, style);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `b`/`B`-style motion repeatedly while avoiding per-step viewport work.
    pub(super) fn move_word_backward_count(&mut self, count: usize, style: WordStyle) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_backward(style);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    /// Move to the end of the current or next word/WORD using `style`.
    pub(super) fn move_word_end(&mut self, style: WordStyle) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_word_end_with_style(&self.buffer, char_idx, style);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `e`/`E`-style motion repeatedly while avoiding per-step viewport work.
    pub(super) fn move_word_end_count(&mut self, count: usize, style: WordStyle) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_end(style);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    /// Move to the end of the previous word or WORD using `style`.
    pub(super) fn move_word_end_backward(&mut self, style: WordStyle) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = match style {
            WordStyle::Small => find_prev_word_end(&self.buffer, char_idx),
            WordStyle::Big => find_prev_word_end_with_style(&self.buffer, char_idx, style),
        };
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `ge`-style motion repeatedly while avoiding per-step viewport work.
    pub(super) fn move_word_end_backward_count(&mut self, count: usize, style: WordStyle) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_end_backward(style);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    pub(super) fn move_paragraph_forward(&mut self) {
        let target_line = find_next_paragraph_line(&self.buffer, self.cursor.line());
        self.cursor = Cursor::new(target_line, self.cursor.desired_column());
    }

    /// Apply `}` paragraph motion repeatedly while preserving desired column.
    pub(super) fn move_paragraph_forward_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.line();
            self.move_paragraph_forward();
            if self.cursor.line() == before {
                break;
            }
        }
    }

    pub(super) fn move_paragraph_backward(&mut self) {
        let target_line = find_prev_paragraph_line(&self.buffer, self.cursor.line());
        self.cursor = Cursor::new(target_line, self.cursor.desired_column());
    }

    /// Apply `{` paragraph motion repeatedly while preserving desired column.
    pub(super) fn move_paragraph_backward_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.line();
            self.move_paragraph_backward();
            if self.cursor.line() == before {
                break;
            }
        }
    }

    /// Move down `count - 1` lines and land on the first non-blank column.
    pub(super) fn move_down_first_non_blank_count(&mut self, count: usize) {
        if count > 1 {
            self.move_down_for_current_wrap_mode_count(count - 1);
        }
        self.move_first_non_blank();
    }

    pub(super) fn move_first_non_blank(&mut self) {
        if let Some(line) = self.buffer.line(self.cursor.line()) {
            let mut col = 0;
            for c in line.chars() {
                if !c.is_whitespace() {
                    break;
                }
                col += 1;
            }
            self.cursor.set_column(col);
        }
    }

    pub(super) fn move_to_last_line(&mut self) {
        let last_line = self.buffer.lines_count().saturating_sub(1);
        if !self.record_jump_origin_for_destination(&self.file_path.clone(), last_line, 0) {
            return;
        }
        self.cursor = Cursor::new(last_line, 0);
    }

    pub(super) fn move_to_first_line(&mut self) {
        let target_column = self.cursor.desired_column();
        if !self.record_jump_origin_for_destination(&self.file_path.clone(), 0, target_column) {
            return;
        }
        self.cursor = Cursor::new(0, target_column);
    }

    /// Place the current cursor row at the top of the viewport.
    pub(super) fn align_viewport_top(&mut self) {
        self.viewport.align_cursor_top(&self.cursor, &self.buffer);
    }

    /// Place the current cursor row at the center of the viewport.
    pub(super) fn align_viewport_center(&mut self) {
        self.viewport
            .align_cursor_center(&self.cursor, &self.buffer);
    }

    /// Place the current cursor row at the bottom of the viewport.
    pub(super) fn align_viewport_bottom(&mut self) {
        self.viewport
            .align_cursor_bottom(&self.cursor, &self.buffer);
    }

    /// Enter visual mode or toggle/switch between the supported visual variants.
    pub(super) fn enter_visual_mode(&mut self, kind: VisualKind) {
        match self.mode {
            Mode::Visual(current) if current == kind => self.exit_visual_mode(),
            Mode::Visual(_) => self.mode = Mode::Visual(kind),
            _ => {
                self.visual_anchor = Some(self.cursor.clone());
                self.mode = Mode::Visual(kind);
            }
        }
    }

    /// Leave visual mode and clear any active selection anchor.
    pub(super) fn exit_visual_mode(&mut self) {
        self.last_visual_selection = self.current_visual_selection();
        self.clear_visual_mode(Mode::Normal);
    }

    /// Swap the active cursor with the stored visual anchor.
    pub(super) fn swap_visual_anchor(&mut self) {
        let Some(anchor) = self.visual_anchor.as_mut() else {
            return;
        };
        std::mem::swap(anchor, &mut self.cursor);
    }

    /// Clear any active visual selection and switch to the requested next mode.
    pub(super) fn clear_visual_mode(&mut self, next_mode: Mode) {
        self.visual_anchor = None;
        self.mode = next_mode;
    }

    /// Capture the active visual selection so it can later be recreated by `gv`.
    pub(super) fn current_visual_selection(&self) -> Option<LastVisualSelection> {
        let anchor = self.visual_anchor.as_ref()?;
        let kind = match self.mode {
            Mode::Visual(kind) => kind,
            _ => return None,
        };
        let anchor_char_idx = anchor.to_char_index(&self.buffer);
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        let start_char_idx = anchor_char_idx.min(cursor_char_idx);
        let end_char_idx = anchor_char_idx.max(cursor_char_idx).saturating_add(1);
        Some(LastVisualSelection {
            start_char_idx,
            end_char_idx: end_char_idx.min(self.buffer.chars_count()),
            line_count: anchor.line().abs_diff(self.cursor.line()) + 1,
            cursor_at_start: cursor_char_idx <= anchor_char_idx,
            anchor_char_idx,
            cursor_char_idx,
            kind,
        })
    }

    /// Clear any active visual selection and switch into insert mode.
    pub(super) fn enter_insert_mode(&mut self) {
        self.visual_insert_session = None;
        self.clear_visual_mode(Mode::Insert);
    }

    /// Leave insert or visual mode and restore Vim-like normal-mode cursor placement.
    pub(super) fn exit_to_normal_mode(&mut self) {
        let undo_depth_before = self.undo_stack.len();
        self.last_visual_selection = self.current_visual_selection();
        if self.mode == Mode::Insert {
            self.cleanup_pending_auto_indent_on_exit();
        }
        self.apply_counted_insert_session_repeats();
        if self.mode == Mode::Insert && self.cursor.column() > 0 {
            self.cursor.move_left(&self.buffer);
        }
        self.dismiss_completion_session(false);
        self.clear_visual_mode(Mode::Normal);
        self.visual_insert_session = None;
        self.finish_history_transaction();
        self.capture_completed_insert_repeat(undo_depth_before);
        self.sync_search_highlights_for_viewport();
    }

    /// Re-enter visual mode with the most recently remembered selection.
    pub(super) fn recreate_last_selection(&mut self) {
        let Some(selection) = self.last_visual_selection else {
            return;
        };

        let max_char_idx = self.buffer.chars_count();
        let (anchor, cursor) = match selection.kind {
            VisualKind::Character => {
                if max_char_idx == 0 {
                    return;
                }
                // Characterwise selections are stored as an exclusive range, so
                // recreating the cursor endpoint must step back one char from the
                // saved end while still clamping into the current buffer.
                let start_char_idx = selection.start_char_idx.min(max_char_idx.saturating_sub(1));
                let end_char_idx = selection
                    .end_char_idx
                    .max(start_char_idx.saturating_add(1))
                    .min(max_char_idx);
                let start = Cursor::from_char_index(&self.buffer, start_char_idx);
                let end = Cursor::from_char_index(&self.buffer, end_char_idx.saturating_sub(1));
                // `gv` should preserve which edge held the cursor so motions such
                // as `o` behave the same after the selection is recreated.
                if selection.cursor_at_start {
                    (end, start)
                } else {
                    (start, end)
                }
            }
            VisualKind::Line => {
                // Linewise selections expand to whole lines, so rebuilding them
                // converts the saved endpoints back into line numbers and places
                // both endpoints at column zero.
                let start_line = if max_char_idx == 0 {
                    0
                } else {
                    self.buffer
                        .char_to_line(selection.start_char_idx.min(max_char_idx.saturating_sub(1)))
                };
                let end_line = if max_char_idx == 0 {
                    0
                } else {
                    self.buffer.char_to_line(
                        selection
                            .end_char_idx
                            .saturating_sub(1)
                            .min(max_char_idx.saturating_sub(1)),
                    )
                };
                let start = Cursor::new(start_line, 0);
                let end = Cursor::new(end_line, 0);
                if selection.cursor_at_start {
                    (end, start)
                } else {
                    (start, end)
                }
            }
            VisualKind::Block => {
                let anchor = Cursor::from_char_index(
                    &self.buffer,
                    selection
                        .anchor_char_idx
                        .min(max_char_idx.saturating_sub(1)),
                );
                let cursor = Cursor::from_char_index(
                    &self.buffer,
                    selection
                        .cursor_char_idx
                        .min(max_char_idx.saturating_sub(1)),
                );
                (anchor, cursor)
            }
        };
        self.visual_anchor = Some(anchor);
        self.cursor = cursor;
        self.mode = Mode::Visual(selection.kind);
    }

    pub(super) fn begin_find_motion(&mut self, motion: FindMotion) {
        self.pending_sequence.clear();
        self.pending_sequence_count = None;
        self.pending_sequence_motion_count = None;
        self.pending_operator = None;
        self.pending_find = Some(motion);
    }

    /// Consume one key while a find/till motion is pending.
    ///
    /// Returns `true` when this function consumed the key.
    pub(super) fn handle_pending_find_key(&mut self, key: Key) -> bool {
        let Some(motion) = self.pending_find else {
            return false;
        };
        if !self.mode_uses_modal_bindings() {
            self.pending_find = None;
            return false;
        }

        if matches!(key, Key::Esc) {
            self.pending_find = None;
            return true;
        }

        if let Some(target) = KeyBindings::is_insertable_char(key) {
            self.pending_find = None;
            self.apply_find_motion(motion, target, LastFindUpdate::Store);
            self.finish_counted_normal_action();
        }

        // While waiting for find target, consume all keys to avoid accidental mode switches.
        true
    }

    /// Consume one key while a multi-key normal-mode sequence is pending.
    ///
    /// Returns `true` when this function consumed the key.
    pub(super) fn handle_pending_sequence_key(&mut self, key: Key) -> bool {
        if !self.mode_uses_modal_bindings() || self.pending_sequence.is_empty() {
            return false;
        }

        if matches!(key, Key::Esc) {
            self.pending_sequence.clear();
            self.pending_sequence_count = None;
            self.pending_sequence_motion_count = None;
            return true;
        }

        if self.pending_sequence_allows_motion_count()
            && let Some(digit) = Self::key_count_digit(key)
            && let Some(next) = Self::append_count_digit(self.pending_sequence_motion_count, digit)
        {
            self.pending_sequence_motion_count = Some(next);
            return true;
        }

        self.pending_sequence.push(KeyInput::from(key));
        match self
            .keybindings
            .match_sequence(&self.mode, &self.pending_sequence)
        {
            SequenceMatch::Exact(actions) => {
                self.pending_sequence.clear();
                let count = self.take_sequence_count();
                self.execute_bound_actions(&actions, count);
            }
            SequenceMatch::Prefix => {}
            SequenceMatch::NoMatch => {
                let reprocess_key =
                    matches!(self.pending_sequence.first(), Some(KeyInput::Char('y')));
                self.pending_sequence.clear();
                self.pending_sequence_count = None;
                self.pending_sequence_motion_count = None;
                // `y` gained a built-in `yy` prefix, but plain follow-up keys like
                // `:` still need to work after an abandoned yank prefix.
                if reprocess_key {
                    return false;
                }
            }
        }
        true
    }

    /// Capture normal-mode count prefixes before resolving actions.
    ///
    /// Returns `true` when the key was consumed as part of count parsing.
    pub(super) fn handle_pending_count_key(&mut self, key: Key) -> bool {
        // Count prefixes are only meaningful in plain Normal-mode dispatch.
        if !self.mode_uses_modal_bindings()
            || !self.pending_sequence.is_empty()
            || self.pending_find.is_some()
        {
            return false;
        }
        // Esc cancels a partially typed numeric prefix.
        if matches!(key, Key::Esc) && self.pending_count.is_some() {
            self.pending_count = None;
            return true;
        }

        let Some(digit) = Self::key_count_digit(key) else {
            return false;
        };
        let Some(next) = Self::append_count_digit(self.pending_count, digit) else {
            return false;
        };
        // Keep the parsed count pending until an action consumes it.
        self.pending_count = Some(next);
        true
    }

    /// Extract a numeric digit eligible for count parsing from key input.
    pub(super) fn key_count_digit(key: Key) -> Option<char> {
        match key {
            Key::Char(c) if c.is_ascii_digit() => Some(c),
            _ => None,
        }
    }

    /// Append one count digit with Vim-like leading-zero rules and count capping.
    pub(super) fn append_count_digit(current: Option<usize>, digit: char) -> Option<usize> {
        if !digit.is_ascii_digit() {
            return None;
        }
        if digit == '0' && current.is_none() {
            return None;
        }
        let digit_value = (digit as u8 - b'0') as usize;
        let next = current
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit_value);
        Some(next)
    }

    /// Whether the pending key prefix supports an in-sequence motion count.
    pub(super) fn pending_sequence_allows_motion_count(&self) -> bool {
        matches!(
            self.pending_sequence.as_slice(),
            [KeyInput::Char('d')]
                | [KeyInput::Char('c')]
                | [KeyInput::Char('=')]
                | [KeyInput::Char('>')]
                | [KeyInput::Char('<')]
        )
    }

    /// Return the generic operator kind started by `action`, if any.
    fn operator_kind_for_action(action: Action) -> Option<OperatorKind> {
        match action {
            Action::BeginDeleteOperator => Some(OperatorKind::Delete),
            Action::BeginChangeOperator => Some(OperatorKind::Change),
            Action::BeginYankOperator => Some(OperatorKind::Yank),
            Action::BeginIndentOperator => Some(OperatorKind::Indent),
            Action::BeginReindentOperator => Some(OperatorKind::Reindent),
            Action::BeginDedentOperator => Some(OperatorKind::Dedent),
            _ => None,
        }
    }

    /// Merge outer and motion counts for operator+motion flows using multiplication.
    pub(super) fn take_sequence_count(&mut self) -> Option<usize> {
        let outer = self.pending_sequence_count.take();
        let inner = self.pending_sequence_motion_count.take();
        match (outer, inner) {
            (None, None) => None,
            (Some(o), None) => Some(o),
            (None, Some(i)) => Some(i),
            (Some(o), Some(i)) => Some(o.saturating_mul(i).min(Self::MAX_COUNT)),
        }
    }

    /// Apply an `f/F/t/T` motion with all-or-nothing counted target resolution.
    pub(super) fn apply_find_motion(
        &mut self,
        motion: FindMotion,
        target: char,
        update_last_find: LastFindUpdate,
    ) {
        if matches!(update_last_find, LastFindUpdate::Store) {
            self.last_find = Some(LastFind { motion, target });
        }

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut search_from = cursor_idx;
        let mut target_idx = None;
        for _ in 0..motion.count {
            let Some(idx) = self.find_char_on_current_line(search_from, motion.direction, target)
            else {
                return;
            };
            target_idx = Some(idx);
            search_from = idx;
        }
        let Some(target_idx) = target_idx else {
            return;
        };

        let destination = match (motion.kind, motion.direction) {
            (FindMotionKind::Find, _) => target_idx,
            (FindMotionKind::Till, FindDirection::Forward) => target_idx.saturating_sub(1),
            (FindMotionKind::Till, FindDirection::Backward) => target_idx.saturating_add(1),
        };

        self.cursor = Cursor::from_char_index(&self.buffer, destination);
    }

    /// Build the current rectangular block selection from the active Visual endpoints.
    fn block_selection(&self, anchor: &Cursor) -> BlockSelection {
        BlockSelection {
            start_line: anchor.line().min(self.cursor.line()),
            end_line: anchor.line().max(self.cursor.line()),
            left_column: anchor.column().min(self.cursor.column()),
            right_column: anchor.column().max(self.cursor.column()),
        }
    }

    /// Return the current Visual selection resolved into one concrete shape.
    pub(super) fn visual_selection(&self) -> Option<VisualSelection> {
        let anchor = self.visual_anchor.as_ref()?;
        let kind = match self.mode {
            Mode::Visual(kind) => kind,
            _ => return None,
        };

        match kind {
            // Characterwise visual mode uses inclusive cursor endpoints, so the
            // selection extends one char beyond the furthest endpoint.
            VisualKind::Character => {
                let anchor_idx = anchor.to_char_index(&self.buffer);
                let cursor_idx = self.cursor.to_char_index(&self.buffer);
                let start = anchor_idx.min(cursor_idx);
                let end = anchor_idx.max(cursor_idx).saturating_add(1);
                Some(VisualSelection::Character(SelectionRange {
                    start,
                    end: end.min(self.buffer.chars_count()),
                }))
            }
            // Linewise mode expands to full logical-line boundaries so edits and
            // highlighting stay consistent regardless of cursor columns.
            VisualKind::Line => {
                let start_line = anchor.line().min(self.cursor.line());
                let end_line = anchor.line().max(self.cursor.line());
                let start = self.buffer.line_to_char(start_line);
                let end = if end_line + 1 < self.buffer.lines_count() {
                    self.buffer.line_to_char(end_line + 1)
                } else {
                    self.buffer.chars_count()
                };
                Some(VisualSelection::Line(SelectionRange { start, end }))
            }
            VisualKind::Block => Some(VisualSelection::Block(self.block_selection(anchor))),
        }
    }

    /// Return the current visual selection as an exclusive character-index range.
    pub(crate) fn selection_range(&self) -> Option<(usize, usize)> {
        match self.visual_selection()? {
            VisualSelection::Character(selection) | VisualSelection::Line(selection) => {
                Some((selection.start, selection.end))
            }
            VisualSelection::Block(_) => None,
        }
    }

    /// Return whether the active Visual selection highlights `column` on `line_idx`.
    ///
    /// Returns `true` when the logical cell belongs to the active selection and
    /// `false` when Visual mode is inactive or that cell is outside the selection.
    pub(crate) fn selection_contains_cell(&self, line_idx: usize, column: usize) -> bool {
        self.visual_selection()
            .is_some_and(|selection| selection.contains_cell(&self.buffer, line_idx, column))
    }

    /// Repeat the last find motion up to `count` times, stopping at first no-op.
    pub(super) fn repeat_find(&mut self, repeat_direction: FindRepeatDirection, count: usize) {
        let Some(last) = self.last_find else {
            return;
        };

        let direction = match repeat_direction {
            FindRepeatDirection::Same => last.motion.direction,
            FindRepeatDirection::Reversed => last.motion.direction.reversed(),
        };

        let motion = FindMotion {
            kind: last.motion.kind,
            direction,
            count: 1,
        };
        for _ in 0..count {
            let before = self.cursor.clone();
            self.apply_find_motion(motion, last.target, LastFindUpdate::Preserve);
            if self.cursor == before {
                break;
            }
        }
    }

    /// Find the next matching target index on the current line in the given direction.
    pub(super) fn find_char_on_current_line(
        &self,
        cursor_idx: usize,
        direction: FindDirection,
        target: char,
    ) -> Option<usize> {
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let line_len = self.buffer.line_len(self.cursor.line());
        let line_end_exclusive = line_start + line_len;

        match direction {
            FindDirection::Forward => ((cursor_idx.saturating_add(1)).min(line_end_exclusive)
                ..line_end_exclusive)
                .find(|&idx| self.buffer.char_at(idx) == Some(target)),
            FindDirection::Backward => {
                if cursor_idx <= line_start {
                    return None;
                }
                (line_start..cursor_idx)
                    .rev()
                    .find(|&idx| self.buffer.char_at(idx) == Some(target))
            }
        }
    }

    /// Move the cursor to the requested percentage of the current file.
    pub(super) fn goto_percent_of_file(&mut self, percent: usize) {
        let total_lines = self.buffer.lines_count().max(1);
        let percent = percent.clamp(1, 100);
        let target_line = total_lines.saturating_mul(percent).saturating_sub(1) / 100;
        self.cursor = Cursor::new(target_line.min(total_lines.saturating_sub(1)), 0);
    }
}
