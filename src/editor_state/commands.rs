//! Command, search, save, and quit helpers for `EditorState`.

use super::ex_commands::{Command, WriteTarget, parse_command};
use super::*;
use crate::substitute::{SubstituteCommand, build_substitute_plan};
use std::collections::VecDeque;
use std::io::{self, Write};

impl EditorState {
    /// Take the next deferred request queued by command execution, if any.
    ///
    /// Requests are one-shot because they describe work for exactly one pass of
    /// the outer event loop after the triggering key sequence completes.
    pub(crate) fn take_pending_request(&mut self) -> Option<EditorRequest> {
        self.pending_request.take()
    }

    /// Execute the current command/search input and apply side effects.
    ///
    /// Command mode supports save/quit commands, buffer management, and numeric
    /// go-to-line input.
    pub(super) fn execute_command(&mut self) {
        if let Some(pattern) = self.mode.take_search_input() {
            self.sync_search_highlights_for_viewport();
            self.prompt_history
                .record(PromptHistoryKind::Search, &pattern);
            self.execute_search(&pattern);
            return;
        }

        let Some(command_input) = self.mode.take_command_input() else {
            return;
        };
        self.prompt_history
            .record(PromptHistoryKind::Command, &command_input);

        match parse_command(&command_input) {
            Ok(Command::Substitute(command)) => self.execute_substitute_command(&command),
            Ok(command) => {
                self.clear_substitute_preview(true);
                self.execute_parsed_command(command);
            }
            Err(error) => {
                self.clear_substitute_preview(true);
                self.show_status_message(error.into_status_message());
            }
        }
    }

    /// Execute one previously parsed command.
    fn execute_parsed_command(&mut self, command: Command) {
        // The structured command carries all parsing decisions into the mutation layer.
        match command {
            Command::GotoLine(line_num) => self.goto_line(line_num),
            Command::Edit(path) => {
                if let Err(error) = self.open_buffer_from_edit(&path) {
                    self.show_status_message(format!("Error opening file: {error}"));
                }
            }
            Command::New => self.open_empty_buffer(),
            Command::BufferNext => self.show_next_buffer(),
            Command::BufferPrev => self.show_prev_buffer(),
            Command::Buffers => {
                let listing = self.format_buffer_list();
                self.show_status_message(listing);
            }
            Command::BufferDelete => self.execute_buffer_delete(),
            Command::Quit { force, exit_code } => self.execute_quit_command(force, exit_code),
            Command::Update { post_save_action } => self.update_current_file(post_save_action),
            Command::Undo => self.undo_changes(1),
            Command::Redo => self.redo_changes(1),
            Command::SaveSession(name) => {
                self.pending_request = Some(EditorRequest::SaveSession(name));
            }
            Command::OpenSession(name) => self.execute_open_session_command(name),
            Command::DeleteSession(name) => {
                self.pending_request = Some(EditorRequest::DeleteSession(name));
            }
            Command::Write {
                overwrite_behavior,
                target,
                post_save_action,
            } => {
                self.execute_write_command(overwrite_behavior, target, post_save_action);
            }
            Command::WriteAll => self.execute_write_all_command(),
            Command::ReloadConfig => {
                self.pending_request = Some(EditorRequest::ReloadConfig);
            }
            Command::Diagnostics => self.open_diagnostics_picker(),
            Command::NextDiagnostic => self.goto_next_diagnostic(),
            Command::PrevDiagnostic => self.goto_prev_diagnostic(),
            Command::Grep(pattern) => self.execute_grep_pattern(pattern),
            Command::RenameSymbol(new_name) => self.request_rename(new_name),
            Command::Substitute(command) => self.execute_substitute_command(&command),
        }
    }

    /// Execute one parsed grep pattern through the shared search-picker flow.
    pub(super) fn execute_grep_pattern(&mut self, pattern: String) {
        self.open_search_picker(pattern);
    }

    /// Execute a parsed quit command while preserving confirmation behavior.
    fn execute_quit_command(&mut self, force: bool, exit_code: i32) {
        if force {
            self.request_quit(exit_code);
            return;
        }

        let Some(dirty_buffers) = self.prepare_dirty_buffer_confirmation() else {
            self.request_quit(exit_code);
            return;
        };

        self.pending_quit_confirmation = Some(PendingQuitConfirmation {
            // `prepare_dirty_buffer_confirmation` leaves the currently displayed
            // buffer first so the prompt can refer to it immediately; the queue
            // only needs the remaining buffers that will be visited afterward.
            remaining_buffer_ids: dirty_buffers.into_iter().skip(1).collect(),
        });
        self.pending_session_open_confirmation = None;
        self.pending_buffer_close_confirmation = false;
        self.clear_status_message();
    }

    /// Execute a parsed open-session command with dirty-buffer confirmation.
    fn execute_open_session_command(&mut self, session_name: String) {
        let Some(dirty_buffers) = self.prepare_dirty_buffer_confirmation() else {
            self.pending_request = Some(EditorRequest::OpenSession(session_name));
            return;
        };

        self.pending_session_open_confirmation = Some(PendingSessionOpenConfirmation {
            session_name,
            // `prepare_dirty_buffer_confirmation` leaves the currently displayed
            // buffer first so the prompt can refer to it immediately; the queue
            // only needs the remaining buffers that will be visited afterward.
            remaining_buffer_ids: dirty_buffers.into_iter().skip(1).collect(),
        });
        self.pending_quit_confirmation = None;
        self.pending_buffer_close_confirmation = false;
        self.clear_status_message();
    }

    /// Execute a parsed write command for the current buffer or a new path.
    fn execute_write_command(
        &mut self,
        overwrite_behavior: OverwriteBehavior,
        target: WriteTarget,
        post_save_action: PostSaveAction,
    ) {
        // Named targets use `request_save_as`; current-file writes use `request_save_current`.
        match target {
            WriteTarget::Path(path) => self.request_save_as(&path, overwrite_behavior),
            WriteTarget::CurrentFile => {
                self.request_save_current(overwrite_behavior, post_save_action);
            }
        }
    }

