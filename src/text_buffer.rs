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

/// A slice of text from the buffer.
///
/// This wraps the underlying rope slice to avoid exposing implementation details
/// while still letting callers borrow rope-backed content without allocating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextSlice<'a> {
    slice: RopeSlice<'a>,
}

impl<'a> TextSlice<'a> {
    /// Wrap one rope slice.
    fn new(slice: RopeSlice<'a>) -> Self {
        Self { slice }
    }

    /// Return the length of the text slice in characters.
    pub(crate) fn chars_count(&self) -> usize {
        self.slice.len_chars()
    }

    /// Return this slice without any trailing line break characters.
    pub(crate) fn trim_trailing_line_breaks(self) -> Self {
        let mut end = self.slice.len();

        // Logical lines can end in `\n`, `\r`, or `\r\n`, so trim all trailing
        // line-break bytes before exposing the display slice. `\n` and `\r`
        // are single-byte ASCII, so trimming at the byte level is sound here.
        while end > 0 {
            let last_byte = self.slice.byte(end - 1);
            if last_byte != b'\n' && last_byte != b'\r' {
                break;
            }
            end -= 1;
        }

        Self::new(self.slice.slice(..end))
    }

    /// Get a character at the specified character index.
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

    /// Iterate over characters in the text slice.
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
#[derive(Debug, Clone)]
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

    /// Return whether the buffer ends with a line-break character.
    ///
    /// Returns `true` when the last stored character is `\n` or `\r`, and
    /// `false` when the buffer is empty or ends with ordinary text.
    fn has_trailing_line_break(&self) -> bool {
        let Some(last_char) = self.char_at(self.chars_count().saturating_sub(1)) else {
            return false;
        };
        matches!(last_char, '\n' | '\r')
    }

    /// Return the logical line count exposed to editor features.
    ///
    /// Returns Ropey's raw line count for ordinary content and one fewer line
    /// when Ropey materializes a trailing sentinel line after a final line
    /// break. The result is always at least one line so an empty buffer still
    /// behaves like one editable line.
    fn logical_lines_count(&self) -> usize {
        let raw_lines = self.rope.len_lines(LINE_TYPE);
        if raw_lines > 1 && self.has_trailing_line_break() {
            raw_lines - 1
        } else {
            raw_lines
        }
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

    /// Copy text in the given character index range into an owned string.
    pub(crate) fn slice_string(&self, start_char: usize, end_char: usize) -> String {
        let end_char = end_char.min(self.chars_count());
        if start_char >= end_char {
            return String::new();
        }
        (start_char..end_char)
            .filter_map(|char_idx| self.char_at(char_idx))
            .collect()
    }

    /// Return whether the character range starts with the given prefix.
    pub(crate) fn rope_slice_starts_with(
        &self,
        start_char: usize,
        end_char: usize,
        prefix: &str,
    ) -> bool {
        let end_char = end_char.min(self.chars_count());
        if start_char > end_char {
            return false;
        }

        // Compare against the rope-backed range directly so callers can test
        // delimiters without copying the covered text into an owned string or
        // restarting a fresh index lookup for each character.
        let mut slice_chars = self.rope.slice(start_char..end_char).chars();
        for expected in prefix.chars() {
            if slice_chars.next() != Some(expected) {
                return false;
            }
        }

        true
    }

    /// Get a line's content (0-indexed)
    /// Returns None if line_idx is out of bounds
    pub(crate) fn line(&self, line_idx: usize) -> Option<TextSlice<'_>> {
        if line_idx >= self.logical_lines_count() {
            return None;
        }
        Some(TextSlice::new(self.rope.line(line_idx, LINE_TYPE)))
    }

