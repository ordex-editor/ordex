//! Command, search, save, and quit helpers for `EditorState`.

use super::*;

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

        if let Some(command) = self.mode.take_command_input() {
            let trimmed = command.trim();

            // Check for line number (go-to line)
            if let Ok(line_num) = trimmed.parse::<usize>() {
                self.goto_line(line_num);
                return;
            }

            // Parse command and arguments
            let (cmd, arg) = match trimmed.split_once(' ') {
                Some((c, a)) => (c, Some(a.trim())),
                None => (trimmed, None),
            };

            match (cmd, arg) {
                ("q", None) => {
                    if self.buffer.is_modified() {
                        self.pending_quit_confirmation = Some(PendingQuitConfirmation {
                            post_save_action: PostSaveAction::QuitOnSuccess,
                        });
                        self.status_message = None;
                    } else {
                        self.request_quit(0);
                    }
                }
                ("q!", None) => {
                    self.request_quit(0);
                }
                ("cquit", None) => {
                    self.request_quit(1);
                }
                ("update", None) => {
                    self.update_current_file(PostSaveAction::StayOpen);
                }
                ("undo", None) => {
                    self.undo_changes(1);
                }
                ("redo", None) => {
                    self.redo_changes(1);
                }
                ("w", None) => {
                    self.request_save_current(
                        OverwriteBehavior::ConfirmIfDifferentPath,
                        PostSaveAction::StayOpen,
                    );
                }
                ("w!", None) => {
                    self.request_save_current(OverwriteBehavior::Force, PostSaveAction::StayOpen);
                }
                ("w", Some(filename)) | ("write", Some(filename)) => {
                    self.request_save_as(filename, OverwriteBehavior::ConfirmIfDifferentPath);
                }
                ("w!", Some(filename)) => {
                    self.request_save_as(filename, OverwriteBehavior::Force);
                }
                ("wq", None) => {
                    self.request_save_current(
                        OverwriteBehavior::ConfirmIfDifferentPath,
                        PostSaveAction::QuitOnSuccess,
                    );
                }
                ("wq!", None) => {
                    self.request_save_current(
                        OverwriteBehavior::Force,
                        PostSaveAction::QuitOnSuccess,
                    );
                }
                ("reload-config", None) => {
                    self.pending_request = Some(EditorRequest::ReloadConfig);
                }
                _ => {
                    self.status_message = Some(format!("Unknown command: {}", trimmed));
                }
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
    /// It decides whether to queue an overwrite prompt or perform the write
    /// immediately, and applies the post-save action only on successful writes.
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

        let save_ok = self.save_to_path(target_path, update_file_path);
        if save_ok && post_save_action == PostSaveAction::QuitOnSuccess {
            self.request_quit(0);
        }
    }

    /// Execute the actual write-to-disk operation and update editor state.
    ///
    /// Returns `true` when write succeeded and state was updated, otherwise
    /// sets an error status message and returns `false`.
    pub(super) fn save_to_path(&mut self, path: PathBuf, update_file_path: bool) -> bool {
        match File::create(&path) {
            Ok(mut file) => match self.buffer.write_to(&mut file) {
                Ok(()) => {
                    if update_file_path {
                        self.file_path = path.clone();
                        self.refresh_syntax();
                    }
                    self.saved_undo_depth = self.undo_stack.len();
                    self.sync_modified_from_history();
                    self.status_message = Some(format!("\"{}\" written", path.display()));
                    true
                }
                Err(e) => {
                    self.status_message = Some(format!("Error writing file: {}", e));
                    false
                }
            },
            Err(e) => {
                self.status_message = Some(format!("Error creating file: {}", e));
                false
            }
        }
    }

    /// Consume one key while an overwrite prompt is pending.
    ///
    /// `y`/`Y` confirms and executes the deferred write; any other key cancels.
    /// Returns `true` when this function consumed the key.
    pub(super) fn handle_pending_overwrite_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_overwrite.take() else {
            return false;
        };

        let confirmed = key == Key::Char('y') || key == Key::Char('Y');
        if confirmed {
            let save_ok = self.save_to_path(pending.target_path, pending.update_file_path);
            if save_ok && pending.post_save_action == PostSaveAction::QuitOnSuccess {
                self.request_quit(0);
            }
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