    /// Save every modified named buffer and restore the originally active buffer afterward.
    fn execute_write_all_command(&mut self) {
        let return_to_buffer_id = self.active_buffer_id;
        let Some(mut dirty_buffer_ids) = self.prepare_write_all_targets() else {
            return;
        };

        // Start with the first queued buffer, then let each deferred write advance
        // the sequence so filesystem I/O still stays at the app layer.
        let first_dirty_id = dirty_buffer_ids
            .pop_front()
            .expect("write-all should have at least one target");
        self.switch_to_buffer_id(first_dirty_id);
        self.request_save_current_after_write(
            OverwriteBehavior::ConfirmIfDifferentPath,
            AfterWriteAction::ContinueWriteAllSequence {
                remaining_buffer_ids: dirty_buffer_ids,
                return_to_buffer_id,
            },
        );
    }

    /// Switch to the next buffer unless only one buffer is open.
    fn show_next_buffer(&mut self) {
        if self.buffer_manager.has_single_buffer() {
            return;
        }

        self.switch_to_next_buffer();
    }

    /// Switch to the previous buffer unless only one buffer is open.
    fn show_prev_buffer(&mut self) {
        if self.buffer_manager.has_single_buffer() {
            return;
        }

        self.switch_to_prev_buffer();
    }

    /// Delete the active buffer or prompt before discarding unsaved edits.
    fn execute_buffer_delete(&mut self) {
        if self.buffer.is_modified() {
            self.pending_buffer_close_confirmation = true;
            self.pending_quit_confirmation = None;
            self.pending_session_open_confirmation = None;
            self.clear_status_message();
            return;
        }

        self.close_active_buffer();
    }

    /// Execute one parsed substitute command against the active buffer.
    fn execute_substitute_command(&mut self, command: &SubstituteCommand) {
        let (plan, used_preview) =
            if let Some(plan) = self.take_substitute_preview_for_commit(command) {
                (plan, true)
            } else {
                match build_substitute_plan(command, &self.buffer, self.cursor.line()) {
                    Ok(plan) => (plan, false),
                    Err(error) => {
                        self.show_status_message(error);
                        return;
                    }
                }
            };
        if plan.substitution_count() == 0 {
            self.show_status_message("Pattern not found");
            return;
        }

        let search = match SearchQuery::compile(plan.pattern()) {
            Ok(search) => search,
            Err(error) => {
                self.show_status_message(format!("Invalid regex:\n{error}"));
                return;
            }
        };

        // Apply edits from the end toward the start so earlier character indices
        // stay valid throughout the whole transaction.
        self.begin_history_transaction();
        for edit in plan.edits().iter().rev() {
            self.remove_buffer_range(edit.start_char, edit.end_char);
            if !edit.replacement.is_empty() {
                self.insert_buffer_text(edit.start_char, &edit.replacement);
            }
        }
        self.clamp_active_cursor_after_substitute();
        self.finish_history_transaction();
        if !used_preview {
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        }
        self.last_search = Some(search);
        self.search_highlighting.reveal_committed();
        self.show_status_message(format_substitute_status_message(plan.substitution_count()));
        self.sync_search_highlights_for_viewport();
    }

    /// Clamp the active cursor after substitute mutates the current buffer.
    fn clamp_active_cursor_after_substitute(&mut self) {
        if self.mode == Mode::Insert {
            self.cursor.clamp_to_buffer(&self.buffer);
        } else {
            self.cursor.clamp_to_buffer_normal(&self.buffer);
        }
    }

    /// Execute one regex search from the current cursor and wrap if needed.
    pub(super) fn execute_search(&mut self, pattern: &str) {
        let repeat_count = self.pending_search_count.take().unwrap_or(1);
        if pattern.is_empty() {
            self.show_status_message("Pattern not found");
            self.sync_search_highlights_for_viewport();
            return;
        }

        // Compile first so invalid patterns do not replace the last successful search.
        let search = match SearchQuery::compile(pattern) {
            Ok(search) => search,
            Err(error) => {
                self.show_status_message(format!("Invalid regex:\n{error}"));
                self.sync_search_highlights_for_viewport();
                return;
            }
        };
        self.last_search = Some(search.clone());
        self.search_highlighting.reveal_committed();

        // Search from the current cursor first, then wrap to the document start.
        let start_idx = self.cursor.to_char_index(&self.buffer);
        if let Some(search_match) = search.find_forward(&self.buffer, start_idx) {
            self.jump_to_search_match(search_match);
            self.repeat_search_count(FindDirection::Forward, repeat_count.saturating_sub(1));
        } else if let Some(search_match) = search.find_forward(&self.buffer, 0) {
            self.jump_to_search_match(search_match);
            self.show_status_message("Search wrapped to beginning");
            self.repeat_search_count(FindDirection::Forward, repeat_count.saturating_sub(1));
        } else {
            self.show_status_message("Pattern not found");
        }
        self.sync_search_highlights_for_viewport();
    }

    /// Repeat the previous search in the requested direction.
    pub(super) fn repeat_search(&mut self, direction: FindDirection) {
        let Some(search) = self.last_search.clone() else {
            self.show_status_message("No previous search");
            return;
        };
        self.search_highlighting.reveal_committed();

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let total_chars = self.buffer.chars_count();

        match direction {
            FindDirection::Forward => {
                // Repeats start one character after the current match start so
                // overlapping regex matches remain reachable.
                let start_idx = cursor_idx.saturating_add(1);
                if let Some(search_match) = search.find_forward(&self.buffer, start_idx) {
                    self.jump_to_search_match(search_match);
                    self.sync_search_highlights_for_viewport();
                    return;
                }

                if let Some(search_match) = search.find_forward(&self.buffer, 0) {
                    self.jump_to_search_match(search_match);
                    self.show_status_message("Search wrapped to beginning");
                } else {
                    self.show_status_message("Pattern not found");
                }
            }
            FindDirection::Backward => {
                // Backward repeats exclude the current cursor position and wrap
                // against the full document when nothing earlier matches.
                if let Some(search_match) = search.find_backward(&self.buffer, cursor_idx) {
                    self.jump_to_search_match(search_match);
                } else if let Some(search_match) = search.find_backward(&self.buffer, total_chars) {
                    self.jump_to_search_match(search_match);
                    self.show_status_message("Search wrapped to end");
                } else {
                    self.show_status_message("Pattern not found");
                }
            }
        }
        self.sync_search_highlights_for_viewport();
    }

