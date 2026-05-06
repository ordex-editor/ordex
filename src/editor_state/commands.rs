//! Command, search, save, and quit helpers for `EditorState`.

use super::*;
use crate::substitute::{SubstituteCommand, build_substitute_plan, parse_substitute_command};
use std::collections::VecDeque;
use std::io::{self, Write};

/// Parsed command-mode input that is ready for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    GotoLine(usize),
    Edit(String),
    BufferNext,
    BufferPrev,
    Buffers,
    BufferDelete,
    Quit {
        force: bool,
        exit_code: i32,
    },
    Update,
    Undo,
    Redo,
    SaveSession(String),
    OpenSession(String),
    DeleteSession(String),
    Write {
        overwrite_behavior: OverwriteBehavior,
        target: WriteTarget,
        post_save_action: PostSaveAction,
    },
    ReloadConfig,
    Diagnostics,
    NextDiagnostic,
    PrevDiagnostic,
    RenameSymbol(String),
    Substitute(SubstituteCommand),
}

/// Target location for a parsed write command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WriteTarget {
    CurrentFile,
    Path(String),
}

/// Error returned when command-mode input does not match a supported command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandParseError {
    Unknown(String),
    MissingArgument(&'static str),
    InvalidSubstitute(String),
}

impl CommandParseError {
    /// Convert a parse error into the status message shown to the user.
    fn into_status_message(self) -> String {
        match self {
            Self::Unknown(command) => format!("Unknown command: {}", command),
            Self::MissingArgument(command) => format!("{command} requires an argument"),
            Self::InvalidSubstitute(error) => error,
        }
    }
}

