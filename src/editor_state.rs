//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::cursor::Cursor;
use crate::keybindings::{Action, KeyBindings, KeyInput, SequenceMatch};
use crate::mode::Mode;
use crate::navigation::{find_next_word_start, find_prev_word_start, find_word_end};
use crate::text_buffer::TextBuffer;
use crate::viewport::Viewport;
use std::fs::File;
use std::path::PathBuf;
use termion::event::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindMotionKind {
    Find,
    Till,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FindMotion {
    kind: FindMotionKind,
    direction: FindDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastFind {
    motion: FindMotion,
    target: char,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOverwrite {
    target_path: PathBuf,
    update_file_path: bool,
    post_save_action: PostSaveAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingQuitConfirmation {
    post_save_action: PostSaveAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverwriteBehavior {
    ConfirmIfDifferentPath,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostSaveAction {
    StayOpen,
    QuitOnSuccess,
}

/// Editor state holding all components for the editor session
pub(crate) struct EditorState {
    /// The text buffer containing file content
    pub(crate) buffer: TextBuffer,
    /// Current cursor position
    pub(crate) cursor: Cursor,
    /// Current editor mode
    pub(crate) mode: Mode,
    /// Viewport for visible portion of document
    pub(crate) viewport: Viewport,
    /// Path to the file being edited
    pub(crate) file_path: PathBuf,
    /// Status message to display (cleared after one render)
    pub(crate) status_message: Option<String>,
    /// Key bindings configuration
    keybindings: KeyBindings,
    /// Flag indicating the editor should quit
    pub(crate) should_quit: bool,
    /// Last non-empty search pattern used by / search.
    last_search_pattern: Option<String>,
    /// Pending multi-key sequence in normal mode (e.g. 'g' waiting for continuation).
    pending_sequence: Vec<KeyInput>,
    /// Pending find/till motion waiting for a target character.
    pending_find: Option<FindMotion>,
    /// Last attempted character find/till motion used by ';' and ','.
    last_find: Option<LastFind>,
    /// Pending overwrite confirmation for save commands targeting an existing file.
    pending_overwrite: Option<PendingOverwrite>,
    /// Pending quit confirmation for `:q` with unsaved changes.
    pending_quit_confirmation: Option<PendingQuitConfirmation>,
}

impl EditorState {
    /// Create a new editor state with an empty buffer
    pub(crate) fn new(terminal_height: usize) -> Self {
        Self {
            buffer: TextBuffer::new(),
            cursor: Cursor::new(0, 0),
            mode: Mode::Normal,
            viewport: Viewport::new(terminal_height.saturating_sub(2)), // Reserve 2 lines for status bar
            file_path: PathBuf::new(),
            status_message: None,
            keybindings: KeyBindings::new(),
            should_quit: false,
            last_search_pattern: None,
            pending_sequence: Vec::new(),
            pending_find: None,
            last_find: None,
            pending_overwrite: None,
            pending_quit_confirmation: None,
        }
    }

    /// Load a file into the editor using chunked reading for efficiency
    pub(crate) fn load_file(&mut self, path: &str) -> std::io::Result<()> {
        let file = File::open(path)?;
        self.buffer = TextBuffer::from_reader(file)?;
        self.file_path = PathBuf::from(path);
        self.cursor = Cursor::new(0, 0);
        self.viewport.set_first_visible_line(0);
        Ok(())
    }

    /// Update viewport dimensions after a terminal resize.
    pub(crate) fn handle_resize(&mut self, terminal_width: usize, terminal_height: usize) {
        self.viewport.set_width(terminal_width);
        self.viewport.set_height(terminal_height.saturating_sub(2));
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Handle a key press and update editor state
    pub(crate) fn handle_key(&mut self, key: Key) {
        if self.handle_pending_overwrite_key(key) {
            return;
        }

        if self.handle_pending_quit_key(key) {
            return;
        }

        if self.handle_pending_find_key(key) {
            return;
        }

        if self.handle_pending_sequence_key(key) {
            return;
        }

        if self.mode.is_normal() {
            let key_input = KeyInput::from(key);
            if self
                .keybindings
                .starts_sequence_prefix(&self.mode, &key_input)
            {
                self.pending_sequence.clear();
                self.pending_sequence.push(key_input);
                return;
            }
        }

        // First check bindings map
        if let Some(action) = self.keybindings.get_action(key, &self.mode) {
            self.execute_action(action);
            return;
        }

        // Handle insertable characters for insert/command/search modes
        if let Some(c) = KeyBindings::is_insertable_char(key) {
            if self.mode.is_normal() {
                // Unbound key in normal mode - ignore
                return;
            }

            if self.mode == Mode::Insert {
                self.insert_char(c);
            } else {
                self.mode.append_char(c);
            }
        }
    }

    fn execute_action(&mut self, action: Action) {
        match action {
            // Navigation
            Action::MoveLeft => {
                if self.mode.is_normal() {
                    self.cursor.move_left_normal();
                } else {
                    self.cursor.move_left(&self.buffer);
                }
            }
            Action::MoveRight => {
                if self.mode.is_normal() {
                    self.cursor.move_right_normal(&self.buffer);
                } else {
                    self.cursor.move_right(&self.buffer);
                }
            }
            Action::MoveUp => {
                if self.mode.is_normal() {
                    self.cursor.move_up_normal(&self.buffer);
                } else {
                    self.cursor.move_up(&self.buffer);
                }
            }
            Action::MoveDown => {
                if self.mode.is_normal() {
                    self.cursor.move_down_normal(&self.buffer);
                } else {
                    self.cursor.move_down(&self.buffer);
                }
            }
            Action::MoveWordForward => self.move_word_forward(),
            Action::MoveWordBackward => self.move_word_backward(),
            Action::MoveWordEnd => self.move_word_end(),
            Action::MoveLineStart => self.cursor.move_to_line_start(),
            Action::MoveLineEnd => self.cursor.move_to_line_end(&self.buffer),
            Action::MovePastLineEnd => self.cursor.move_past_line_end(&self.buffer),
            Action::MoveFirstNonBlank => self.move_first_non_blank(),
            Action::MoveToFirstLine => self.move_to_first_line(),
            Action::MoveToLastLine => self.move_to_last_line(),
            Action::PageUp => self.viewport.page_up(&mut self.cursor, &self.buffer),
            Action::PageDown => self.viewport.page_down(&mut self.cursor, &self.buffer),
            Action::FindForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Forward,
                });
            }
            Action::FindBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Backward,
                });
            }
            Action::TillForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Forward,
                });
            }
            Action::TillBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Backward,
                });
            }
            Action::RepeatFindForward => self.repeat_find(false),
            Action::RepeatFindBackward => self.repeat_find(true),

            // Mode switching
            Action::EnterInsertMode => self.mode = Mode::Insert,
            Action::OpenLineBelow => self.open_line_below(),
            Action::OpenLineAbove => self.open_line_above(),
            Action::EnterCommandMode => self.mode = Mode::Command(String::new()),
            Action::EnterSearchMode => self.mode = Mode::Search(String::new()),
            Action::ExitToNormalMode => self.mode = Mode::Normal,
            Action::SearchNext => self.repeat_search(true),
            Action::SearchPrevious => self.repeat_search(false),

            // Insert mode
            Action::DeleteCharBackward => self.delete_char_backward(),
            Action::DeleteCharForward => self.delete_char_forward(),
            Action::DeleteWordBackward => self.delete_word_backward(),
            Action::DeleteToLineStart => self.delete_to_line_start(),
            Action::InsertNewline => self.insert_newline(),

            // Command/Search mode
            Action::ExecuteCommand => self.execute_command(),
            Action::CancelCommand => self.mode = Mode::Normal,
            Action::DeleteInputChar => self.delete_input_char(),
        }

        // In normal mode, cursor must stay on a real character for non-empty lines.
        if self.mode.is_normal() {
            self.cursor.clamp_to_line_normal(&self.buffer);
        } else {
            self.pending_sequence.clear();
            self.pending_find = None;
        }

        // Ensure cursor is visible after any action
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    fn move_word_forward(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_next_word_start(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    fn move_word_backward(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_prev_word_start(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    fn move_word_end(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_word_end(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    fn move_first_non_blank(&mut self) {
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

    fn move_to_last_line(&mut self) {
        let last_line = self.buffer.lines_count().saturating_sub(1);
        self.cursor = Cursor::new(last_line, 0);
    }

    fn move_to_first_line(&mut self) {
        self.cursor = Cursor::new(0, self.cursor.desired_column());
    }

    fn begin_find_motion(&mut self, motion: FindMotion) {
        self.pending_sequence.clear();
        self.pending_find = Some(motion);
    }

    fn handle_pending_find_key(&mut self, key: Key) -> bool {
        let Some(motion) = self.pending_find else {
            return false;
        };
        if !self.mode.is_normal() {
            self.pending_find = None;
            return false;
        }

        if matches!(key, Key::Esc) {
            self.pending_find = None;
            return true;
        }

        if let Some(target) = KeyBindings::is_insertable_char(key) {
            self.pending_find = None;
            self.apply_find_motion(motion, target, true);
            self.cursor.clamp_to_line_normal(&self.buffer);
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        }

        // While waiting for find target, consume all keys to avoid accidental mode switches.
        true
    }

    fn handle_pending_sequence_key(&mut self, key: Key) -> bool {
        if !self.mode.is_normal() || self.pending_sequence.is_empty() {
            return false;
        }

        if matches!(key, Key::Esc) {
            self.pending_sequence.clear();
            return true;
        }

        self.pending_sequence.push(KeyInput::from(key));
        match self
            .keybindings
            .match_sequence(&self.mode, &self.pending_sequence)
        {
            SequenceMatch::Exact(action) => {
                self.pending_sequence.clear();
                self.execute_action(action);
            }
            SequenceMatch::Prefix => {}
            SequenceMatch::NoMatch => {
                self.pending_sequence.clear();
            }
        }
        true
    }

    fn apply_find_motion(&mut self, motion: FindMotion, target: char, update_last_find: bool) {
        if update_last_find {
            self.last_find = Some(LastFind { motion, target });
        }

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let Some(target_idx) = self.find_char_on_current_line(cursor_idx, motion.direction, target)
        else {
            return;
        };

        let destination = match (motion.kind, motion.direction) {
            (FindMotionKind::Find, _) => target_idx,
            (FindMotionKind::Till, FindDirection::Forward) => target_idx.saturating_sub(1),
            (FindMotionKind::Till, FindDirection::Backward) => target_idx.saturating_add(1),
        };

        self.cursor = Cursor::from_char_index(&self.buffer, destination);
    }

    fn repeat_find(&mut self, reverse_direction: bool) {
        let Some(last) = self.last_find else {
            return;
        };

        let direction = if reverse_direction {
            match last.motion.direction {
                FindDirection::Forward => FindDirection::Backward,
                FindDirection::Backward => FindDirection::Forward,
            }
        } else {
            last.motion.direction
        };

        let motion = FindMotion {
            kind: last.motion.kind,
            direction,
        };
        self.apply_find_motion(motion, last.target, false);
    }

    fn find_char_on_current_line(
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

    fn insert_char(&mut self, c: char) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.buffer.insert(char_idx, &c.to_string());
        self.cursor.move_right(&self.buffer);
    }

    fn insert_newline(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.buffer.insert(char_idx, "\n");
        self.cursor.move_down(&self.buffer);
        self.cursor.set_column(0);
    }

    fn open_line_below(&mut self) {
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.buffer.insert(line_end, "\n");
        self.cursor = Cursor::new(line + 1, 0);
        self.mode = Mode::Insert;
    }

    fn open_line_above(&mut self) {
        let line = self.cursor.line();
        let line_start = self.buffer.line_to_char(line);
        self.buffer.insert(line_start, "\n");
        self.cursor = Cursor::new(line, 0);
        self.mode = Mode::Insert;
    }

    fn delete_char_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx > 0 {
            self.cursor.move_left(&self.buffer);
            self.buffer.remove(char_idx - 1, char_idx);
        }
    }

    fn delete_char_forward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx < self.buffer.chars_count() {
            self.buffer.remove(char_idx, char_idx + 1);
        }
    }

    fn delete_word_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx == 0 {
            return;
        }

        let word_start = find_prev_word_start(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, word_start);
        self.buffer.remove(word_start, char_idx);
    }

    fn delete_to_line_start(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let line = self.cursor.line();
        let col = self.cursor.column();
        if col == 0 {
            return;
        }

        // Get the start of the current line in char index
        let line_start = self.buffer.line_to_char(line);
        let char_idx = self.cursor.to_char_index(&self.buffer);

        self.cursor.set_column(0);
        self.buffer.remove(line_start, char_idx);
    }

    fn delete_input_char(&mut self) {
        self.mode.pop_char();
    }

    /// Execute the current command/search input and apply side effects.
    ///
    /// Command mode supports save/quit commands and numeric go-to-line input.
    fn execute_command(&mut self) {
        // Extract the input from the mode, taking ownership
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);

        match mode {
            Mode::Search(pattern) => {
                self.execute_search(&pattern);
            }
            Mode::Command(command) => {
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
                            self.should_quit = true;
                        }
                    }
                    ("q!", None) => {
                        self.should_quit = true;
                    }
                    ("w", None) => {
                        self.request_save_current(
                            OverwriteBehavior::ConfirmIfDifferentPath,
                            PostSaveAction::StayOpen,
                        );
                    }
                    ("w!", None) => {
                        self.request_save_current(
                            OverwriteBehavior::Force,
                            PostSaveAction::StayOpen,
                        );
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
                    _ => {
                        self.status_message = Some(format!("Unknown command: {}", trimmed));
                    }
                }
            }
            _ => {
                // Restore the mode if it wasn't Command or Search
                self.mode = mode;
            }
        }
    }

    fn execute_search(&mut self, pattern: &str) {
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

    fn repeat_search(&mut self, forward: bool) {
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

    fn goto_line(&mut self, line_num: usize) {
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

    /// Request a save to the current file path.
    ///
    /// This centralizes `:w` and `:wq` behavior while keeping overwrite and
    /// post-save handling explicit at the callsite.
    fn request_save_current(
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
    fn request_save_as(&mut self, filename: &str, overwrite_behavior: OverwriteBehavior) {
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
    fn request_save(
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
            self.should_quit = true;
        }
    }

    /// Execute the actual write-to-disk operation and update editor state.
    ///
    /// Returns `true` when write succeeded and state was updated, otherwise
    /// sets an error status message and returns `false`.
    fn save_to_path(&mut self, path: PathBuf, update_file_path: bool) -> bool {
        match File::create(&path) {
            Ok(mut file) => match self.buffer.write_to(&mut file) {
                Ok(()) => {
                    if update_file_path {
                        self.file_path = path.clone();
                    }
                    self.buffer.clear_modified();
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
    fn handle_pending_overwrite_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_overwrite.take() else {
            return false;
        };

        let confirmed = key == Key::Char('y') || key == Key::Char('Y');
        if confirmed {
            let save_ok = self.save_to_path(pending.target_path, pending.update_file_path);
            if save_ok && pending.post_save_action == PostSaveAction::QuitOnSuccess {
                self.should_quit = true;
            }
        } else {
            self.status_message = Some("Write cancelled".to_string());
        }

        true
    }

    /// Get the current mode name for display
    pub(crate) fn mode_name(&self) -> &str {
        match &self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command(_) => "COMMAND",
            Mode::Search(_) => "SEARCH",
        }
    }

    /// Get the command/search input string for display
    pub(crate) fn input_line(&self) -> Option<&str> {
        self.mode
            .command_string()
            .or_else(|| self.mode.search_string())
    }

    /// Get the prompt character for command/search mode
    pub(crate) fn input_prompt(&self) -> Option<char> {
        match &self.mode {
            Mode::Command(_) => Some(':'),
            Mode::Search(_) => Some('/'),
            _ => None,
        }
    }

    pub(crate) fn overwrite_prompt(&self) -> Option<String> {
        self.pending_overwrite
            .as_ref()
            .map(|pending| format!("Overwrite \"{}\"? [y/N]", pending.target_path.display()))
    }

    pub(crate) fn quit_prompt(&self) -> Option<String> {
        if self.pending_quit_confirmation.is_none() {
            return None;
        }

        let file_name = self
            .file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("[No Name]");
        Some(format!(
            "Save changes to \"{}\"? [y]es/[n]o/[c]ancel",
            file_name
        ))
    }

    /// Get a short pending multi-key prefix label for UI display.
    pub(crate) fn pending_prefix_label(&self) -> Option<String> {
        if !self.mode.is_normal() {
            return None;
        }

        if let Some(motion) = self.pending_find {
            let label = match (motion.kind, motion.direction) {
                (FindMotionKind::Find, FindDirection::Forward) => "f",
                (FindMotionKind::Find, FindDirection::Backward) => "F",
                (FindMotionKind::Till, FindDirection::Forward) => "t",
                (FindMotionKind::Till, FindDirection::Backward) => "T",
            };
            return Some(label.to_string());
        }

        if self.pending_sequence.is_empty() {
            return None;
        }

        let mut label = String::new();
        for key in &self.pending_sequence {
            match key {
                KeyInput::Char(c) => label.push(*c),
                KeyInput::Ctrl(c) => label.push_str(&format!("^{}", c)),
                KeyInput::Alt(c) => label.push_str(&format!("M-{}", c)),
                KeyInput::Backspace => label.push_str("BS"),
                KeyInput::Escape => label.push_str("Esc"),
                KeyInput::Up => label.push_str("Up"),
                KeyInput::Down => label.push_str("Down"),
                KeyInput::Left => label.push_str("Left"),
                KeyInput::Right => label.push_str("Right"),
                KeyInput::Home => label.push_str("Home"),
                KeyInput::End => label.push_str("End"),
                KeyInput::PageUp => label.push_str("PgUp"),
                KeyInput::PageDown => label.push_str("PgDn"),
                KeyInput::Delete => label.push_str("Del"),
                KeyInput::Insert => label.push_str("Ins"),
                KeyInput::F(n) => label.push_str(&format!("F{}", n)),
            }
        }
        Some(label)
    }

    /// Consume one key while a quit confirmation prompt is pending.
    ///
    /// `y`/`Y` saves and quits on success, `n`/`N` quits without saving, and
    /// any other key cancels quit.
    fn handle_pending_quit_key(&mut self, key: Key) -> bool {
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
                self.should_quit = true;
            }
            _ => {
                self.status_message = Some("Quit cancelled".to_string());
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_editor_with_content(content: &str) -> EditorState {
        let mut editor = EditorState::new(24);
        editor.buffer = TextBuffer::from_str(content);
        editor
    }

    fn unique_temp_path(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos));
        path.to_string_lossy().to_string()
    }

    #[test]
    fn test_hjkl_navigation() {
        let mut editor = create_editor_with_content("hello\nworld\ntest");

        // Move right
        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.cursor.column(), 1);

        // Move down
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 1);

        // Move left
        editor.handle_key(Key::Char('h'));
        assert_eq!(editor.cursor.column(), 0);

        // Move up
        editor.handle_key(Key::Char('k'));
        assert_eq!(editor.cursor.line(), 0);
    }

    #[test]
    fn test_word_navigation() {
        let mut editor = create_editor_with_content("hello world test");

        // Move to next word
        editor.handle_key(Key::Char('w'));
        assert_eq!(editor.cursor.column(), 6); // 'w' of world

        // Move to next word again
        editor.handle_key(Key::Char('w'));
        assert_eq!(editor.cursor.column(), 12); // 't' of test

        // Move back
        editor.handle_key(Key::Char('b'));
        assert_eq!(editor.cursor.column(), 6); // 'w' of world
    }

    #[test]
    fn test_enter_insert_mode() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('i'));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_exit_insert_mode() {
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::Insert;

        editor.handle_key(Key::Esc);
        assert!(matches!(editor.mode, Mode::Normal));
    }

    #[test]
    fn test_open_line_below_enters_insert_mode() {
        let mut editor = create_editor_with_content("line1\nline2");
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "line1\n\nline2");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_above_enters_insert_mode() {
        let mut editor = create_editor_with_content("line1\nline2");
        editor.cursor = Cursor::new(1, 3);

        editor.handle_key(Key::Char('O'));

        assert_eq!(editor.buffer.to_string(), "line1\n\nline2");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_insert_character() {
        let mut editor = create_editor_with_content("hllo");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('e'));
        assert_eq!(editor.buffer.to_string(), "hello");
    }

    #[test]
    fn test_command_mode() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char(':'));
        assert!(matches!(editor.mode, Mode::Command(_)));

        editor.handle_key(Key::Char('q'));
        if let Mode::Command(ref input) = editor.mode {
            assert_eq!(input, "q");
        }

        editor.handle_key(Key::Char('\n'));
        assert!(editor.should_quit);
    }

    #[test]
    fn test_goto_line() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4\nline5");

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.cursor.line(), 2); // 0-indexed
    }

    #[test]
    fn test_search() {
        let mut editor = create_editor_with_content("hello world\nfoo bar");

        editor.handle_key(Key::Char('/'));
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_search_next_and_previous() {
        let mut editor = create_editor_with_content("target\nx\ntarget\n");

        editor.handle_key(Key::Char('/'));
        for c in "target\n".chars() {
            editor.handle_key(Key::Char(c));
        }
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('n'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('N'));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_search_repeat_without_previous_search() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('n'));
        assert_eq!(
            editor.status_message,
            Some("No previous search".to_string())
        );
    }

    #[test]
    fn test_handle_resize_keeps_cursor_visible() {
        let mut editor = create_editor_with_content("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
        editor.cursor = Cursor::new(9, 0);

        editor.handle_resize(80, 4);

        assert!(
            editor
                .viewport
                .visible_range()
                .contains(&editor.cursor.line())
        );
    }

    #[test]
    fn test_boundary_protection_left() {
        let mut editor = create_editor_with_content("hello");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('h'));
        assert_eq!(editor.cursor.column(), 0); // Should not go negative
    }

    #[test]
    fn test_boundary_protection_up() {
        let mut editor = create_editor_with_content("hello\nworld");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('k'));
        assert_eq!(editor.cursor.line(), 0); // Should not go negative
    }

    #[test]
    fn test_boundary_protection_right_in_normal_mode() {
        let mut editor = create_editor_with_content("ab");
        editor.cursor = Cursor::new(0, 1); // Last character

        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.cursor.column(), 1); // Should not go past end in normal mode
    }

    #[test]
    fn test_exit_insert_mode_clamps_from_past_line_end() {
        let mut editor = create_editor_with_content("ab");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 2); // Insert-mode valid position (past end)

        editor.handle_key(Key::Esc);
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 1); // Last character in normal mode
    }

    #[test]
    fn test_input_line_returns_str_slice() {
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::Command("test".to_string());

        let input = editor.input_line();
        assert_eq!(input, Some("test"));
    }

    #[test]
    fn test_move_line_start() {
        let mut editor = create_editor_with_content("hello world");
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('0'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_move_line_end() {
        let mut editor = create_editor_with_content("hello world");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('$'));
        assert_eq!(editor.cursor.column(), 10); // 'd' is at index 10
    }

    #[test]
    fn test_move_first_non_blank() {
        let mut editor = create_editor_with_content("   hello world");
        editor.cursor = Cursor::new(0, 10);

        editor.handle_key(Key::Char('^'));
        assert_eq!(editor.cursor.column(), 3); // 'h' is at index 3
    }

    #[test]
    fn test_move_to_last_line() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('G'));
        assert_eq!(editor.cursor.line(), 3); // Last line (0-indexed)
    }

    #[test]
    fn test_move_word_end() {
        let mut editor = create_editor_with_content("hello world test");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('e'));
        assert_eq!(editor.cursor.column(), 4); // 'o' of hello

        editor.handle_key(Key::Char('e'));
        assert_eq!(editor.cursor.column(), 10); // 'd' of world
    }

    #[test]
    fn test_save_file_as_with_w_command() {
        let target = unique_temp_path("ordex_test_save_as");
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::Command(format!("w {}", target));

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.file_path.to_string_lossy(), target);
        assert!(!editor.buffer.is_modified());
        assert!(editor.status_message.as_ref().unwrap().contains("written"));

        // Cleanup
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_save_file_as_with_write_command() {
        let target = unique_temp_path("ordex_test_write");
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::Command(format!("write {}", target));

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.file_path.to_string_lossy(), target);
        assert!(!editor.buffer.is_modified());

        // Cleanup
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_save_file_as_updates_file_path() {
        let target = unique_temp_path("ordex_new_file");
        let mut editor = create_editor_with_content("new file content");
        assert!(editor.file_path.as_os_str().is_empty());

        editor.mode = Mode::Command(format!("w {}", target));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.file_path.to_string_lossy(), target);

        // Cleanup
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_save_without_filename_shows_error() {
        let mut editor = create_editor_with_content("some content");
        assert!(editor.file_path.as_os_str().is_empty());

        // Try to save without filename
        editor.mode = Mode::Command("w".to_string());
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_w_current_file_writes_without_confirmation() {
        let target = unique_temp_path("ordex_confirm_write");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.mode = Mode::Command("w".to_string());
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.overwrite_prompt(), None);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(
            editor
                .status_message
                .as_deref()
                .unwrap()
                .contains("written")
        );

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_w_save_as_existing_file_cancel_keeps_target_unchanged() {
        let source = unique_temp_path("ordex_save_as_source");
        let target = unique_temp_path("ordex_confirm_cancel");
        fs::write(&source, "source_old").unwrap();
        fs::write(&target, "target_old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&source);
        editor.mode = Mode::Command(format!("w {}", target));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.overwrite_prompt(),
            Some(format!("Overwrite \"{}\"? [y/N]", target))
        );
        editor.handle_key(Key::Esc);

        assert_eq!(fs::read_to_string(&target).unwrap(), "target_old");
        assert_eq!(editor.status_message, Some("Write cancelled".to_string()));
        assert_eq!(editor.file_path.to_string_lossy(), source);

        let _ = fs::remove_file(source);
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_w_bang_bypasses_confirmation_for_existing_file() {
        let target = unique_temp_path("ordex_force_write");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.mode = Mode::Command("w!".to_string());
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.overwrite_prompt(), None);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_wq_current_file_writes_and_quits_without_confirmation() {
        let target = unique_temp_path("ordex_wq_cancel");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.mode = Mode::Command("wq".to_string());
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_wq_force_no_file_name_does_not_quit() {
        let mut editor = create_editor_with_content("new");
        editor.mode = Mode::Command("wq!".to_string());
        editor.handle_key(Key::Char('\n'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_q_modified_buffer_does_not_quit_immediately() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));

        assert!(!editor.should_quit);
    }

    #[test]
    fn test_q_bang_quits_with_unsaved_changes() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q!".to_string());
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
    }

    #[test]
    fn test_q_modified_buffer_shows_quit_prompt_with_base_name() {
        let mut editor = create_editor_with_content("abc");
        editor.file_path = PathBuf::from("/tmp/ordex_test_name.txt");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.quit_prompt(),
            Some("Save changes to \"ordex_test_name.txt\"? [y]es/[n]o/[c]ancel".to_string())
        );
        assert!(!editor.should_quit);
    }

    #[test]
    fn test_q_modified_buffer_n_quits_without_saving() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('n'));

        assert!(editor.should_quit);
    }

    #[test]
    fn test_q_modified_buffer_c_cancels_quit() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('c'));

        assert!(!editor.should_quit);
        assert_eq!(editor.quit_prompt(), None);
        assert_eq!(editor.status_message, Some("Quit cancelled".to_string()));
    }

    #[test]
    fn test_q_modified_buffer_other_key_cancels_quit() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Esc);

        assert!(!editor.should_quit);
        assert_eq!(editor.quit_prompt(), None);
        assert_eq!(editor.status_message, Some("Quit cancelled".to_string()));
    }

    #[test]
    fn test_q_unmodified_buffer_quits_directly() {
        let mut editor = create_editor_with_content("abc");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(editor.quit_prompt(), None);
    }

    #[test]
    fn test_q_unnamed_buffer_y_shows_no_file_name_and_does_not_quit() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::Command("q".to_string());
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('y'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_find_forward_and_backward_on_current_line() {
        let mut editor = create_editor_with_content("abca");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char('F'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_till_forward_and_backward() {
        let mut editor = create_editor_with_content("abcde");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('t'));
        editor.handle_key(Key::Char('d'));
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char('T'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_till_adjacent_target_stays_in_place() {
        let mut editor = create_editor_with_content("abc");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('t'));
        editor.handle_key(Key::Char('b'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_find_does_not_cross_line_boundaries() {
        let mut editor = create_editor_with_content("abc\nxa");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    fn test_repeat_find_semicolon_and_comma() {
        let mut editor = create_editor_with_content("abca");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);

        // ';' repeats original find direction (forward), not the temporary ',' opposite direction.
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    fn test_repeat_find_without_previous_motion_is_silent() {
        let mut editor = create_editor_with_content("abc");
        editor.status_message = None;
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.status_message, None);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    fn test_failed_repeat_attempt_does_not_change_base_repeat_direction() {
        let mut editor = create_editor_with_content("cxxc");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('c'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char('0'));
        assert_eq!(editor.cursor.column(), 0);

        // Opposite direction repeat fails at line start.
        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);

        // ';' keeps the original forward direction and should jump to the next match.
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    fn test_failed_find_then_semicolon_on_line_with_match_moves_cursor() {
        let mut editor = create_editor_with_content("bbbb\naxxa");
        editor.cursor = Cursor::new(0, 0);

        // Fail to find 'a' on first line, but keep last-find state.
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor, Cursor::new(0, 0));

        // Move to a line where the same motion has a match and repeat it.
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor, Cursor::new(1, 0));

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor, Cursor::new(1, 3));
    }

    #[test]
    fn test_semicolon_repeatedly_moves_in_base_direction() {
        let mut editor = create_editor_with_content("abacada");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 4);

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 6);

        // No further match, so repeated ';' stays put.
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 6);
    }

    #[test]
    fn test_comma_repeatedly_moves_in_opposite_direction() {
        let mut editor = create_editor_with_content("abacada");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char(';'));
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 6);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 4);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);

        // No further match in opposite direction, so repeated ',' stays put.
        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_find_pending_indicator_and_escape_cancel() {
        let mut editor = create_editor_with_content("abc");

        editor.handle_key(Key::Char('f'));
        assert_eq!(editor.pending_prefix_label(), Some("f".to_string()));

        editor.handle_key(Key::Esc);
        assert_eq!(editor.pending_prefix_label(), None);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_pending_find_consumes_non_printable_input() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        assert_eq!(editor.pending_prefix_label(), Some("f".to_string()));

        // Ctrl+F is normally page-down, but should be consumed while waiting for find target.
        editor.handle_key(Key::Ctrl('f'));
        assert_eq!(editor.pending_prefix_label(), Some("f".to_string()));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_g_starts_pending_sequence() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('g'));

        assert_eq!(editor.pending_prefix_label(), Some("g".to_string()));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_gg_moves_to_first_line_and_keeps_column() {
        let mut editor = create_editor_with_content("abcdef\nxy");
        editor.cursor = Cursor::new(1, 1);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_g_dollar_moves_to_current_line_end() {
        let mut editor = create_editor_with_content("abcde");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('$'));

        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_g_zero_moves_to_current_line_start() {
        let mut editor = create_editor_with_content("abcde");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('0'));

        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_gi_consumes_both_and_does_not_enter_insert_mode() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('i'));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_g_colon_consumes_both_and_does_not_enter_command_mode() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char(':'));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_g_slash_consumes_both_and_does_not_enter_search_mode() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('/'));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_escape_clears_pending_sequence() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        assert_eq!(editor.pending_prefix_label(), Some("g".to_string()));

        editor.handle_key(Key::Esc);
        assert_eq!(editor.pending_prefix_label(), None);
    }
}
