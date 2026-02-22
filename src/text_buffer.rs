//! Text buffer abstraction wrapping ropey::Rope
//!
//! This module provides a high-level interface for text manipulation,
//! abstracting away the underlying rope data structure. This allows for
//! potential future changes to the text storage implementation without
//! affecting the rest of the codebase.

use ropey::{Rope, RopeSlice};
use std::fmt;
use std::io::{self, BufReader, Read, Write};

// We use LF_CR line type which recognizes LF, CR, and CRLF as line breaks
// This is the default feature in ropey 2.0
use ropey::LineType;
const LINE_TYPE: LineType = LineType::LF_CR;

/// A slice of text from the buffer
/// This wraps the underlying rope slice to avoid exposing implementation details
pub(crate) struct TextSlice<'a> {
    slice: RopeSlice<'a>,
}

impl<'a> TextSlice<'a> {
    fn new(slice: RopeSlice<'a>) -> Self {
        Self { slice }
    }

    /// Get the length of the text slice in characters
    #[cfg(test)]
    pub(crate) fn chars_count(&self) -> usize {
        self.slice.len_chars()
    }

    /// Get a character at the specified character index
    #[cfg(test)]
    pub(crate) fn char_at(&self, char_idx: usize) -> Option<char> {
        if char_idx < self.slice.len_chars() {
            // Convert char index to byte index
            let byte_idx = self.slice.char_to_byte_idx(char_idx);
            Some(self.slice.char(byte_idx))
        } else {
            None
        }
    }

    /// Iterate over characters in the text slice
    pub(crate) fn chars(&self) -> impl Iterator<Item = char> + 'a {
        self.slice.chars()
    }
}

impl fmt::Display for TextSlice<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.slice)
    }
}

/// Text buffer managing document content with efficient editing operations
pub(crate) struct TextBuffer {
    rope: Rope,
    modified: bool,
}

impl TextBuffer {
    /// Create an empty text buffer
    pub(crate) fn new() -> Self {
        Self {
            rope: Rope::new(),
            modified: false,
        }
    }