    /// Get a line's display content (0-indexed), without trailing line breaks.
    ///
    /// This is intended for terminal rendering, where writing raw '\n' or '\r'
    /// would move the cursor and corrupt positioned output.
    pub(crate) fn line_for_display(&self, line_idx: usize) -> Option<TextSlice<'_>> {
        self.line(line_idx)
            .map(TextSlice::trim_trailing_line_breaks)
    }

    /// Get one owned display line for callers that still require contiguous text.
    pub(crate) fn line_for_display_string(&self, line_idx: usize) -> Option<String> {
        let mut line = self.line(line_idx)?.to_string();

        // Some parser paths still require contiguous `&str` input, so keep this
        // helper for them while render and `%` can borrow `TextSlice` directly.
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }

        Some(line)
    }

    /// Get the length of a line in characters (0-indexed)
    /// Excludes the newline character
    pub(crate) fn line_len(&self, line_idx: usize) -> usize {
        if line_idx >= self.logical_lines_count() {
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
        self.logical_lines_count()
    }

    /// Get the total number of characters in the buffer
    pub(crate) fn chars_count(&self) -> usize {
        self.rope.len_chars()
    }

    /// Convert a character index to a line number
    pub(crate) fn char_to_line(&self, char_idx: usize) -> usize {
        let max_line = self.logical_lines_count().saturating_sub(1);
        let clamped_char = char_idx.min(self.chars_count());

        // Ropey maps EOF after a trailing newline onto its sentinel line. Vim
        // keeps EOF on the final logical line instead, so clamp that one case.
        if clamped_char == self.chars_count() && self.has_trailing_line_break() {
            return max_line;
        }

        let byte_idx = self.rope.char_to_byte_idx(clamped_char);
        self.rope
            .byte_to_line_idx(byte_idx, LINE_TYPE)
            .min(max_line)
    }

    /// Convert a line number to the character index of the start of that line
    pub(crate) fn line_to_char(&self, line_idx: usize) -> usize {
        if line_idx >= self.logical_lines_count() {
            return self.chars_count();
        }

        let byte_idx = self.rope.line_to_byte_idx(line_idx, LINE_TYPE);
        self.rope.byte_to_char_idx(byte_idx)
    }

    /// Convert a character index to a byte index within the buffer.
    pub(crate) fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte_idx(char_idx.min(self.chars_count()))
    }

    /// Convert a byte index to a character index within the buffer.
    pub(crate) fn byte_to_char(&self, byte_idx: usize) -> usize {
        let max_byte = self.rope.char_to_byte_idx(self.chars_count());
        self.rope.byte_to_char_idx(byte_idx.min(max_byte))
    }

    /// Return the total number of UTF-8 bytes stored in the buffer.
    pub(crate) fn bytes_count(&self) -> usize {
        self.rope.len()
    }

    /// Borrow the full rope slice for allocation-free buffer traversals.
    pub(crate) fn rope_slice(&self) -> RopeSlice<'_> {
        self.rope.slice(..)
    }

    /// Write the buffer contents to a writer, chunk by chunk for efficiency
    /// This is the preferred method for saving files
    pub(crate) fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        for chunk in self.rope.chunks() {
            writer.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }

    /// Write the buffer contents using the save-file newline policy.
    pub(crate) fn write_to_for_save<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.write_to(writer)?;
        if !self.has_trailing_line_break() {
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Iterate over contiguous UTF-8 chunks of the underlying rope.
    pub(crate) fn chunks(&self) -> impl Iterator<Item = &str> + '_ {
        self.rope.chunks()
    }

    /// Clone the underlying rope so background tasks can snapshot text cheaply.
    pub(crate) fn clone_rope(&self) -> Rope {
        self.rope.clone()
    }

    /// Clone the buffer using the save-file newline policy.
    pub(crate) fn clone_rope_for_save(&self) -> Rope {
        let mut rope = self.rope.clone();
        if !self.has_trailing_line_break() {
            rope.insert(rope.len(), "\n");
        }
        rope
    }

    /// Normalize the buffer to match the save-file trailing-newline policy.
    pub(crate) fn normalize_after_save(&mut self) {
        if self.has_trailing_line_break() {
            return;
        }
        let was_modified = self.modified;
        self.rope.insert(self.rope.len(), "\n");
        self.modified = was_modified;
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

    /// Override the modified flag to match higher-level editor history state.
    pub(crate) fn set_modified(&mut self, modified: bool) {
        self.modified = modified;
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
    fn test_trailing_newline_does_not_add_a_logical_line() {
        let buffer = TextBuffer::from_str("alpha\nbeta\n");

        assert_eq!(buffer.lines_count(), 2);
        assert_eq!(buffer.line(0).unwrap().to_string(), "alpha\n");
        assert_eq!(buffer.line(1).unwrap().to_string(), "beta\n");
        assert!(buffer.line(2).is_none());
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
    /// Regression test for extracting one owned substring by character range.
    fn test_slice_string() {
        let buffer = TextBuffer::from_str("Hello\nWorld");

        assert_eq!(buffer.slice_string(0, 5), "Hello");
        assert_eq!(buffer.slice_string(5, 6), "\n");
        assert_eq!(buffer.slice_string(6, 11), "World");
        assert_eq!(buffer.slice_string(20, 25), "");
    }

    #[test]
    /// Prefix checks should read directly from the rope-backed character range.
    fn test_rope_slice_starts_with() {
        let buffer = TextBuffer::from_str("Hello\nWorld");

        assert!(buffer.rope_slice_starts_with(0, 11, "Hello"));
        assert!(buffer.rope_slice_starts_with(6, 11, "World"));
        assert!(!buffer.rope_slice_starts_with(6, 10, "World"));
        assert!(!buffer.rope_slice_starts_with(0, 11, "World"));
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
        assert_eq!(
            buffer.line_for_display(0).map(|line| line.to_string()),
            Some("Line 1".to_string())
        );
        assert_eq!(
            buffer.line_for_display(1).map(|line| line.to_string()),
            Some("Line 2".to_string())
        );
        assert_eq!(
            buffer.line_for_display(2).map(|line| line.to_string()),
            Some("Line 3".to_string())
        );
        assert!(buffer.line_for_display(3).is_none());
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
    fn test_char_to_line_maps_trailing_newline_eof_to_last_logical_line() {
        let buffer = TextBuffer::from_str("alpha\nbeta\n");

        assert_eq!(buffer.char_to_line(buffer.chars_count()), 1);
    }

    #[test]
    fn test_line_to_char_conversion() {
        let buffer = TextBuffer::from_str("Line 1\nLine 2\nLine 3");
        assert_eq!(buffer.line_to_char(0), 0);
        assert_eq!(buffer.line_to_char(1), 7);
        assert_eq!(buffer.line_to_char(2), 14);
    }

    #[test]
    fn test_line_to_char_returns_eof_for_past_last_logical_line() {
        let buffer = TextBuffer::from_str("alpha\nbeta\n");

        assert_eq!(buffer.line_to_char(2), buffer.chars_count());
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
    fn test_write_to_for_save_appends_trailing_newline_when_missing() {
        let buffer = TextBuffer::from_str("Hello\nWorld");
        let mut output = Vec::new();
        buffer.write_to_for_save(&mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "Hello\nWorld\n");
    }

    #[test]
    fn test_clone_rope_for_save_appends_trailing_newline_when_missing() {
        let buffer = TextBuffer::from_str("Hello");

        assert_eq!(buffer.clone_rope_for_save().to_string(), "Hello\n");
    }

    #[test]
    /// Save normalization should append a trailing newline without dirtying the buffer.
    fn test_normalize_after_save_appends_trailing_newline_without_dirtying_buffer() {
        let mut buffer = TextBuffer::from_str("Hello");

        buffer.normalize_after_save();

        assert_eq!(buffer.to_string(), "Hello\n");
        assert!(!buffer.is_modified());
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

    #[test]
    /// Regression test for byte and character coordinate conversions.
    fn test_byte_char_conversions() {
        let buffer = TextBuffer::from_str("aé🙂");
        assert_eq!(buffer.char_to_byte(0), 0);
        assert_eq!(buffer.char_to_byte(1), 1);
        assert_eq!(buffer.char_to_byte(2), 3);
        assert_eq!(buffer.char_to_byte(3), 7);
        assert_eq!(buffer.byte_to_char(0), 0);
        assert_eq!(buffer.byte_to_char(1), 1);
        assert_eq!(buffer.byte_to_char(3), 2);
        assert_eq!(buffer.byte_to_char(7), 3);
    }
}