    /// Repeat search motion `count` times while preserving existing wrap/error behavior.
    pub(super) fn repeat_search_count(&mut self, direction: FindDirection, count: usize) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.repeat_search(direction);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    /// Move the cursor to the start of one matched search span.
    fn jump_to_search_match(&mut self, search_match: SearchMatch) {
        let target = Cursor::from_char_index(&self.buffer, search_match.start);
        if !self.record_jump_origin_for_destination(
            &self.file_path.clone(),
            target.line(),
            target.column(),
        ) {
            return;
        }
        self.cursor = target;
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Move the cursor to one requested 1-based line number.
    pub(super) fn goto_line(&mut self, line_num: usize) {
        let total_lines = self.buffer.lines_count();
        let target_line = if line_num == 0 {
            0
        } else if line_num > total_lines {
            self.show_status_message(format!(
                "Line {} out of range, moved to last line",
                line_num
            ));
            total_lines.saturating_sub(1)
        } else {
            line_num - 1 // Convert to 0-indexed
        };

        if !self.record_jump_origin_for_destination(&self.file_path.clone(), target_line, 0) {
            return;
        }
        self.cursor = Cursor::new(target_line, 0);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Update the active buffer only when it is dirty.
    ///
    /// `:update` and `<Space>q` use this helper so unchanged buffers can skip
    /// disk writes while still honoring a follow-up quit request.
    pub(super) fn update_current_file(&mut self, post_save_action: PostSaveAction) {
        if self.buffer.is_modified() {
            self.request_save_current(OverwriteBehavior::ConfirmIfDifferentPath, post_save_action);
        } else if post_save_action == PostSaveAction::QuitOnSuccess {
            self.request_quit(0);
        }
    }

    /// Request a save to the current file path.
    ///
    /// This centralizes `:w` and `:wq` behavior while keeping overwrite and
    /// post-save handling explicit at the callsite.
    pub(super) fn request_save_current(
        &mut self,
        overwrite_behavior: OverwriteBehavior,
        post_save_action: PostSaveAction,
    ) {
        self.request_save_current_after_write(
            overwrite_behavior,
            Self::after_write_action_from_post_save_action(post_save_action),
        );
    }

    /// Request a save to the current file path with one explicit post-write action.
    fn request_save_current_after_write(
        &mut self,
        overwrite_behavior: OverwriteBehavior,
        after_write_action: AfterWriteAction,
    ) {
        if self.file_path.as_os_str().is_empty() {
            self.show_status_message("No file name");
            return;
        }

        self.request_save(
            self.file_path.clone(),
            false,
            overwrite_behavior,
            after_write_action,
        );
    }

    /// Request a save to a user-supplied path (`:w <path>` / `:write <path>`).
    pub(super) fn request_save_as(
        &mut self,
        filename: &str,
        overwrite_behavior: OverwriteBehavior,
    ) {
        if filename.is_empty() {
            self.show_status_message("No file name");
            return;
        }

        self.request_save(
            PathBuf::from(filename),
            true,
            overwrite_behavior,
            AfterWriteAction::StayOpen,
        );
    }

    /// Convert one direct write command outcome into its post-write action.
    fn after_write_action_from_post_save_action(
        post_save_action: PostSaveAction,
    ) -> AfterWriteAction {
        match post_save_action {
            PostSaveAction::StayOpen => AfterWriteAction::StayOpen,
            PostSaveAction::QuitOnSuccess => AfterWriteAction::Quit,
        }
    }

    /// Shared save request pipeline for all write commands.
    ///
    /// It decides whether to queue an overwrite prompt or defer the filesystem
    /// write to the app layer, and applies post-write work only after a
    /// successful deferred write completes.
    pub(super) fn request_save(
        &mut self,
        target_path: PathBuf,
        update_file_path: bool,
        overwrite_behavior: OverwriteBehavior,
        after_write_action: AfterWriteAction,
    ) {
        if target_path.as_os_str().is_empty() {
            self.show_status_message("No file name");
            return;
        }

        if self.soft_read_only && !update_file_path && paths_match(&self.file_path, &target_path) {
            self.pending_soft_read_only_save = Some(PendingSoftReadOnlySave {
                target_path,
                update_file_path,
                after_write_action,
            });
            self.clear_status_message();
            return;
        }

        self.queue_or_confirm_overwrite(
            target_path,
            update_file_path,
            overwrite_behavior,
            after_write_action,
        );
    }

    /// Queue one save after the user confirms a soft read-only write.
    fn continue_soft_read_only_save(&mut self, pending: PendingSoftReadOnlySave) {
        self.queue_or_confirm_overwrite(
            pending.target_path,
            pending.update_file_path,
            OverwriteBehavior::ConfirmIfDifferentPath,
            pending.after_write_action,
        );
    }

    /// Queue a write immediately or prompt first when overwrite confirmation is needed.
    fn queue_or_confirm_overwrite(
        &mut self,
        target_path: PathBuf,
        update_file_path: bool,
        overwrite_behavior: OverwriteBehavior,
        after_write_action: AfterWriteAction,
    ) {
        let needs_overwrite_confirmation = overwrite_behavior
            == OverwriteBehavior::ConfirmIfDifferentPath
            && target_path.exists()
            && self.file_path != target_path;

        if needs_overwrite_confirmation {
            self.pending_overwrite = Some(PendingOverwrite {
                target_path,
                update_file_path,
                after_write_action,
                reason: OverwritePromptKind::DifferentTargetPath,
            });
            self.clear_status_message();
            return;
        }

        if overwrite_behavior == OverwriteBehavior::ConfirmIfDifferentPath {
            if self.enqueue_save_conflict_check(
                target_path.clone(),
                update_file_path,
                after_write_action.clone(),
            ) {
                return;
            }

            match self.check_external_save_conflict_sync(&target_path) {
                Ok(true) => {
                    self.pending_overwrite = Some(PendingOverwrite {
                        target_path,
                        update_file_path,
                        after_write_action,
                        reason: OverwritePromptKind::ExternalChange,
                    });
                    self.clear_status_message();
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    self.show_status_message(format!(
                        "Failed to verify external changes for {}: {error}",
                        target_path.display()
                    ));
                    return;
                }
            }
        }

        self.queue_write_request(target_path, update_file_path, after_write_action);
    }

    /// Queue one deferred write request for app-layer filesystem execution.
    pub(super) fn queue_write_request(
        &mut self,
        path: PathBuf,
        update_file_path: bool,
        after_write_action: AfterWriteAction,
    ) {
        self.pending_request = Some(EditorRequest::WriteBuffer(DeferredWrite {
            path,
            update_file_path,
            after_write_action,
        }));
    }

    /// Stream the current buffer contents into one caller-owned writer.
    pub(crate) fn write_buffer_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.buffer.write_to_for_save(writer)
    }

    /// Apply editor-local state changes after the app layer completes one write.
    pub(crate) fn complete_deferred_write(&mut self, write: DeferredWrite) {
        if write.update_file_path {
            // Saving to a new path updates both the displayed file name and
            // syntax detection for the current buffer.
            self.file_path = write.path.clone();
            self.soft_read_only = false;
            self.refresh_syntax();
            self.pending_lsp_sync_at = (!self.file_path.as_os_str().is_empty()).then(Instant::now);
            self.record_active_named_buffer();
        }
        self.refresh_active_read_only_state();
        self.buffer.normalize_after_save();
        self.external_file.sync_to_saved_buffer(&self.buffer);

        // The current undo depth becomes the clean on-disk reference point.
        self.saved_undo_depth = self.undo_stack.len();
        self.sync_modified_from_history();
        self.show_status_message(format!("\"{}\" written", write.path.display()));

        match write.after_write_action {
            AfterWriteAction::StayOpen => {}
            AfterWriteAction::Quit => self.request_quit(0),
            AfterWriteAction::ContinueQuitSequence(remaining) => {
                self.continue_quit_sequence(remaining);
            }
            AfterWriteAction::ContinueWriteAllSequence {
                remaining_buffer_ids,
                return_to_buffer_id,
            } => self.continue_write_all_sequence(remaining_buffer_ids, return_to_buffer_id),
            AfterWriteAction::ContinueSessionOpenSequence {
                session_name,
                remaining_buffer_ids,
            } => {
                self.continue_session_open_sequence(session_name, remaining_buffer_ids);
            }
            AfterWriteAction::CloseActiveBuffer => self.close_active_buffer(),
        }
    }

    /// Reconcile swap ownership after one write completed on disk.
    ///
    /// Returns `Some(message)` when the write itself succeeded but swap cleanup or
    /// swap recreation still needs to surface one warning to the user, and
    /// returns `None` when no extra status message is needed.
    pub(crate) fn finalize_swap_after_successful_write(
        &mut self,
        write: &DeferredWrite,
    ) -> Option<String> {
        if !write.update_file_path {
            return None;
        }

        let mut warning = None;
        if let Some(swap) = self.swap.take() {
            let swap_path = swap.swap_path().to_path_buf();
            if let Err(error) = swap.delete() {
                warning = Some(format!(
                    "\"{}\" written, but swap cleanup failed for {}: {error}",
                    write.path.display(),
                    swap_path.display()
                ));
            }
        }

        self.suppress_swap_creation = false;
        if !self.active_path_is_swap_excluded()
            && let Err(error) = self.create_active_swap_handle()
        {
            warning = Some(format!(
                "\"{}\" written, but swap protection is unavailable: {error}",
                write.path.display()
            ));
        }
        warning
    }

    /// Report a filesystem error that occurred while creating a target file.
    pub(crate) fn report_file_create_error(&mut self, error: io::Error) {
        self.show_status_message(format!("Error creating file: {}", error));
    }

    /// Report a filesystem error that occurred while streaming buffer bytes.
    pub(crate) fn report_file_write_error(&mut self, error: io::Error) {
        self.show_status_message(format!("Error writing file: {}", error));
    }

    /// Best-effort cleanup of every swap handle held by the current editor state.
    pub(crate) fn cleanup_all_swap_files(&mut self) {
        self.cleanup_active_swap_file();
        for buffer in self.buffer_manager.inactive_buffers_mut() {
            if let Some(swap) = buffer.swap.take() {
                let _ = swap.delete();
            }
        }
    }

    /// Consume one key while a swap-recovery choice is pending.
    ///
    /// Returns `true` when recovery mode consumed the key, even if the key only
    /// kept the prompt open because it was not one of the recognized choices.
    pub(super) fn handle_pending_swap_recovery_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_swap_recovery.take() else {
            return false;
        };

        match key {
            Key::Char('o') | Key::Char('O') => self.open_conflicting_swap_read_only(pending),
            Key::Char('e') | Key::Char('E') => self.open_conflicting_swap_edit_anyway(pending),
            Key::Char('r') | Key::Char('R') => self.restore_pending_swap_recovery(pending),
            Key::Char('d') | Key::Char('D') => self.discard_pending_swap_recovery(pending),
            Key::Char('c') | Key::Char('C') | Key::Esc => {
                self.cancel_pending_swap_recovery(pending)
            }
            _ => {
                self.pending_swap_recovery = Some(pending);
            }
        }
        true
    }