    /// Create a text buffer from a string
    #[cfg(test)]
    pub(crate) fn from_str(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            modified: false,
        }
    }

    /// Create a text buffer by reading from a reader in chunks
    /// This is more efficient for large files than reading the entire content into a string
    pub(crate) fn from_reader<R: Read>(reader: R) -> io::Result<Self> {
        let buf_reader = BufReader::new(reader);
        let rope = Rope::from_reader(buf_reader)?;
        Ok(Self {
            rope,
            modified: false,
        })
    }

    /// Insert text at the given character index
    pub(crate) fn insert(&mut self, char_idx: usize, text: &str) {
        let byte_idx = self.rope.char_to_byte_idx(char_idx);
        self.rope.insert(byte_idx, text);
        self.modified = true;
    }

    /// Remove text in the given character index range [start, end)
    pub(crate) fn remove(&mut self, start_char: usize, end_char: usize) {
        let start_byte = self.rope.char_to_byte_idx(start_char);
        let end_byte = self.rope.char_to_byte_idx(end_char);
        self.rope.remove(start_byte..end_byte);
        self.modified = true;
    }

    /// Get a line's content (0-indexed)
    /// Returns None if line_idx is out of bounds
    pub(crate) fn line(&self, line_idx: usize) -> Option<TextSlice<'_>> {
        if line_idx >= self.rope.len_lines(LINE_TYPE) {
            return None;
        }
        Some(TextSlice::new(self.rope.line(line_idx, LINE_TYPE)))
    }

    /// Get a line's display content (0-indexed), without trailing line breaks.
    ///
    /// This is intended for terminal rendering, where writing raw '\n' or '\r'
    /// would move the cursor and corrupt positioned output.
    pub(crate) fn line_for_display(&self, line_idx: usize) -> Option<String> {
        let mut line = self.line(line_idx)?.to_string();
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        Some(line)
    }

    /// Get the length of a line in characters (0-indexed)
    /// Excludes the newline character
    pub(crate) fn line_len(&self, line_idx: usize) -> usize {
        if line_idx >= self.rope.len_lines(LINE_TYPE) {
            return 0;
        }
        let line = self.rope.line(line_idx, LINE_TYPE);
        let len = line.len_chars();
        // Subtract newline characters if present
        if len > 0 {
            let last_byte = line.char_to_byte_idx(len - 1);
            let last_char = line.char(last_byte);
            if last_char == '\n' || last_char == '\r' {
                len - 1
            } else {
                len
            }
        } else {
            0
        }
    }

    /// Get the total number of lines in the buffer
    pub(crate) fn lines_count(&self) -> usize {
        self.rope.len_lines(LINE_TYPE)
    }

    /// Get the total number of characters in the buffer
    pub(crate) fn chars_count(&self) -> usize {
        self.rope.len_chars()
    }

    /// Convert a character index to a line number
    pub(crate) fn char_to_line(&self, char_idx: usize) -> usize {
        let byte_idx = self.rope.char_to_byte_idx(char_idx);
        self.rope.byte_to_line_idx(byte_idx, LINE_TYPE)
    }

    /// Convert a line number to the character index of the start of that line
    pub(crate) fn line_to_char(&self, line_idx: usize) -> usize {
        let byte_idx = self.rope.line_to_byte_idx(line_idx, LINE_TYPE);
        self.rope.byte_to_char_idx(byte_idx)
    }

    /// Write the buffer contents to a writer, chunk by chunk for efficiency
    /// This is the preferred method for saving files
    pub(crate) fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        for chunk in self.rope.chunks() {
            writer.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }

    /// Convert the buffer to a string (for tests and small buffers)
    /// For saving files, prefer write_to() for better performance
    #[cfg(test)]
    #[expect(clippy::inherent_to_string)]
    pub(crate) fn to_string(&self) -> String {
        self.rope.to_string()
    }

    /// Check if the buffer has been modified
    pub(crate) fn is_modified(&self) -> bool {
        self.modified
    }

    /// Clear the modified flag (e.g., after saving)
    pub(crate) fn clear_modified(&mut self) {
        self.modified = false;
    }

    /// Find the first occurrence of a pattern starting from the given character index
    /// Returns the character index of the match, or None if not found
    pub(crate) fn find(&self, pattern: &str, start_char: usize) -> Option<usize> {
        if pattern.is_empty() {
            return None;
        }

        let total_chars = self.chars_count();
        if start_char >= total_chars {
            return None;
        }

        let pattern_len = pattern.chars().count();

        for idx in start_char..total_chars {
            if idx + pattern_len > total_chars {
                break;
            }

            // Compare pattern chars against buffer chars using iterators
            let matches = pattern
                .chars()
                .zip(idx..)
                .all(|(pc, buf_idx)| self.char_at(buf_idx).is_some_and(|c| c == pc));

            if matches {
                return Some(idx);
            }
        }

        None
    }

    /// Get a character at the specified character index
    pub(crate) fn char_at(&self, char_idx: usize) -> Option<char> {
        if char_idx >= self.chars_count() {
            return None;
        }
        let byte_idx = self.rope.char_to_byte_idx(char_idx);
        Some(self.rope.char(byte_idx))
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer_is_empty() {
        let buffer = TextBuffer::new();
        assert_eq!(buffer.lines_count(), 1); // Empty buffer has 1 line
        assert_eq!(buffer.chars_count(), 0);
        assert!(!buffer.is_modified());
    }

    #[test]
    fn test_from_str() {
        let buffer = TextBuffer::from_str("Hello\nWorld");
        assert_eq!(buffer.lines_count(), 2);
        assert!(!buffer.is_modified());
    }

    #[test]
    fn test_insert_and_modified() {
        let mut buffer = TextBuffer::new();
        buffer.insert(0, "Test");
        assert!(buffer.is_modified());
        assert_eq!(buffer.to_string(), "Test");
    }

    #[test]
    fn test_remove() {
        let mut buffer = TextBuffer::from_str("Hello World");
        buffer.remove(5, 11);
        assert!(buffer.is_modified());
        assert_eq!(buffer.to_string(), "Hello");
    }

    #[test]
    fn test_line_access() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2\nLine 3");
        assert_eq!(buffer.line(0).unwrap().to_string(), "Line 1\n");
        assert_eq!(buffer.line(1).unwrap().to_string(), "Line 2\n");
        assert_eq!(buffer.line(2).unwrap().to_string(), "Line 3");
        assert!(buffer.line(3).is_none());
    }

    #[test]
    fn test_line_for_display_removes_line_breaks() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2\r\nLine 3");
        assert_eq!(buffer.line_for_display(0), Some("Line 1".to_string()));
        assert_eq!(buffer.line_for_display(1), Some("Line 2".to_string()));
        assert_eq!(buffer.line_for_display(2), Some("Line 3".to_string()));
        assert_eq!(buffer.line_for_display(3), None);
    }

    #[test]
    fn test_line_len() {
        let buffer = TextBuffer::from_str("Hello\nWorld");
        assert_eq!(buffer.line_len(0), 5); // "Hello" (newline not counted)
        assert_eq!(buffer.line_len(1), 5); // "World"
    }

    #[test]
    fn test_char_to_line_conversion() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2\nLine 3");
        assert_eq!(buffer.char_to_line(0), 0);
        assert_eq!(buffer.char_to_line(7), 1);
        assert_eq!(buffer.char_to_line(14), 2);
    }

    #[test]
    fn test_line_to_char_conversion() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2\nLine 3");
        assert_eq!(buffer.line_to_char(0), 0);
        assert_eq!(buffer.line_to_char(1), 7);
        assert_eq!(buffer.line_to_char(2), 14);
    }

    #[test]
    fn test_clear_modified() {
        let mut buffer = TextBuffer::new();
        buffer.insert(0, "Test");
        assert!(buffer.is_modified());
        buffer.clear_modified();
        assert!(!buffer.is_modified());
    }

    #[test]
    fn test_write_to() {
        let buffer = TextBuffer::from_str("Hello\nWorld");
        let mut output = Vec::new();
        buffer.write_to(&mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "Hello\nWorld");
    }

    #[test]
    fn test_text_slice() {
        let buffer = TextBuffer::from_str("Hello World");
        let slice = buffer.line(0).unwrap();
        assert_eq!(slice.chars_count(), 11);
        assert_eq!(slice.char_at(0), Some('H'));
        assert_eq!(slice.char_at(6), Some('W'));
        assert_eq!(slice.char_at(20), None);
    }
}
