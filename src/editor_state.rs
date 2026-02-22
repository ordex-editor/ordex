//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::cursor::Cursor;
use crate::keybindings::{Action, KeyBindings};
use crate::mode::Mode;
use crate::navigation::{find_next_word_start, find_prev_word_start, find_word_end};
use crate::text_buffer::TextBuffer;
use crate::viewport::Viewport;
use std::fs::File;
use std::path::PathBuf;
use termion::event::Key;

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

    /// Handle a key press and update editor state
    pub(crate) fn handle_key(&mut self, key: Key) {
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
            Action::MoveToLastLine => self.move_to_last_line(),
            Action::PageUp => self.viewport.page_up(&mut self.cursor, &self.buffer),
            Action::PageDown => self.viewport.page_down(&mut self.cursor, &self.buffer),

            // Mode switching
            Action::EnterInsertMode => self.mode = Mode::Insert,
            Action::EnterCommandMode => self.mode = Mode::Command(String::new()),
            Action::EnterSearchMode => self.mode = Mode::Search(String::new()),
            Action::ExitToNormalMode => self.mode = Mode::Normal,

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
                        self.should_quit = true;
                    }
                    ("w", None) => {
                        self.save_file();
                    }
                    ("w", Some(filename)) | ("write", Some(filename)) => {
                        self.save_file_as(filename);
                    }
                    ("wq", None) => {
                        self.save_file();
                        self.should_quit = true;
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

        // Search from current position
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

    fn save_file(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            self.status_message = Some("No file name".to_string());
            return;
        }

        match File::create(&self.file_path) {
            Ok(mut file) => match self.buffer.write_to(&mut file) {
                Ok(()) => {
                    self.buffer.clear_modified();
                    self.status_message = Some(format!("\"{}\" written", self.file_path.display()));
                }
                Err(e) => {
                    self.status_message = Some(format!("Error writing file: {}", e));
                }
            },
            Err(e) => {
                self.status_message = Some(format!("Error creating file: {}", e));
            }
        }
    }

    fn save_file_as(&mut self, filename: &str) {
        if filename.is_empty() {
            self.status_message = Some("No file name".to_string());
            return;
        }

        let path = PathBuf::from(filename);
        match File::create(&path) {
            Ok(mut file) => match self.buffer.write_to(&mut file) {
                Ok(()) => {
                    self.file_path = path;
                    self.buffer.clear_modified();
                    self.status_message = Some(format!("\"{}\" written", self.file_path.display()));
                }
                Err(e) => {
                    self.status_message = Some(format!("Error writing file: {}", e));
                }
            },
            Err(e) => {
                self.status_message = Some(format!("Error creating file: {}", e));
            }
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_editor_with_content(content: &str) -> EditorState {
        let mut editor = EditorState::new(24);
        editor.buffer = TextBuffer::from_str(content);
        editor
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
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::Command("w /tmp/ordex_test_save_as.txt".to_string());

        editor.handle_key(Key::Char('\n'));

        assert!(
            editor
                .file_path
                .to_str()
                .unwrap()
                .contains("ordex_test_save_as")
        );
        assert!(!editor.buffer.is_modified());
        assert!(editor.status_message.as_ref().unwrap().contains("written"));

        // Cleanup
        let _ = std::fs::remove_file("/tmp/ordex_test_save_as.txt");
    }

    #[test]
    fn test_save_file_as_with_write_command() {
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::Command("write /tmp/ordex_test_write.txt".to_string());

        editor.handle_key(Key::Char('\n'));

        assert!(
            editor
                .file_path
                .to_str()
                .unwrap()
                .contains("ordex_test_write")
        );
        assert!(!editor.buffer.is_modified());

        // Cleanup
        let _ = std::fs::remove_file("/tmp/ordex_test_write.txt");
    }

    #[test]
    fn test_save_file_as_updates_file_path() {
        let mut editor = create_editor_with_content("new file content");
        assert!(editor.file_path.as_os_str().is_empty());

        editor.mode = Mode::Command("w /tmp/ordex_new_file.txt".to_string());
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.file_path.to_str().unwrap(),
            "/tmp/ordex_new_file.txt"
        );

        // Cleanup
        let _ = std::fs::remove_file("/tmp/ordex_new_file.txt");
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
}