    /// Consume one key while a soft read-only save confirmation is pending.
    ///
    /// Returns `true` when the prompt consumed the key, and `false` when no
    /// soft read-only confirmation was waiting for input.
    pub(super) fn handle_pending_soft_read_only_save_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_soft_read_only_save.take() else {
            return false;
        };

        if key == Key::Char('y') || key == Key::Char('Y') {
            self.continue_soft_read_only_save(pending);
        } else {
            self.show_status_message("Write cancelled");
        }
        true
    }

    /// Consume one key while an external-change reload prompt is active.
    ///
    /// Returns `true` when the active buffer had a pending external-change
    /// decision, and `false` when no such prompt was active.
    pub(super) fn handle_pending_external_change_key(&mut self, key: Key) -> bool {
        if !self.active_external_change_prompt_active() {
            return false;
        }

        match key {
            Key::Char('r') | Key::Char('R') => self.reload_active_buffer_after_external_change(),
            Key::Char('i') | Key::Char('I') | Key::Esc => {
                self.external_file.mark_change_ignored();
                self.show_status_message(format!(
                    "\"{}\" kept despite external change",
                    self.file_name()
                ));
            }
            _ => {}
        }
        true
    }

    /// Consume one key while an overwrite prompt is pending.
    ///
    /// `y`/`Y` confirms and queues the deferred write; any other key cancels.
    /// Returns `true` when an overwrite prompt consumed the key, and `false`
    /// when no overwrite confirmation was waiting for input.
    pub(super) fn handle_pending_overwrite_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_overwrite.take() else {
            return false;
        };

        let confirmed = key == Key::Char('y') || key == Key::Char('Y');
        if confirmed {
            self.queue_write_request(
                pending.target_path,
                pending.update_file_path,
                pending.after_write_action,
            );
        } else {
            self.show_status_message("Write cancelled");
        }

        true
    }

    /// Consume one key while a quit confirmation prompt is pending.
    ///
    /// `y`/`Y` saves and continues quitting, `n`/`N` discards the current
    /// buffer's changes and continues, and any other key cancels quit.
    /// Returns `true` when a quit confirmation consumed the key, and `false`
    /// when no quit confirmation prompt was active.
    pub(super) fn handle_pending_quit_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_quit_confirmation.take() else {
            return false;
        };

        match key {
            Key::Char('y') | Key::Char('Y') => {
                let after_write_action = if pending.remaining_buffer_ids.is_empty() {
                    AfterWriteAction::Quit
                } else {
                    AfterWriteAction::ContinueQuitSequence(pending.remaining_buffer_ids)
                };
                self.request_save_current_after_write(
                    OverwriteBehavior::ConfirmIfDifferentPath,
                    after_write_action,
                );
            }
            Key::Char('n') | Key::Char('N') => {
                self.continue_quit_sequence(pending.remaining_buffer_ids);
            }
            _ => {
                self.show_status_message("Quit cancelled");
            }
        }

        true
    }

    /// Consume one key while a dirty-buffer close confirmation is pending.
    ///
    /// Returns `true` when the close-confirmation prompt consumed the key, and
    /// `false` when no close confirmation was active.
    pub(super) fn handle_pending_buffer_close_key(&mut self, key: Key) -> bool {
        if !self.pending_buffer_close_confirmation {
            return false;
        }

        self.pending_buffer_close_confirmation = false;
        match key {
            Key::Char('y') | Key::Char('Y') => {
                self.request_save_current_after_write(
                    OverwriteBehavior::ConfirmIfDifferentPath,
                    AfterWriteAction::CloseActiveBuffer,
                );
            }
            Key::Char('n') | Key::Char('N') => self.close_active_buffer(),
            _ => {
                self.show_status_message("Buffer delete cancelled");
            }
        }

        true
    }

    /// Consume one key while a session-open confirmation prompt is pending.
    ///
    /// Returns `true` when the session-open confirmation consumed the key, and
    /// `false` when no session-open confirmation prompt was active.
    pub(super) fn handle_pending_session_open_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_session_open_confirmation.take() else {
            return false;
        };

        match key {
            Key::Char('y') | Key::Char('Y') => {
                let after_write_action = AfterWriteAction::ContinueSessionOpenSequence {
                    session_name: pending.session_name,
                    remaining_buffer_ids: pending.remaining_buffer_ids,
                };
                self.request_save_current_after_write(
                    OverwriteBehavior::ConfirmIfDifferentPath,
                    after_write_action,
                );
            }
            Key::Char('n') | Key::Char('N') => {
                self.continue_session_open_sequence(
                    pending.session_name,
                    pending.remaining_buffer_ids,
                );
            }
            _ => {
                self.show_status_message("Session open cancelled");
            }
        }

        true
    }

    /// Continue quitting by switching to the next dirty buffer or exiting.
    fn continue_quit_sequence(&mut self, mut remaining_buffer_ids: VecDeque<usize>) {
        if let Some(next_id) = remaining_buffer_ids.pop_front() {
            self.switch_to_buffer_id(next_id);
            self.pending_quit_confirmation = Some(PendingQuitConfirmation {
                remaining_buffer_ids,
            });
            return;
        }

        self.request_quit(0);
    }

    /// Continue opening a session after saving or discarding the current dirty buffer.
    fn continue_session_open_sequence(
        &mut self,
        session_name: String,
        mut remaining_buffer_ids: VecDeque<usize>,
    ) {
        if let Some(next_id) = remaining_buffer_ids.pop_front() {
            self.switch_to_buffer_id(next_id);
            self.pending_session_open_confirmation = Some(PendingSessionOpenConfirmation {
                session_name,
                remaining_buffer_ids,
            });
            return;
        }

        self.pending_request = Some(EditorRequest::OpenSession(session_name));
    }

    /// Continue saving modified named buffers until the write-all queue is exhausted.
    fn continue_write_all_sequence(
        &mut self,
        mut remaining_buffer_ids: VecDeque<usize>,
        return_to_buffer_id: usize,
    ) {
        if let Some(next_id) = remaining_buffer_ids.pop_front() {
            self.switch_to_buffer_id(next_id);
            self.request_save_current_after_write(
                OverwriteBehavior::ConfirmIfDifferentPath,
                AfterWriteAction::ContinueWriteAllSequence {
                    remaining_buffer_ids,
                    return_to_buffer_id,
                },
            );
            return;
        }

        self.switch_to_buffer_id(return_to_buffer_id);
        self.show_status_message("All modified buffers written");
    }

    /// Prepare dirty-buffer confirmation so prompts always reference the shown buffer.
    fn prepare_dirty_buffer_confirmation(&mut self) -> Option<Vec<usize>> {
        let mut dirty_buffers = self.dirty_buffer_ids();
        if dirty_buffers.is_empty() {
            return None;
        }

        let current_id = self.active_buffer_id;
        // Keep the active dirty buffer first so the current screen stays stable
        // whenever the active buffer itself needs confirmation. If the current
        // buffer is clean but another buffer is dirty, switch to that buffer so
        // the prompt always refers to the buffer currently on screen.
        if let Some(current_index) = dirty_buffers
            .iter()
            .position(|&buffer_id| buffer_id == current_id)
        {
            dirty_buffers.swap(0, current_index);
        } else {
            self.switch_to_buffer_id(dirty_buffers[0]);
        }

        Some(dirty_buffers)
    }

    /// Prepare the ordered dirty-buffer list for `:wall`, or show why it cannot run.
    fn prepare_write_all_targets(&mut self) -> Option<VecDeque<usize>> {
        let mut dirty_buffers = self.dirty_buffer_ids();
        if dirty_buffers.is_empty() {
            self.show_status_message("No modified buffers");
            return None;
        }

        // Preflight the full set first so `:wall` never partially saves one
        // buffer and then stops when a later dirty buffer has no file name.
        for &buffer_id in &dirty_buffers {
            if self.named_file_path_for_buffer_id(buffer_id).is_none() {
                self.switch_to_buffer_id(buffer_id);
                self.show_status_message("No file name");
                return None;
            }
        }

        // Keep the current buffer first when it is already dirty so save-all
        // minimizes visible churn before returning to the original buffer.
        if let Some(current_index) = dirty_buffers
            .iter()
            .position(|&buffer_id| buffer_id == self.active_buffer_id)
        {
            dirty_buffers.swap(0, current_index);
        }

        Some(dirty_buffers.into_iter().collect())
    }

    /// Remove the active buffer and activate the next visible snapshot.
    fn close_active_buffer(&mut self) {
        self.cleanup_active_swap_file();
        let Some(next_id) = self.buffer_manager.remove_active_id(self.active_buffer_id) else {
            // Keep one empty buffer alive so the editor always has an active
            // document after the last buffer is closed.
            let replacement = BufferState::new_empty(
                self.active_buffer_id,
                self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
            );
            let _ = self.replace_active_buffer_state(replacement);
            self.reset_mode_for_buffer_switch();
            return;
        };
        // Buffer order already chose the visible successor, so resolve that
        // parked buffer and make it active in one swap.
        let target = self
            .buffer_manager
            .take_inactive_by_id(next_id)
            .expect("next buffer id should resolve to an inactive buffer");
        let _ = self.replace_active_buffer_state(target);
        self.reset_mode_for_buffer_switch();
    }

    /// Mark the editor as ready to quit with the requested process exit code.
    pub(super) fn request_quit(&mut self, exit_code: i32) {
        self.quit_exit_code = exit_code;
        self.should_quit = true;
    }

    /// Apply one cancel action for a dismissed swap prompt.
    fn execute_pending_swap_cancel_action(&mut self, action: PendingSwapCancelAction) {
        match action {
            PendingSwapCancelAction::CloseBuffer => self.close_active_buffer(),
            PendingSwapCancelAction::Quit => self.request_quit(0),
        }
    }

    /// Open the current on-disk buffer in soft read-only mode during a conflict.
    fn open_conflicting_swap_read_only(&mut self, pending: PendingSwapPrompt) {
        let PendingSwapPromptKind::Conflict = pending.kind else {
            self.pending_swap_recovery = Some(pending);
            return;
        };
        // The other Ordex instance still owns the swap file, so this buffer must
        // not refresh or recreate that path while it is merely observing the file.
        self.suppress_swap_creation = true;
        // Soft read-only keeps local navigation and in-memory edits available, but
        // the later save path asks again before writing back to the same file.
        self.soft_read_only = true;
        self.refresh_active_read_only_state();
        self.pending_swap_refresh_at = None;
        self.show_status_message("Opened read-only; writes will ask for confirmation");
    }

    /// Continue editing the current on-disk buffer without owning the swap file.
    fn open_conflicting_swap_edit_anyway(&mut self, pending: PendingSwapPrompt) {
        let PendingSwapPromptKind::Conflict = pending.kind else {
            self.pending_swap_recovery = Some(pending);
            return;
        };
        // Editing anyway still leaves swap ownership with the other instance, so
        // this buffer must avoid rewriting that swap file.
        self.suppress_swap_creation = true;
        // This path clears the soft read-only flag because the user explicitly
        // chose to continue as a normal writable buffer.
        self.soft_read_only = false;
        self.refresh_active_read_only_state();
        self.pending_swap_refresh_at = None;
        self.show_status_message(
            "Opened without swap protection while another instance owns the swap",
        );
    }

    /// Restore the active buffer from the pending swap-recovery payload.
    fn restore_pending_swap_recovery(&mut self, pending: PendingSwapPrompt) {
        let line = self
            .cursor
            .line()
            .min(pending.recovered_buffer.lines_count().saturating_sub(1));
        // Rebuild the cursor against the recovered text so reopening a swap does
        // not leave the cursor beyond the restored line length.
        let mut recovered_cursor = Cursor::new(line, self.cursor.column());
        recovered_cursor.clamp_to_line(&pending.recovered_buffer);
        self.buffer = pending.recovered_buffer;
        self.cursor = recovered_cursor;
        self.desired_visual_column = None;
        self.viewport.set_first_visible_line(0);
        self.refresh_syntax();
        self.reset_history();
        self.pending_swap_refresh_at = None;
        self.soft_read_only = false;
        self.refresh_active_read_only_state();

        // Recovered content is intentionally dirty because it differs from the
        // last confirmed on-disk state even before the user makes new edits.
        self.saved_undo_depth = usize::MAX;
        self.buffer.set_modified(true);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        // Recovering from a foreign conflict reuses the recovered text without
        // stealing swap ownership from the still-running editor instance.
        self.suppress_swap_creation = matches!(pending.kind, PendingSwapPromptKind::Conflict);
        if !self.suppress_swap_creation
            && let Err(error) = self.create_active_swap_handle()
        {
            self.show_swap_unavailable_error(&error);
            return;
        }
        self.show_status_message("Recovered unsaved work");
    }

    /// Discard the pending recovery payload and optionally recreate a fresh swap file.
    fn discard_pending_swap_recovery(&mut self, pending: PendingSwapPrompt) {
        self.cleanup_active_swap_file();
        if let Err(error) = swap::delete_swap_path(&pending.swap_path) {
            self.show_status_message(format!(
                "Swap cleanup failed for {}: {error}",
                pending.swap_path.display()
            ));
            return;
        }
        self.suppress_swap_creation = false;
        self.soft_read_only = false;
        self.refresh_active_read_only_state();
        if pending.recreate_handle_on_discard
            && let Err(error) = self.create_active_swap_handle()
        {
            self.show_swap_unavailable_error(&error);
            return;
        }
        self.pending_swap_refresh_at = None;
        self.show_status_message("Recovery data discarded");
    }

    /// Cancel the pending swap prompt and close or quit when appropriate.
    fn cancel_pending_swap_recovery(&mut self, pending: PendingSwapPrompt) {
        self.pending_swap_refresh_at = None;
        self.execute_pending_swap_cancel_action(pending.cancel_action);
    }

    /// Delete the active swap file on a best-effort basis and clear the handle.
    fn cleanup_active_swap_file(&mut self) {
        self.pending_swap_refresh_at = None;
        if let Some(swap) = self.swap.take() {
            let swap_path = swap.swap_path().to_path_buf();
            if let Err(error) = swap.delete() {
                self.show_status_message(format!(
                    "Swap cleanup failed for {}: {error}",
                    swap_path.display()
                ));
            }
        }
    }
}

