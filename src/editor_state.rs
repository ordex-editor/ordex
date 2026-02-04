//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::cursor::Cursor;
use crate::keybindings::{Action, KeyBindings};
use crate::mode::Mode;
use crate::navigation::{find_next_word_start, find_prev_word_start};
use crate::text_buffer::TextBuffer;
use crate::viewport::Viewport;
use std::fs::File;
use std::path::PathBuf;
use termion::event::Key;

/// Editor state holding all components for the editor session
pub struct EditorState {
    /// The text buffer containing file content
    pub buffer: TextBuffer,
    /// Current cursor position
    pub cursor: Cursor,
    /// Current editor mode
    pub mode: Mode,
    /// Viewport for visible portion of document
    pub viewport: Viewport,
    /// Path to the file being edited
    pub file_path: PathBuf,
    /// Status message to display (cleared after one render)
    pub status_message: Option<String>,
    /// Key bindings configuration
    keybindings: KeyBindings,
    /// Flag indicating the editor should quit
    pub should_quit: bool,
}

impl EditorState {
    /// Create a new editor state with an empty buffer
    pub fn new(terminal_height: usize) -> Self {
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
    pub fn load_file(&mut self, path: &str) -> std::io::Result<()> {
        let file = File::open(path)?;
        self.buffer = TextBuffer::from_reader(file)?;
        self.file_path = PathBuf::from(path);
        self.cursor = Cursor::new(0, 0);
        self.viewport.set_first_visible_line(0);
        Ok(())
    }

    /// Handle a key press and update editor state
    pub fn handle_key(&mut self, key: Key) {
        // First check bindings map
        if let Some(action) = self.keybindings.get_action(key, &self.mode) {
            self.execute_action(action);
            return;
        }

        // Handle insertable characters for insert/command/search modes
        if let Some(c) = KeyBindings::is_insertable_char(key) {
            match &mut self.mode {
                Mode::Insert => {
                    self.insert_char(c);
                }
                Mode::Command(input) => {
                    input.push(c);
                }
                Mode::Search(input) => {
                    input.push(c);
                }
                Mode::Normal => {
                    // Unbound key in normal mode - ignore
                }
            }
        }
    }

    fn execute_action(&mut self, action: Action) {
        match action {
            // Navigation
            Action::MoveLeft => self.cursor.move_left(&self.buffer),
            Action::MoveRight => self.cursor.move_right(&self.buffer),
            Action::MoveUp => self.cursor.move_up(&self.buffer),
            Action::MoveDown => self.cursor.move_down(&self.buffer),
            Action::MoveWordForward => self.move_word_forward(),
            Action::MoveWordBackward => self.move_word_backward(),
            Action::PageUp => self.viewport.page_up(&mut self.cursor, &self.buffer),
            Action::PageDown => self.viewport.page_down(&mut self.cursor, &self.buffer),

            // Mode switching
            Action::EnterInsertMode => self.mode = Mode::Insert,
            Action::EnterCommandMode => self.mode = Mode::Command(String::new()),
            Action::EnterSearchMode => self.mode = Mode::Search(String::new()),
            Action::ExitToNormalMode => self.mode = Mode::Normal,

            // Insert mode
            Action::DeleteCharBackward => self.delete_char_backward(),
            Action::InsertNewline => self.insert_newline(),

            // Command/Search mode
            Action::ExecuteCommand => self.execute_command(),
            Action::CancelCommand => self.mode = Mode::Normal,
            Action::DeleteInputChar => self.delete_input_char(),

            // File operations
            Action::SaveFile => self.save_file(),

            // Editor control
            Action::Quit => self.should_quit = true,
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

    fn delete_input_char(&mut self) {
        match &mut self.mode {
            Mode::Command(input) | Mode::Search(input) => {
                input.pop();
            }
            _ => {}
        }
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

                match trimmed {
                    "q" => {
                        self.should_quit = true;
                    }
                    "w" => {
                        self.save_file();
                    }
                    "wq" => {
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

    /// Get the current mode name for display
    pub fn mode_name(&self) -> &str {
        match &self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command(_) => "COMMAND",
            Mode::Search(_) => "SEARCH",
        }
    }

    /// Get the command/search input string for display
    pub fn input_line(&self) -> Option<&str> {
        match &self.mode {
            Mode::Command(input) => Some(input.as_str()),
            Mode::Search(input) => Some(input.as_str()),
            _ => None,
        }
    }

    /// Get the prompt character for command/search mode
    pub fn input_prompt(&self) -> Option<char> {
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
    fn test_input_line_returns_str_slice() {
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::Command("test".to_string());

        let input = editor.input_line();
        assert_eq!(input, Some("test"));
    }
}