/// Parse one command-mode input string into a structured command.
fn parse_command(input: &str) -> Result<Command, CommandParseError> {
    let trimmed = input.trim();

    // Numeric input maps directly to the command-mode line jump.
    if let Ok(line_num) = trimmed.parse::<usize>() {
        return Ok(Command::GotoLine(line_num));
    }
    if let Some(result) = parse_substitute_command(trimmed) {
        return result
            .map(Command::Substitute)
            .map_err(CommandParseError::InvalidSubstitute);
    }

    // Split once so `:w path with spaces` preserves the full target path.
    let (name, arg) = match trimmed.split_once(' ') {
        Some((name, arg)) => (name, Some(arg.trim())),
        None => (trimmed, None),
    };

    match (name, arg) {
        ("q", None) => Ok(Command::Quit {
            force: false,
            exit_code: 0,
        }),
        ("q!", None) => Ok(Command::Quit {
            force: true,
            exit_code: 0,
        }),
        ("cquit", None) => Ok(Command::Quit {
            force: true,
            exit_code: 1,
        }),
        ("update", None) => Ok(Command::Update),
        ("undo", None) => Ok(Command::Undo),
        ("redo", None) => Ok(Command::Redo),
        ("save-session", Some(name)) => Ok(Command::SaveSession(name.to_string())),
        ("open-session", Some(name)) => Ok(Command::OpenSession(name.to_string())),
        ("delete-session", Some(name)) => Ok(Command::DeleteSession(name.to_string())),
        ("e" | "edit", Some(path)) => Ok(Command::Edit(path.to_string())),
        ("bn" | "buffer-next", None) => Ok(Command::BufferNext),
        ("bp" | "buffer-prev", None) => Ok(Command::BufferPrev),
        ("ls" | "buffers", None) => Ok(Command::Buffers),
        ("bd" | "buffer-delete", None) => Ok(Command::BufferDelete),
        ("w", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("w!", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::Force,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("w", Some(filename)) | ("write", Some(filename)) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
            target: WriteTarget::Path(filename.to_string()),
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("w!", Some(filename)) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::Force,
            target: WriteTarget::Path(filename.to_string()),
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("wq", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::QuitOnSuccess,
        }),
        ("wq!", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::Force,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::QuitOnSuccess,
        }),
        ("reload-config", None) => Ok(Command::ReloadConfig),
        ("diagnostics", None) => Ok(Command::Diagnostics),
        ("next-diagnostic", None) => Ok(Command::NextDiagnostic),
        ("prev-diagnostic", None) => Ok(Command::PrevDiagnostic),
        ("rename", Some(new_name)) if !new_name.is_empty() => {
            Ok(Command::RenameSymbol(new_name.to_string()))
        }
        ("rename", _) => Err(CommandParseError::MissingArgument("rename")),
        _ => Err(CommandParseError::Unknown(trimmed.to_string())),
    }
}

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
            self.execute_search(&pattern);
            return;
        }

        let Some(command_input) = self.mode.take_command_input() else {
            return;
        };

        match parse_command(&command_input) {
            Ok(command) => self.execute_parsed_command(command),
            Err(error) => self.status_message = Some(error.into_status_message()),
        }
    }

    /// Execute one previously parsed command.
    fn execute_parsed_command(&mut self, command: Command) {
        // The structured command carries all parsing decisions into the mutation layer.
        match command {
            Command::GotoLine(line_num) => self.goto_line(line_num),
            Command::Edit(path) => {
                if let Err(error) = self.open_buffer(&path) {
                    self.show_status_message(format!("Error opening file: {error}"));
                }
            }
            Command::BufferNext => self.show_next_buffer(),
            Command::BufferPrev => self.show_prev_buffer(),
            Command::Buffers => {
                let listing = self.format_buffer_list();
                self.show_status_message(listing);
            }
            Command::BufferDelete => self.execute_buffer_delete(),
            Command::Quit { force, exit_code } => self.execute_quit_command(force, exit_code),
            Command::Update => self.update_current_file(PostSaveAction::StayOpen),
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
            Command::ReloadConfig => {
                self.pending_request = Some(EditorRequest::ReloadConfig);
            }
            Command::Diagnostics => self.open_diagnostics_picker(),
            Command::NextDiagnostic => self.goto_next_diagnostic(),
            Command::PrevDiagnostic => self.goto_prev_diagnostic(),
            Command::RenameSymbol(new_name) => self.request_rename(new_name),
            Command::Substitute(command) => self.execute_substitute_command(&command),
        }
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
        self.status_message = None;
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
        self.status_message = None;
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
            self.status_message = None;
            return;
        }

        self.close_active_buffer();
    }

    /// Execute one parsed substitute command against the active buffer.
    fn execute_substitute_command(&mut self, command: &SubstituteCommand) {
        let plan = match build_substitute_plan(command, &self.buffer, self.cursor.line()) {
            Ok(plan) => plan,
            Err(error) => {
                self.show_status_message(error);
                return;
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
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.last_search = Some(search);
        self.show_status_message(format_substitute_status_message(plan.substitution_count()));
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
            self.status_message = Some("Pattern not found".to_string());
            return;
        }

        // Compile first so invalid patterns do not replace the last successful search.
        let search = match SearchQuery::compile(pattern) {
            Ok(search) => search,
            Err(error) => {
                self.status_message = Some(format!("Invalid regex:\n{error}"));
                return;
            }
        };
        self.last_search = Some(search.clone());

        // Search from the current cursor first, then wrap to the document start.
        let start_idx = self.cursor.to_char_index(&self.buffer);
        if let Some(search_match) = search.find_forward(&self.buffer, start_idx) {
            self.jump_to_search_match(search_match);
            self.repeat_search_count(FindDirection::Forward, repeat_count.saturating_sub(1));
        } else if let Some(search_match) = search.find_forward(&self.buffer, 0) {
            self.jump_to_search_match(search_match);
            self.status_message = Some("Search wrapped to beginning".to_string());
            self.repeat_search_count(FindDirection::Forward, repeat_count.saturating_sub(1));
        } else {
            self.status_message = Some("Pattern not found".to_string());
        }
    }

    /// Repeat the previous search in the requested direction.
    pub(super) fn repeat_search(&mut self, direction: FindDirection) {
        let Some(search) = self.last_search.clone() else {
            self.status_message = Some("No previous search".to_string());
            return;
        };

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let total_chars = self.buffer.chars_count();

        match direction {
            FindDirection::Forward => {
                // Repeats start one character after the current match start so
                // overlapping regex matches remain reachable.
                let start_idx = cursor_idx.saturating_add(1);
                if let Some(search_match) = search.find_forward(&self.buffer, start_idx) {
                    self.jump_to_search_match(search_match);
                    return;
                }

                if let Some(search_match) = search.find_forward(&self.buffer, 0) {
                    self.jump_to_search_match(search_match);
                    self.status_message = Some("Search wrapped to beginning".to_string());
                } else {
                    self.status_message = Some("Pattern not found".to_string());
                }
            }
            FindDirection::Backward => {
                // Backward repeats exclude the current cursor position and wrap
                // against the full document when nothing earlier matches.
                if let Some(search_match) = search.find_backward(&self.buffer, cursor_idx) {
                    self.jump_to_search_match(search_match);
                } else if let Some(search_match) = search.find_backward(&self.buffer, total_chars) {
                    self.jump_to_search_match(search_match);
                    self.status_message = Some("Search wrapped to end".to_string());
                } else {
                    self.status_message = Some("Pattern not found".to_string());
                }
            }
        }
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
            self.status_message = Some(format!(
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
            self.status_message = Some("No file name".to_string());
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
            self.status_message = Some("No file name".to_string());
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
            self.status_message = Some("No file name".to_string());
            return;
        }

        let needs_overwrite_confirmation = overwrite_behavior
            == OverwriteBehavior::ConfirmIfDifferentPath
            && target_path.exists()
            && self.file_path != target_path;

        if needs_overwrite_confirmation {
            self.pending_overwrite = Some(PendingOverwrite {
                target_path,
                update_file_path,
                after_write_action,
            });
            self.status_message = None;
            return;
        }

        self.queue_write_request(target_path, update_file_path, after_write_action);
    }

    /// Queue one deferred write request for app-layer filesystem execution.
    fn queue_write_request(
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
        self.buffer.write_to(writer)
    }

    /// Apply editor-local state changes after the app layer completes one write.
    pub(crate) fn complete_deferred_write(&mut self, write: DeferredWrite) {
        if write.update_file_path {
            // Saving to a new path updates both the displayed file name and
            // syntax detection for the current buffer.
            self.file_path = write.path.clone();
            self.refresh_syntax();
            self.pending_lsp_sync_at = (!self.file_path.as_os_str().is_empty()).then(Instant::now);
        }

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
            AfterWriteAction::ContinueSessionOpenSequence {
                session_name,
                remaining_buffer_ids,
            } => {
                self.continue_session_open_sequence(session_name, remaining_buffer_ids);
            }
            AfterWriteAction::CloseActiveBuffer => self.close_active_buffer(),
        }
    }

    /// Report a filesystem error that occurred while creating a target file.
    pub(crate) fn report_file_create_error(&mut self, error: io::Error) {
        self.show_status_message(format!("Error creating file: {}", error));
    }

    /// Report a filesystem error that occurred while streaming buffer bytes.
    pub(crate) fn report_file_write_error(&mut self, error: io::Error) {
        self.show_status_message(format!("Error writing file: {}", error));
    }

    /// Remove and return the active swap handle so the app layer can clean it up.
    pub(crate) fn take_active_swap(&mut self) -> Option<SwapHandle> {
        self.swap.take()
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
            Key::Char('r') | Key::Char('R') => self.restore_pending_swap_recovery(pending),
            Key::Char('d') | Key::Char('D') => self.discard_pending_swap_recovery(pending),
            _ => {
                self.pending_swap_recovery = Some(pending);
            }
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
            self.status_message = Some("Write cancelled".to_string());
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
                self.status_message = Some("Quit cancelled".to_string());
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
                self.status_message = Some("Buffer delete cancelled".to_string());
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
                self.status_message = Some("Session open cancelled".to_string());
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
            let _previous = self.replace_active_buffer_state(replacement);
            self.reset_mode_for_buffer_switch();
            return;
        };
        // Buffer order already chose the visible successor, so resolve that
        // parked buffer and make it active in one swap.
        let target = self
            .buffer_manager
            .take_inactive_by_id(next_id)
            .expect("next buffer id should resolve to an inactive buffer");
        let _previous = self.replace_active_buffer_state(target);
        self.reset_mode_for_buffer_switch();
    }

    /// Mark the editor as ready to quit with the requested process exit code.
    pub(super) fn request_quit(&mut self, exit_code: i32) {
        self.quit_exit_code = exit_code;
        self.should_quit = true;
    }

    /// Restore the active buffer from the pending swap-recovery payload.
    fn restore_pending_swap_recovery(&mut self, pending: PendingSwapRecovery) {
        let line = self
            .cursor
            .line()
            .min(pending.recovered_buffer.lines_count().saturating_sub(1));
        let mut recovered_cursor = Cursor::new(line, self.cursor.column());
        recovered_cursor.clamp_to_line(&pending.recovered_buffer);
        self.buffer = pending.recovered_buffer;
        self.cursor = recovered_cursor;
        self.desired_visual_column = None;
        self.viewport.set_first_visible_line(0);
        self.refresh_syntax();
        self.reset_history();
        self.pending_swap_refresh_at = None;

        // Recovered content is intentionally dirty because it differs from the
        // last confirmed on-disk state even before the user makes new edits.
        self.saved_undo_depth = usize::MAX;
        self.buffer.set_modified(true);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.show_status_message("Recovered unsaved work");
    }

    /// Discard the pending recovery payload and optionally recreate a fresh swap file.
    fn discard_pending_swap_recovery(&mut self, pending: PendingSwapRecovery) {
        self.cleanup_active_swap_file();
        if pending.recreate_handle_on_discard
            && let Err(error) = self.create_active_swap_handle()
        {
            self.show_swap_unavailable_error(&error);
            return;
        }
        self.pending_swap_refresh_at = None;
        self.show_status_message("Recovery data discarded");
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

    /// Parse numeric command input as command-mode go-to-line shorthand.
    #[test]
    fn test_parse_command_parses_line_numbers() {
        assert_eq!(parse_command(" 42 "), Ok(Command::GotoLine(42)));
    }

    /// Parse `:w` paths without splitting away spaces inside the filename.
    #[test]
    fn test_parse_command_preserves_write_target_spacing() {
        assert_eq!(
            parse_command("w  notes and drafts.txt"),
            Ok(Command::Write {
                overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
                target: WriteTarget::Path("notes and drafts.txt".to_string()),
                post_save_action: PostSaveAction::StayOpen,
            })
        );
    }

    /// Parse force-write-and-quit commands into one structured write request.
    #[test]
    fn test_parse_command_parses_force_write_quit() {
        assert_eq!(
            parse_command("wq!"),
            Ok(Command::Write {
                overwrite_behavior: OverwriteBehavior::Force,
                target: WriteTarget::CurrentFile,
                post_save_action: PostSaveAction::QuitOnSuccess,
            })
        );
    }

    /// Parse substitute commands into a structured command variant.
    #[test]
    fn test_parse_command_parses_substitute_commands() {
        assert_eq!(
            parse_command("s/foo/bar/"),
            Ok(Command::Substitute(SubstituteCommand {
                scope: crate::substitute::SubstituteScope::CurrentLine,
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
            }))
        );
        assert_eq!(
            parse_command(r"%s#([a-z]+)-(\d+)#$2:$1#"),
            Ok(Command::Substitute(SubstituteCommand {
                scope: crate::substitute::SubstituteScope::WholeFile,
                pattern: r"([a-z]+)-(\d+)".to_string(),
                replacement: "$2:$1".to_string(),
            }))
        );
        assert_eq!(
            parse_command("s/foo/bar"),
            Ok(Command::Substitute(SubstituteCommand {
                scope: crate::substitute::SubstituteScope::CurrentLine,
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
            }))
        );
    }

    /// Parse both long and short aliases for buffer commands.
    #[test]
    fn test_parse_command_parses_buffer_aliases() {
        assert_eq!(parse_command("bn"), Ok(Command::BufferNext));
        assert_eq!(parse_command("buffer-prev"), Ok(Command::BufferPrev));
        assert_eq!(parse_command("ls"), Ok(Command::Buffers));
        assert_eq!(parse_command("buffer-delete"), Ok(Command::BufferDelete));
        assert_eq!(
            parse_command("save-session project-one"),
            Ok(Command::SaveSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("open-session project-one"),
            Ok(Command::OpenSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("delete-session project-one"),
            Ok(Command::DeleteSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("e notes.txt"),
            Ok(Command::Edit("notes.txt".to_string()))
        );
    }

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
        editor.pending_swap_recovery = Some(PendingSwapRecovery {
            recovered_buffer: TextBuffer::from_str("recovered"),
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
        editor.pending_swap_recovery = Some(PendingSwapRecovery {
            recovered_buffer: TextBuffer::from_str("recovered"),
            recreate_handle_on_discard: false,
        });

        assert!(editor.handle_pending_swap_recovery_key(Key::Char('d')));

        assert!(!swap_path.exists());
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Recovery data discarded")
        );
    }
}
