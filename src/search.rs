//! Regex-backed document search helpers.

use crate::text_buffer::TextBuffer;
use regex::Regex;

/// One compiled search query reused across repeated search motions.
#[derive(Debug, Clone)]
pub(crate) struct SearchQuery {
    regex: Regex,
}

/// One matched search span in character coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SearchMatch {
    /// Start of the matched span in character coordinates.
    pub(crate) start: usize,
    /// End of the matched span in character coordinates.
    pub(crate) end: usize,
}

impl SearchQuery {
    /// Compile one search query from user input.
    pub(crate) fn compile(pattern: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(pattern)?;
        Ok(Self { regex })
    }

    /// Find the earliest match whose start is at or after `start_char`.
    pub(crate) fn find_forward(
        &self,
        buffer: &TextBuffer,
        start_char: usize,
    ) -> Option<SearchMatch> {
        // Convert the rope-backed buffer into one contiguous haystack so the
        // regex engine can search it with byte offsets.
        let haystack = collect_buffer_text(buffer);
        let start_byte = buffer.char_to_byte(start_char);
        let found = self.regex.find_at(&haystack, start_byte)?;
        Some(SearchMatch {
            start: buffer.byte_to_char(found.start()),
            end: buffer.byte_to_char(found.end()),
        })
    }

    /// Find the last match whose start lies before `end_char`.
    pub(crate) fn find_backward(
        &self,
        buffer: &TextBuffer,
        end_char: usize,
    ) -> Option<SearchMatch> {
        let end_char = end_char.min(buffer.chars_count());
        if end_char == 0 {
            return None;
        }

        let haystack = collect_buffer_text(buffer);
        let end_byte = buffer.char_to_byte(end_char);
        let total_chars = buffer.chars_count();
        let mut next_start_char = 0;
        let mut last_match = None;

        loop {
            let start_byte = buffer.char_to_byte(next_start_char);
            let Some(found) = self.regex.find_at(&haystack, start_byte) else {
                break;
            };

            // Stop once matches start at or beyond the excluded search boundary.
            if found.start() >= end_byte {
                break;
            }

            let search_match = SearchMatch {
                start: buffer.byte_to_char(found.start()),
                end: buffer.byte_to_char(found.end()),
            };

            // Backward repeats exclude only starts at or beyond the boundary so
            // earlier overlapping matches remain reachable.
            last_match = Some(search_match);

            // Advance by one character from the last match start so overlapping
            // matches stay reachable while the scan still makes progress.
            let next_char = search_match.start.saturating_add(1);
            if next_char > total_chars {
                break;
            }
            next_start_char = next_char;
        }

        last_match
    }
}

/// Collect the full buffer text into one contiguous string for regex matching.
fn collect_buffer_text(buffer: &TextBuffer) -> String {
    buffer.chunks().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Forward regex search should return character-based spans.
    fn test_find_forward_returns_character_span() {
        let buffer = TextBuffer::from_str("aé🙂\nbye");
        let query = SearchQuery::compile("é🙂").expect("compile regex");

        assert_eq!(
            query.find_forward(&buffer, 0),
            Some(SearchMatch { start: 1, end: 3 })
        );
    }

    #[test]
    /// Backward regex search should keep overlapping matches reachable.
    fn test_find_backward_returns_last_overlapping_match() {
        let buffer = TextBuffer::from_str("banana");
        let query = SearchQuery::compile("ana").expect("compile regex");

        assert_eq!(
            query.find_backward(&buffer, buffer.chars_count()),
            Some(SearchMatch { start: 3, end: 6 })
        );
        assert_eq!(
            query.find_backward(&buffer, 3),
            Some(SearchMatch { start: 1, end: 4 })
        );
    }
}