/// Format the user-visible status line after a successful substitute command.
fn format_substitute_status_message(count: usize) -> String {
    match count {
        1 => "1 substitution".to_string(),
        _ => format!("{count} substitutions"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempFile;

    /// Successful substitute commands should mutate the buffer and refresh last search.
    #[test]
    fn test_execute_substitute_command_updates_buffer_and_last_search() {
        let mut editor = EditorState::new(10);
        editor.buffer_mut().insert(0, "foo foo\nfoo\n");

        editor.execute_parsed_command(Command::Substitute(SubstituteCommand {
            scope: crate::substitute::SubstituteScope::CurrentLine,
            pattern: "foo".to_string(),
            replacement: "bar".to_string(),
        }));

        assert_eq!(editor.buffer.to_string(), "bar bar\nfoo\n");
        assert_eq!(editor.status_message.as_deref(), Some("2 substitutions"));
        assert!(editor.last_search.is_some());
    }

    /// Failed substitute commands should leave the previous search unchanged.
    #[test]
    fn test_execute_substitute_command_without_match_keeps_previous_search() {
        let mut editor = EditorState::new(10);
        editor.buffer_mut().insert(0, "foo\nbar\n");
        editor.execute_search("foo");

        editor.execute_parsed_command(Command::Substitute(SubstituteCommand {
            scope: crate::substitute::SubstituteScope::CurrentLine,
            pattern: "zzz".to_string(),
            replacement: "bar".to_string(),
        }));

        assert_eq!(editor.buffer.to_string(), "foo\nbar\n");
        assert_eq!(editor.status_message.as_deref(), Some("Pattern not found"));
        editor.repeat_search(FindDirection::Forward);
        assert_eq!(editor.cursor.line(), 0);
    }

    /// Opening a clean session should immediately queue the deferred app request.
    #[test]
    fn test_open_session_queues_deferred_request_when_clean() {
        let mut editor = EditorState::new(10);

        editor.execute_parsed_command(Command::OpenSession("demo".to_string()));

        assert_eq!(
            editor.take_pending_request(),
            Some(EditorRequest::OpenSession("demo".to_string()))
        );
    }

    /// Opening a session from dirty buffers should prompt before replacement.
    #[test]
    fn test_open_session_prompts_when_dirty() {
        let mut editor = EditorState::new(10);
        editor.buffer_mut().insert(0, "dirty");

        editor.execute_parsed_command(Command::OpenSession("demo".to_string()));

        assert_eq!(
            editor.session_open_prompt().as_deref(),
            Some(
                "Save changes to \"[No Name]\" before opening session \"demo\"? [y]es/[n]o/[c]ancel"
            )
        );
        assert_eq!(editor.take_pending_request(), None);
    }

    /// Restoring pending recovery should replace the buffer and keep it dirty.
    #[test]
    fn test_pending_swap_recovery_restore_marks_buffer_dirty() {
        let mut editor = EditorState::new(10);
        editor.pending_swap_recovery = Some(PendingSwapPrompt {
            prompt: "prompt".to_string(),
            recovered_buffer: TextBuffer::from_str("recovered"),
            swap_path: PathBuf::from("/tmp/recovered.swp"),
            kind: PendingSwapPromptKind::Conflict,
            cancel_action: PendingSwapCancelAction::Quit,
            recreate_handle_on_discard: false,
        });

        assert!(editor.handle_pending_swap_recovery_key(Key::Char('r')));

        assert_eq!(editor.buffer.to_string(), "recovered");
        assert!(editor.buffer.is_modified());
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Recovered unsaved work")
        );
    }

    /// `:wall` should reject dirty unnamed buffers before starting any save sequence.
    #[test]
    fn test_prepare_write_all_targets_rejects_unnamed_dirty_buffer() {
        let mut editor = EditorState::new(10);
        editor.file_path = "named.txt".into();
        editor.buffer_mut().insert(0, "named");
        editor.open_empty_buffer();
        editor.buffer_mut().insert(0, "scratch");

        assert!(editor.prepare_write_all_targets().is_none());
        assert_eq!(editor.status_message.as_deref(), Some("No file name"));
    }

    /// Discarding pending recovery should delete the stale swap file.
    #[test]
    fn test_pending_swap_recovery_discard_deletes_swap_file() {
        let source_file = TempFile::with_suffix("_swap_discard_source.txt").expect("temp file");
        source_file.write_all(b"disk").expect("seed source");
        let source_path = source_file.path().to_path_buf();

        let mut editor = EditorState::new(10);
        editor.file_path = source_path.clone();
        editor.swap = Some(
            SwapHandle::create_from_buffer(&source_path, &TextBuffer::from_str("disk"))
                .expect("create swap"),
        );
        let swap_path = editor
            .swap
            .as_ref()
            .expect("swap handle")
            .swap_path()
            .to_path_buf();
        editor.pending_swap_recovery = Some(PendingSwapPrompt {
            prompt: "prompt".to_string(),
            recovered_buffer: TextBuffer::from_str("recovered"),
            swap_path: swap_path.clone(),
            kind: PendingSwapPromptKind::Recovery,
            cancel_action: PendingSwapCancelAction::Quit,
            recreate_handle_on_discard: false,
        });

        assert!(editor.handle_pending_swap_recovery_key(Key::Char('d')));

        assert!(!swap_path.exists());
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Recovery data discarded")
        );
    }

    /// Soft read-only buffers should ask once more before saving in place.
    #[test]
    fn test_soft_read_only_save_prompts_before_queueing_write() {
        let file = TempFile::with_suffix("_soft_read_only.txt").expect("temp file");
        file.write_all(b"disk").expect("seed file");
        let mut editor = EditorState::new(10);
        editor.file_path = file.path().to_path_buf();
        editor.soft_read_only = true;
        editor.refresh_active_read_only_state();

        editor.request_save_current_after_write(
            OverwriteBehavior::ConfirmIfDifferentPath,
            AfterWriteAction::StayOpen,
        );

        assert_eq!(
            editor.soft_read_only_save_prompt(),
            Some(format!(
                "Write read-only file \"{}\" anyway? [y/N]",
                file.path().display()
            ))
        );
        assert!(editor.handle_pending_soft_read_only_save_key(Key::Char('y')));
        assert_eq!(
            editor.take_pending_request(),
            Some(EditorRequest::WriteBuffer(DeferredWrite {
                path: file.path().to_path_buf(),
                update_file_path: false,
                after_write_action: AfterWriteAction::StayOpen,
            }))
        );
    }

    /// Conflict read-only opens should keep the read-only indicator and suppress swap writes.
    #[test]
    fn test_conflicting_swap_read_only_sets_soft_read_only_state() {
        let mut editor = EditorState::new(10);
        editor.pending_swap_recovery = Some(PendingSwapPrompt {
            prompt: "prompt".to_string(),
            recovered_buffer: TextBuffer::from_str("recovered"),
            swap_path: PathBuf::from("/tmp/conflict.swp"),
            kind: PendingSwapPromptKind::Conflict,
            cancel_action: PendingSwapCancelAction::Quit,
            recreate_handle_on_discard: false,
        });

        assert!(editor.handle_pending_swap_recovery_key(Key::Char('o')));
        assert!(editor.is_read_only());
        assert!(editor.soft_read_only);
        assert!(editor.suppress_swap_creation);
    }

    /// Saving to a new path should move swap ownership to that destination immediately.
    #[test]
    fn test_finalize_swap_after_successful_write_moves_swap_to_new_path() {
        let source_file = TempFile::with_suffix("_swap_move_source.txt").expect("temp file");
        source_file.write_all(b"disk").expect("seed file");
        let target_dir = test_utils::TempTree::with_prefix("ordex_swap_move_target").expect("tree");
        let target_path = target_dir.path().join("renamed.txt");
        let mut editor = EditorState::new(10);
        editor.file_path = source_file.path().to_path_buf();
        editor.swap = Some(
            SwapHandle::create_from_buffer(source_file.path(), &TextBuffer::from_str("disk"))
                .expect("create swap"),
        );
        let old_swap_path = editor
            .swap
            .as_ref()
            .expect("swap")
            .swap_path()
            .to_path_buf();
        let write = DeferredWrite {
            path: target_path.clone(),
            update_file_path: true,
            after_write_action: AfterWriteAction::StayOpen,
        };

        editor.complete_deferred_write(write.clone());
        assert_eq!(editor.finalize_swap_after_successful_write(&write), None);

        assert!(!old_swap_path.exists());
        assert_ne!(
            old_swap_path,
            editor
                .swap
                .as_ref()
                .expect("new swap")
                .swap_path()
                .to_path_buf()
        );
        assert!(
            editor
                .swap
                .as_ref()
                .is_some_and(|swap| swap.swap_path().exists())
        );
        assert_eq!(editor.file_path, target_path);
    }

    /// `:edit` should replace the startup unnamed buffer when it is still pristine.
    #[test]
    fn test_edit_replaces_default_unnamed_buffer_for_existing_file() {
        let file = TempFile::with_suffix("_edit_existing.txt").expect("temp file");
        file.write_all(b"existing\n").expect("seed file");
        let mut editor = EditorState::new(10);
        let initial_buffer_id = editor.active_buffer_id();

        // Command execution should reuse the initial slot instead of allocating a second buffer.
        editor.execute_parsed_command(Command::Edit(file.path().display().to_string()));

        assert_eq!(editor.buffer.to_string(), "existing\n");
        assert_eq!(editor.file_path, file.path().to_path_buf());
        assert_eq!(editor.active_buffer_id(), initial_buffer_id);
        assert_eq!(editor.format_buffer_list().split(" | ").count(), 1);
    }

    /// `:edit` should also replace the startup unnamed buffer for a missing target path.
    #[test]
    fn test_edit_replaces_default_unnamed_buffer_for_missing_file() {
        let tree = test_utils::TempTree::with_prefix("ordex_edit_missing_target").expect("tree");
        let missing_path = tree.path().join("missing.txt");
        let mut editor = EditorState::new(10);
        let initial_buffer_id = editor.active_buffer_id();

        // Missing targets still become the active named buffer without preserving `[No Name]`.
        editor.execute_parsed_command(Command::Edit(missing_path.display().to_string()));

        assert_eq!(editor.file_path, missing_path);
        assert_eq!(editor.buffer.chars_count(), 0);
        assert!(!editor.buffer.is_modified());
        assert_eq!(editor.active_buffer_id(), initial_buffer_id);
        assert_eq!(editor.format_buffer_list().split(" | ").count(), 1);
    }

    /// `:edit` should keep the old buffer when the unnamed startup buffer has unsaved edits.
    #[test]
    fn test_edit_keeps_modified_unnamed_buffer() {
        let file = TempFile::with_suffix("_edit_dirty_unnamed.txt").expect("temp file");
        file.write_all(b"replacement\n").expect("seed file");
        let mut editor = EditorState::new(10);
        editor.buffer_mut().insert(0, "dirty");

        // Unsaved edits must block replacement so the dirty unnamed buffer remains open.
        editor.execute_parsed_command(Command::Edit(file.path().display().to_string()));

        assert_eq!(editor.file_path, file.path().to_path_buf());
        assert_eq!(editor.format_buffer_list().split(" | ").count(), 2);
    }

    /// `:edit` should keep existing buffer entries when more than one buffer is open.
    #[test]
    fn test_edit_keeps_unnamed_buffer_when_multiple_buffers_open() {
        let file = TempFile::with_suffix("_edit_multi_buffer.txt").expect("temp file");
        file.write_all(b"replacement\n").expect("seed file");
        let mut editor = EditorState::new(10);
        editor.open_empty_buffer();

        // Replacement only applies to the startup single-buffer state.
        editor.execute_parsed_command(Command::Edit(file.path().display().to_string()));

        assert_eq!(editor.file_path, file.path().to_path_buf());
        assert_eq!(editor.format_buffer_list().split(" | ").count(), 3);
    }
}
