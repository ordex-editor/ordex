//! Cursor position and movement logic
//!
//! The cursor tracks the current editing position within the document.
//! It maintains a "desired column" to preserve horizontal position during
//! vertical movement through lines of varying lengths.

use crate::text_buffer::TextBuffer;

/// Cursor representing the current editing position
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    line: usize,
    column: usize,
    desired_column: usize,
}

impl Cursor {
    /// Create a new cursor at the specified position
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            desired_column: column,
        }
    }

    /// Get the current line (0-indexed)
    pub fn line(&self) -> usize {
        self.line
    }

    /// Get the current column (0-indexed)
    pub fn column(&self) -> usize {
        self.column
    }

    /// Set the column position
    pub fn set_column(&mut self, column: usize) {
        self.column = column;
        self.desired_column = column;
    }

    /// Move cursor left by one character
    pub fn move_left(&mut self, buffer: &TextBuffer) {
        if self.column > 0 {
            self.column -= 1;
            self.desired_column = self.column;
        } else if self.line > 0 {
            // Move to end of previous line
            self.line -= 1;
            self.column = buffer.line_len(self.line);
            self.desired_column = self.column;
        }
    }

    /// Move cursor right by one character
    pub fn move_right(&mut self, buffer: &TextBuffer) {
        let line_len = buffer.line_len(self.line);
        if self.column < line_len {
            self.column += 1;
            self.desired_column = self.column;
        } else if self.line + 1 < buffer.lines_count() {
            // Move to start of next line
            self.line += 1;
            self.column = 0;
            self.desired_column = self.column;
        }
    }

    /// Move cursor up by one line
    pub fn move_up(&mut self, buffer: &TextBuffer) {
        if self.line > 0 {
            self.line -= 1;
            self.clamp_to_line(buffer);
        }
    }

    /// Move cursor down by one line
    pub fn move_down(&mut self, buffer: &TextBuffer) {
        if self.line + 1 < buffer.lines_count() {
            self.line += 1;
            self.clamp_to_line(buffer);
        }
    }

    /// Move cursor left by one character (normal mode semantics, no line wrap)
    pub fn move_left_normal(&mut self) {
        if self.column > 0 {
            self.column -= 1;
            self.desired_column = self.column;
        }
    }

    /// Move cursor right by one character (normal mode semantics, no line wrap)
    pub fn move_right_normal(&mut self, buffer: &TextBuffer) {
        let line_len = buffer.line_len(self.line);
        if self.column + 1 < line_len {
            self.column += 1;
            self.desired_column = self.column;
        }
    }

    /// Move cursor up by one line (normal mode semantics)
    pub fn move_up_normal(&mut self, buffer: &TextBuffer) {
        if self.line > 0 {
            self.line -= 1;
            self.clamp_to_line_normal(buffer);
        }
    }

    /// Move cursor down by one line (normal mode semantics)
    pub fn move_down_normal(&mut self, buffer: &TextBuffer) {
        if self.line + 1 < buffer.lines_count() {
            self.line += 1;
            self.clamp_to_line_normal(buffer);
        }
    }

    /// Move cursor to the start of the current line
    pub fn move_to_line_start(&mut self) {
        self.column = 0;
        self.desired_column = 0;
    }

    /// Move cursor to the end of the current line (last character, for normal mode)
    pub fn move_to_line_end(&mut self, buffer: &TextBuffer) {
        let len = buffer.line_len(self.line);
        self.column = len.saturating_sub(1);
        self.desired_column = self.column;
    }

    /// Move cursor past the end of the current line (for insert mode)
    pub fn move_past_line_end(&mut self, buffer: &TextBuffer) {
        let len = buffer.line_len(self.line);
        self.column = len;
        self.desired_column = self.column;
    }

    /// Clamp the cursor column to the current line length
    /// This preserves the desired_column for vertical movement
    pub fn clamp_to_line(&mut self, buffer: &TextBuffer) {
        let line_len = buffer.line_len(self.line);
        self.column = self.desired_column.min(line_len);
    }

    /// Clamp to the current line's valid normal-mode range.
    /// In normal mode, non-empty lines allow [0, len - 1] and empty lines allow 0.
    pub fn clamp_to_line_normal(&mut self, buffer: &TextBuffer) {
        let line_len = buffer.line_len(self.line);
        let max_col = line_len.saturating_sub(1);
        self.column = self.desired_column.min(max_col);
    }

    /// Convert cursor position to a character index in the buffer
    pub fn to_char_index(&self, buffer: &TextBuffer) -> usize {
        buffer.line_to_char(self.line) + self.column
    }

    /// Create a cursor from a character index in the buffer
    pub fn from_char_index(buffer: &TextBuffer, char_idx: usize) -> Self {
        let line = buffer.char_to_line(char_idx);
        let line_start = buffer.line_to_char(line);
        let column = char_idx - line_start;
        Self {
            line,
            column,
            desired_column: column,
        }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cursor() {
        let cursor = Cursor::new(5, 10);
        assert_eq!(cursor.line(), 5);
        assert_eq!(cursor.column(), 10);
    }

    #[test]
    fn test_move_left_in_line() {
        let buffer = TextBuffer::from_str("Hello World");
        let mut cursor = Cursor::new(0, 5);
        cursor.move_left(&buffer);
        assert_eq!(cursor.column(), 4);
    }

    #[test]
    fn test_move_left_at_line_start() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2");
        let mut cursor = Cursor::new(1, 0);
        cursor.move_left(&buffer);
        assert_eq!(cursor.line(), 0);
        assert_eq!(cursor.column(), 6); // End of "Line 1"
    }

    #[test]
    fn test_move_right_in_line() {
        let buffer = TextBuffer::from_str("Hello World");
        let mut cursor = Cursor::new(0, 5);
        cursor.move_right(&buffer);
        assert_eq!(cursor.column(), 6);
    }

    #[test]
    fn test_move_right_at_line_end() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2");
        let mut cursor = Cursor::new(0, 6);
        cursor.move_right(&buffer);
        assert_eq!(cursor.line(), 1);
        assert_eq!(cursor.column(), 0);
    }

    #[test]
    fn test_move_up() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2");
        let mut cursor = Cursor::new(1, 3);
        cursor.move_up(&buffer);
        assert_eq!(cursor.line(), 0);
        assert_eq!(cursor.column(), 3);
    }

    #[test]
    fn test_move_down() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2");
        let mut cursor = Cursor::new(0, 3);
        cursor.move_down(&buffer);
        assert_eq!(cursor.line(), 1);
        assert_eq!(cursor.column(), 3);
    }

    #[test]
    fn test_desired_column_preservation() {
        let buffer = TextBuffer::from_str("Long line here\nHi\nAnother long line");
        let mut cursor = Cursor::new(0, 10);
        cursor.move_down(&buffer); // Move to short line "Hi"
        assert_eq!(cursor.column(), 2); // Clamped to line length
        cursor.move_down(&buffer); // Move to long line
        assert_eq!(cursor.column(), 10); // Restored to desired column
    }

    #[test]
    fn test_move_to_line_start() {
        let mut cursor = Cursor::new(0, 5);
        cursor.move_to_line_start();
        assert_eq!(cursor.column(), 0);
    }

    #[test]
    fn test_move_to_line_end() {
        let buffer = TextBuffer::from_str("Hello World");
        let mut cursor = Cursor::new(0, 0);
        cursor.move_to_line_end(&buffer);
        assert_eq!(cursor.column(), 10); // Last char of "Hello World" (0-indexed)
    }

    #[test]
    fn test_to_char_index() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2");
        let cursor = Cursor::new(1, 3);
        assert_eq!(cursor.to_char_index(&buffer), 10); // 7 + 3
    }

    #[test]
    fn test_from_char_index() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2");
        let cursor = Cursor::from_char_index(&buffer, 10);
        assert_eq!(cursor.line(), 1);
        assert_eq!(cursor.column(), 3);
    }
}
