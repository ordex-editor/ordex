//! Viewport management for scrolling and visible region tracking
//!
//! The Viewport manages which portion of the document is visible on screen
//! and handles scrolling to keep the cursor in view.

use crate::cursor::Cursor;
use crate::text_buffer::TextBuffer;
use std::ops::Range;

/// Viewport managing the visible region of the document
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    first_visible_line: usize,
    height: usize,
    scroll_margin: usize,
}

impl Viewport {
    /// Create a new viewport with the given height
    /// scroll_margin defaults to 3 lines
    pub fn new(height: usize) -> Self {
        Self {
            first_visible_line: 0,
            height,
            scroll_margin: 3,
        }
    }

    /// Get the range of visible lines [first, last)
    pub fn visible_range(&self) -> Range<usize> {
        self.first_visible_line..self.first_visible_line + self.height
    }

    /// Get the first visible line
    pub fn first_visible_line(&self) -> usize {
        self.first_visible_line
    }

    /// Set the first visible line
    pub fn set_first_visible_line(&mut self, line: usize) {
        self.first_visible_line = line;
    }

    /// Ensure the cursor is visible, scrolling if necessary
    pub fn ensure_cursor_visible(&mut self, cursor: &Cursor, buffer: &TextBuffer) {
        let cursor_line = cursor.line();
        let total_lines = buffer.lines_count();

        // Check if we need to scroll up
        if cursor_line < self.first_visible_line + self.scroll_margin {
            self.first_visible_line = cursor_line.saturating_sub(self.scroll_margin);
        }

        // Check if we need to scroll down
        let last_visible_line = self.first_visible_line + self.height;
        if cursor_line + self.scroll_margin + 1 > last_visible_line {
            self.first_visible_line = (cursor_line + self.scroll_margin + 1)
                .saturating_sub(self.height)
                .min(total_lines.saturating_sub(self.height));
        }
    }

    /// Scroll the viewport up by the specified number of lines
    pub fn scroll_up(&mut self, lines: usize) {
        self.first_visible_line = self.first_visible_line.saturating_sub(lines);
    }

    /// Scroll the viewport down by the specified number of lines
    pub fn scroll_down(&mut self, lines: usize, buffer: &TextBuffer) {
        let max_first_line = buffer.lines_count().saturating_sub(1);
        self.first_visible_line = (self.first_visible_line + lines).min(max_first_line);
    }

    /// Page up: move viewport and cursor up by (height - 1) lines
    pub fn page_up(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        let page_size = self.height.saturating_sub(1).max(1);

        // Move cursor up by page size
        // TODO: implement without a loop.
        for _ in 0..page_size {
            if cursor.line() == 0 {
                break;
            }
            cursor.move_up(buffer);
        }

        // Adjust viewport to keep cursor visible
        self.ensure_cursor_visible(cursor, buffer);
    }

    /// Page down: move viewport and cursor down by (height - 1) lines
    pub fn page_down(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        let page_size = self.height.saturating_sub(1).max(1);

        // Move cursor down by page size
        // TODO: implement without a loop.
        for _ in 0..page_size {
            if cursor.line() + 1 >= buffer.lines_count() {
                break;
            }
            cursor.move_down(buffer);
        }

        // Adjust viewport to keep cursor visible
        self.ensure_cursor_visible(cursor, buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_buffer(num_lines: usize) -> TextBuffer {
        let lines: Vec<String> = (1..=num_lines).map(|i| format!("Line {}", i)).collect();
        TextBuffer::from_str(&lines.join("\n"))
    }

    #[test]
    fn test_new_viewport() {
        let viewport = Viewport::new(20);
        assert_eq!(viewport.first_visible_line(), 0);
        assert_eq!(viewport.visible_range(), 0..20);
    }

    #[test]
    fn test_ensure_cursor_visible_no_scroll_needed() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let cursor = Cursor::new(10, 0);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 0); // No scroll needed
    }

    #[test]
    fn test_ensure_cursor_visible_scroll_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let cursor = Cursor::new(50, 0);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        // Cursor at line 50, margin 3, height 20
        // Should scroll so cursor is not too close to bottom
        assert!(viewport.first_visible_line() > 0);
    }

    #[test]
    fn test_ensure_cursor_visible_scroll_up() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        viewport.first_visible_line = 50;
        let cursor = Cursor::new(45, 0);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        // Cursor at line 45, should scroll up to keep margin
        assert!(viewport.first_visible_line() < 50);
    }

    #[test]
    fn test_scroll_up() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        viewport.first_visible_line = 10;

        viewport.scroll_up(5);
        assert_eq!(viewport.first_visible_line(), 5);

        viewport.scroll_up(10); // Should not go below 0
        assert_eq!(viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_scroll_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);

        viewport.scroll_down(10, &buffer);
        assert_eq!(viewport.first_visible_line(), 10);
    }

    #[test]
    fn test_page_up() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(50, 0);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        let initial_line = cursor.line();

        viewport.page_up(&mut cursor, &buffer);
        // Cursor should have moved up by approximately height - 1
        assert!(cursor.line() < initial_line);
        assert!(cursor.line() + 19 <= initial_line + 1); // Moved by ~19 lines
    }

    #[test]
    fn test_page_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(10, 0);

        let initial_line = cursor.line();
        viewport.page_down(&mut cursor, &buffer);

        // Cursor should have moved down by approximately height - 1
        assert!(cursor.line() > initial_line);
        assert!(cursor.line() >= initial_line + 19); // Moved by ~19 lines
    }

    #[test]
    fn test_page_up_at_start() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(5, 0);

        viewport.page_up(&mut cursor, &buffer);
        // Should move to line 0 and not go negative
        assert_eq!(cursor.line(), 0);
        assert_eq!(viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_page_down_at_end() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(95, 0);

        viewport.page_down(&mut cursor, &buffer);
        // Should move to last line and not go beyond
        assert_eq!(cursor.line(), 99);
    }
}
