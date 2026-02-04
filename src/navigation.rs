//! Word navigation logic
//!
//! Provides functions for moving the cursor by words, respecting word
//! boundaries defined by whitespace and punctuation characters.

use crate::text_buffer::TextBuffer;

/// Check if a character is a word character (alphanumeric or underscore)
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Find the start of the next word from the given position
/// Returns the character index of the next word start, or the end of the buffer
pub fn find_next_word_start(buffer: &TextBuffer, char_idx: usize) -> usize {
    let total_chars = buffer.chars_count();
    if char_idx >= total_chars {
        return total_chars;
    }

    let mut idx = char_idx;

    // Get the character at current position
    let current_char = buffer.char_at(idx);
    let in_word = current_char.is_some_and(is_word_char);

    if in_word {
        // Skip rest of current word
        while idx < total_chars {
            match buffer.char_at(idx) {
                Some(c) if is_word_char(c) => idx += 1,
                _ => break,
            }
        }
    }

    // Skip whitespace and punctuation
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if !is_word_char(c) && c != '\n' => idx += 1,
            Some('\n') => {
                // Stop at newline, move past it
                idx += 1;
                break;
            }
            _ => break,
        }
    }

    // Handle case where we landed on whitespace at start of line
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if c.is_whitespace() && c != '\n' => idx += 1,
            _ => break,
        }
    }

    idx
}

/// Find the start of the previous word from the given position
/// Returns the character index of the previous word start, or 0
pub fn find_prev_word_start(buffer: &TextBuffer, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }

    let mut idx = char_idx;

    // Move back one position to start
    idx = idx.saturating_sub(1);

    // Skip whitespace and punctuation backwards
    while idx > 0 {
        match buffer.char_at(idx) {
            Some(c) if !is_word_char(c) => idx -= 1,
            _ => break,
        }
    }

    // If we're at a word char, skip to the beginning of the word
    while idx > 0 {
        match buffer.char_at(idx - 1) {
            Some(c) if is_word_char(c) => idx -= 1,
            _ => break,
        }
    }

    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_next_word_start_simple() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'h', should go to 'w'
        assert_eq!(find_next_word_start(&buffer, 0), 6);
    }

    #[test]
    fn test_find_next_word_start_from_middle_of_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'e', should go to 'w'
        assert_eq!(find_next_word_start(&buffer, 1), 6);
    }

    #[test]
    fn test_find_next_word_start_at_last_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'w', should go to end
        assert_eq!(find_next_word_start(&buffer, 6), 11);
    }

    #[test]
    fn test_find_next_word_start_at_end() {
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(find_next_word_start(&buffer, 5), 5);
    }

    #[test]
    fn test_find_next_word_start_with_newline() {
        let buffer = TextBuffer::from_str("hello\nworld");
        // From 'h', should stop at newline boundary, then 'w'
        assert_eq!(find_next_word_start(&buffer, 0), 6);
    }

    #[test]
    fn test_find_prev_word_start_simple() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'w', should go to 'h'
        assert_eq!(find_prev_word_start(&buffer, 6), 0);
    }

    #[test]
    fn test_find_prev_word_start_from_middle_of_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'o' in world, should go to 'w'
        assert_eq!(find_prev_word_start(&buffer, 8), 6);
    }

    #[test]
    fn test_find_prev_word_start_from_end() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from end, should go to 'w'
        assert_eq!(find_prev_word_start(&buffer, 11), 6);
    }

    #[test]
    fn test_find_prev_word_start_at_beginning() {
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(find_prev_word_start(&buffer, 0), 0);
    }

    #[test]
    fn test_find_prev_word_start_from_first_char() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'e', should go to 'h'
        assert_eq!(find_prev_word_start(&buffer, 1), 0);
    }
}
