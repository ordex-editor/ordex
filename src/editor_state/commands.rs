//! Command, search, save, and quit helpers for `EditorState`.

use super::*;
use std::io::{self, Write};

/// Parsed command-mode input that is ready for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    GotoLine(usize),
    Quit {
        force: bool,
        exit_code: i32,
    },
    Update,
    Undo,
    Redo,
    Write {
        overwrite_behavior: OverwriteBehavior,
        target: WriteTarget,
        post_save_action: PostSaveAction,
    },
    ReloadConfig,
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
}

impl CommandParseError {
    /// Convert a parse error into the status message shown to the user.
    fn into_status_message(self) -> String {
        match self {
            Self::Unknown(command) => format!("Unknown command: {}", command),
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
    /// Command mode supports save/quit commands and numeric go-to-line input.
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
            Command::Quit { force, exit_code } => self.execute_quit_command(force, exit_code),
            Command::Update => self.update_current_file(PostSaveAction::StayOpen),
            Command::Undo => self.undo_changes(1),
            Command::Redo => self.redo_changes(1),
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
        }
    }

    /// Execute a parsed quit command while preserving confirmation behavior.
    fn execute_quit_command(&mut self, force: bool, exit_code: i32) {
        if force {
            self.request_quit(exit_code);
            return;
        }

        // Plain `:q` prompts when the buffer is dirty.
        if self.buffer.is_modified() {
            self.pending_quit_confirmation = Some(PendingQuitConfirmation {
                post_save_action: PostSaveAction::QuitOnSuccess,
            });
            self.status_message = None;
            return;
        }

        self.request_quit(exit_code);
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

    pub(super) fn execute_search(&mut self, pattern: &str) {
        if pattern.is_empty() {
            self.status_message = Some("Pattern not found".to_string());
            return;
        }

        self.last_search_pattern = Some(pattern.to_string());

        // Search from current position.
        let start_idx = self.cursor.to_char_index(&self.buffer);
        if let Some(found_idx) = self.buffer.find(pattern, start_idx) {
            self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        } else {
            // Wrap around to beginning
            if let Some(found_idx) = self.buffer.find(pattern, 0) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
                self.status_message = Some("Search wrapped to beginning".to_string());
            } else {
                self.status_message = Some("Pattern not found".to_string());
            }
        }
    }

    pub(super) fn repeat_search(&mut self, forward: bool) {
        let Some(pattern) = self.last_search_pattern.clone() else {
            self.status_message = Some("No previous search".to_string());
            return;
        };

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let total_chars = self.buffer.chars_count();

        if forward {
            let start_idx = cursor_idx.saturating_add(1);
            if let Some(found_idx) = self.buffer.find(&pattern, start_idx) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                return;
            }

            if let Some(found_idx) = self.buffer.find(&pattern, 0) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                self.status_message = Some("Search wrapped to beginning".to_string());
            } else {
                self.status_message = Some("Pattern not found".to_string());
            }
        } else {
            if let Some(found_idx) = self.buffer.find_backward(&pattern, cursor_idx) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                return;
            }

            if let Some(found_idx) = self.buffer.find_backward(&pattern, total_chars) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                self.status_message = Some("Search wrapped to end".to_string());
            } else {
                self.status_message = Some("Pattern not found".to_string());
            }
        }
    }

    /// Repeat search motion `count` times while preserving existing wrap/error behavior.
    pub(super) fn repeat_search_count(&mut self, forward: bool, count: usize) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.repeat_search(forward);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

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

        self.cursor = Cursor::new(target_line, 0);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Save the current file only when the buffer is dirty.
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
        if self.file_path.as_os_str().is_empty() {
            self.status_message = Some("No file name".to_string());
            return;
        }

        self.request_save(
            self.file_path.clone(),
            false,
            overwrite_behavior,
            post_save_action,
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
            PostSaveAction::StayOpen,
        );
    }

    /// Shared save request pipeline for all write commands.
    ///
    /// It decides whether to queue an overwrite prompt or defer the filesystem
    /// write to the app layer, and applies the post-save action only after a
    /// successful deferred write completes.
    pub(super) fn request_save(
        &mut self,
        target_path: PathBuf,
        update_file_path: bool,
        overwrite_behavior: OverwriteBehavior,
        post_save_action: PostSaveAction,
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
                post_save_action,
            });
            self.status_message = None;
            return;
        }

        self.queue_write_request(target_path, update_file_path, post_save_action);
    }

    /// Queue one deferred write request for app-layer filesystem execution.
    fn queue_write_request(
        &mut self,
        path: PathBuf,
        update_file_path: bool,
        post_save_action: PostSaveAction,
    ) {
        self.pending_request = Some(EditorRequest::WriteBuffer(DeferredWrite {
            path,
            update_file_path,
            quit_on_success: post_save_action == PostSaveAction::QuitOnSuccess,
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
        }

        // The current undo depth becomes the clean on-disk reference point.
        self.saved_undo_depth = self.undo_stack.len();
        self.sync_modified_from_history();
        self.show_status_message(format!("\"{}\" written", write.path.display()));
        if write.quit_on_success {
            self.request_quit(0);
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

    /// Consume one key while an overwrite prompt is pending.
    ///
    /// `y`/`Y` confirms and queues the deferred write; any other key cancels.
    /// Returns `true` when this function consumed the key.
    pub(super) fn handle_pending_overwrite_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_overwrite.take() else {
            return false;
        };

        let confirmed = key == Key::Char('y') || key == Key::Char('Y');
        if confirmed {
            self.queue_write_request(
                pending.target_path,
                pending.update_file_path,
                pending.post_save_action,
            );
        } else {
            self.status_message = Some("Write cancelled".to_string());
        }

        true
    }

    /// Consume one key while a quit confirmation prompt is pending.
    ///
    /// `y`/`Y` saves and quits on success, `n`/`N` quits without saving, and
    /// any other key cancels quit.
    /// Returns `true` when this function consumed the key.
    pub(super) fn handle_pending_quit_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_quit_confirmation.take() else {
            return false;
        };

        match key {
            Key::Char('y') | Key::Char('Y') => {
                self.request_save_current(
                    OverwriteBehavior::ConfirmIfDifferentPath,
                    pending.post_save_action,
                );
            }
            Key::Char('n') | Key::Char('N') => {
                self.request_quit(0);
            }
            _ => {
                self.status_message = Some("Quit cancelled".to_string());
            }
        }

        true
    }

    /// Mark the editor as ready to quit with the requested process exit code.
    pub(super) fn request_quit(&mut self, exit_code: i32) {
        self.quit_exit_code = exit_code;
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Reject unsupported commands so execution never sees raw command text.
    #[test]
    fn test_parse_command_rejects_unknown_commands() {
        assert_eq!(
            parse_command("write"),
            Err(CommandParseError::Unknown("write".to_string()))
        );
    }
}
