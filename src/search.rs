//! Regex-backed document search helpers.

use crate::text_buffer::TextBuffer;
use regex_cursor::engines::meta::Regex;
use regex_cursor::{Cursor, Input as RegexInput};
use ropey::{RopeSlice, iter::Chunks};

/// Track whether the chunk iterator currently points at the start or end edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkPosition {
    /// The iterator is positioned just before the current chunk.
    Start,
    /// The iterator is positioned just after the current chunk.
    End,
}

/// Present Ordex's Ropey 2 rope slices as regex-cursor haystacks.
///
/// This follows the same chunk-cursor shape as regex-cursor's RopeyCursor, but
/// targets Ordex's Ropey 2 API surface instead of the crate's built-in Ropey 1 adapter.
/// TODO: Remove this adapter once regex-cursor ships native Ropey 2 support.
#[derive(Clone)]
pub(crate) struct BufferCursor<'a> {
    iter: Chunks<'a>,
    current: &'a [u8],
    position: ChunkPosition,
    len: usize,
    offset: usize,
}

impl<'a> BufferCursor<'a> {
    /// Create one cursor positioned at the chunk containing `offset`.
    pub(crate) fn at(slice: RopeSlice<'a>, offset: usize) -> Self {
        let slice_len = slice.len();
        let (iter, offset) = slice.chunks_at(offset.min(slice_len));
        if offset == slice_len {
            // End-positioned searches need to seed the cursor from the previous
            // chunk so regex-cursor still has visible bytes available.
            let mut cursor = Self {
                iter,
                current: &[],
                position: ChunkPosition::Start,
                len: slice_len,
                offset,
            };
            cursor.backtrack();
            cursor
        } else {
            // Mid-buffer searches begin at the chunk that owns the requested
            // byte offset so the engine can start near the search boundary.
            let mut cursor = Self {
                iter,
                current: &[],
                position: ChunkPosition::End,
                len: slice_len,
                offset,
            };
            cursor.advance();
            cursor
        }
    }
}

/// Build one regex-cursor input over a byte range in the active text buffer.
pub(crate) fn regex_input_for_byte_range(
    buffer: &TextBuffer,
    start_byte: usize,
    end_byte: usize,
) -> RegexInput<BufferCursor<'_>> {
    let cursor = BufferCursor::at(buffer.rope_slice(), start_byte);
    RegexInput::new(cursor).range(start_byte..end_byte)
}

impl Cursor for BufferCursor<'_> {
    /// Return the current contiguous UTF-8 chunk.
    fn chunk(&self) -> &[u8] {
        self.current
    }

    /// Advance to the next non-empty chunk.
    fn advance(&mut self) -> bool {
        if self.position == ChunkPosition::Start {
            self.iter.next();
            self.position = ChunkPosition::End;
        }
        for next in self.iter.by_ref() {
            // Ropey can surface empty sentinel chunks around boundaries, so skip
            // them to preserve regex-cursor's "never return empty unless empty"
            // contract for non-empty haystacks.
            if next.is_empty() {
                continue;
            }
            self.offset += self.current.len();
            self.current = next.as_bytes();
            return true;
        }
        false
    }

    /// Move to the previous non-empty chunk.
    fn backtrack(&mut self) -> bool {
        if self.position == ChunkPosition::End {
            self.iter.prev();
            self.position = ChunkPosition::Start;
        }
        while let Some(previous) = self.iter.prev() {
            // Regex searches only care about real text chunks, so the cursor
            // keeps stepping until it lands on one with visible bytes.
            if previous.is_empty() {
                continue;
            }
            self.offset -= previous.len();
            self.current = previous.as_bytes();
            return true;
        }
        false
    }

    /// Report that chunk boundaries always respect UTF-8 code point boundaries.
    fn utf8_aware(&self) -> bool {
        true
    }

    /// Return the haystack's total byte length.
    fn total_bytes(&self) -> Option<usize> {
        Some(self.len)
    }

    /// Return the current chunk's byte offset from the start of the haystack.
    fn offset(&self) -> usize {
        self.offset
    }
}

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
    pub(crate) fn compile(pattern: &str) -> Result<Self, String> {
        compile_pattern_regex(pattern).map(|regex| Self { regex })
    }

    /// Find the earliest match whose start is at or after `start_char`.
    pub(crate) fn find_forward(
        &self,
        buffer: &TextBuffer,
        start_char: usize,
    ) -> Option<SearchMatch> {
        let start_byte = buffer.char_to_byte(start_char);
        let found = self.find_in_byte_range(buffer, start_byte, buffer.bytes_count())?;
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
        let total_chars = buffer.chars_count();
        let end_char = end_char.min(total_chars);
        let end_byte = buffer.char_to_byte(end_char);
        if end_byte == 0 {
            return None;
        }

        let mut next_start_char = 0;
        let mut last_match = None;

        loop {
            let start_byte = buffer.char_to_byte(next_start_char);
            let Some(found) = self.find_in_byte_range(buffer, start_byte, buffer.bytes_count())
            else {
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

    /// Collect every non-empty match whose start lies inside `start_char..end_char`.
    pub(crate) fn find_all_in_char_range(
        &self,
        buffer: &TextBuffer,
        start_char: usize,
        end_char: usize,
    ) -> Vec<SearchMatch> {
        let total_chars = buffer.chars_count();
        let start_char = start_char.min(total_chars);
        let end_char = end_char.min(total_chars);
        if start_char >= end_char {
            return Vec::new();
        }

        let end_byte = buffer.char_to_byte(end_char);
        let mut next_start_char = start_char;
        let mut matches = Vec::new();

        loop {
            let start_byte = buffer.char_to_byte(next_start_char);
            let Some(found) = self.find_in_byte_range(buffer, start_byte, buffer.bytes_count())
            else {
                break;
            };

            // Visible highlighting only needs matches that begin inside the
            // bounded range, so stop once the next match starts beyond it.
            if found.start() >= end_byte {
                break;
            }

            let search_match = SearchMatch {
                start: buffer.byte_to_char(found.start()),
                end: buffer.byte_to_char(found.end()),
            };
            if search_match.start < search_match.end {
                matches.push(search_match);
            }

            // Advance from the greater of the requested start and actual match
            // start so overlapping matches remain reachable and zero-length
            // matches still make forward progress.
            let next_char = next_start_char.max(search_match.start).saturating_add(1);
            if next_char > total_chars {
                break;
            }
            next_start_char = next_char;
        }

        matches
    }

    /// Search one byte range without materializing the full buffer into a string.
    fn find_in_byte_range(
        &self,
        buffer: &TextBuffer,
        start_byte: usize,
        end_byte: usize,
    ) -> Option<regex_cursor::regex_automata::Match> {
        if start_byte > end_byte || end_byte > buffer.bytes_count() {
            return None;
        }

        // The cursor starts near the search boundary for throughput, while the
        // explicit byte range still lets the engine inspect surrounding context
        // for assertions like word boundaries.
        self.regex
            .find(regex_input_for_byte_range(buffer, start_byte, end_byte))
    }
}

/// Compile one user-facing regex pattern and normalize editor-specific errors.
pub(crate) fn compile_pattern_regex(pattern: &str) -> Result<Regex, String> {
    // Search treats `^` and `$` as per-line anchors, so compile every pattern
    // in multiline mode instead of file-boundary-only mode.
    let multiline_pattern = format!("(?m:{pattern})");
    Regex::new(&multiline_pattern).map_err(|error| error.to_string())
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
    /// Non-zero starts should still preserve word-boundary context.
    fn test_find_forward_preserves_boundary_context() {
        let buffer = TextBuffer::from_str("xx foo xx");
        let query = SearchQuery::compile(r"\bfoo\b").expect("compile regex");

        assert_eq!(
            query.find_forward(&buffer, 3),
            Some(SearchMatch { start: 3, end: 6 })
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

    #[test]
    /// Visible-range scans should collect every overlapping non-empty match start.
    fn test_find_all_in_char_range_collects_overlapping_matches() {
        let buffer = TextBuffer::from_str("banana");
        let query = SearchQuery::compile("ana").expect("compile regex");

        assert_eq!(
            query.find_all_in_char_range(&buffer, 0, buffer.chars_count()),
            vec![
                SearchMatch { start: 1, end: 4 },
                SearchMatch { start: 3, end: 6 }
            ]
        );
    }

    #[test]
    /// Visible-range scans should stop before matches that start after the range.
    fn test_find_all_in_char_range_bounds_match_starts() {
        let buffer = TextBuffer::from_str("alpha\nbeta\ngamma");
        let query = SearchQuery::compile("a").expect("compile regex");

        assert_eq!(
            query.find_all_in_char_range(&buffer, 0, 5),
            vec![
                SearchMatch { start: 0, end: 1 },
                SearchMatch { start: 4, end: 5 }
            ]
        );
    }

    /// Plain search patterns should match across logical line breaks with `\n`.
    #[test]
    fn test_find_forward_matches_newline_escape() {
        let buffer = TextBuffer::from_str("one\nalpha\nbeta\nthree\n");
        let query = SearchQuery::compile(r"alpha\nbeta").expect("compile regex");

        assert_eq!(
            query.find_forward(&buffer, 0),
            Some(SearchMatch { start: 4, end: 14 })
        );
    }

    /// Escaped backslashes should preserve a literal `\n` search pattern.
    #[test]
    fn test_find_forward_preserves_literal_backslash_n() {
        let buffer = TextBuffer::from_str(r"literal \n marker");
        let query = SearchQuery::compile(r"\\n").expect("compile regex");

        assert_eq!(
            query.find_forward(&buffer, 0),
            Some(SearchMatch { start: 8, end: 10 })
        );
    }

    #[test]
    /// `^` should match the start of each logical line.
    fn test_find_forward_line_start_anchor_matches_each_line() {
        let buffer = TextBuffer::from_str("alpha\nbeta\ngamma");
        let query = SearchQuery::compile("^").expect("compile regex");

        // Forward scans should surface each line start anchor in order.
        assert_eq!(
            query.find_forward(&buffer, 0),
            Some(SearchMatch { start: 0, end: 0 })
        );
        assert_eq!(
            query.find_forward(&buffer, 1),
            Some(SearchMatch { start: 6, end: 6 })
        );
        assert_eq!(
            query.find_forward(&buffer, 7),
            Some(SearchMatch { start: 11, end: 11 })
        );
    }

    #[test]
    /// `$` should match the end of each logical line.
    fn test_find_forward_line_end_anchor_matches_each_line() {
        let buffer = TextBuffer::from_str("ab\ncde\nf");
        let query = SearchQuery::compile("$").expect("compile regex");

        // Forward scans should surface each line end anchor in order.
        assert_eq!(
            query.find_forward(&buffer, 0),
            Some(SearchMatch { start: 2, end: 2 })
        );
        assert_eq!(
            query.find_forward(&buffer, 3),
            Some(SearchMatch { start: 6, end: 6 })
        );
        assert_eq!(
            query.find_forward(&buffer, 7),
            Some(SearchMatch { start: 8, end: 8 })
        );
    }

    #[test]
    /// Line anchors should include empty trailing lines after a final newline.
    fn test_find_forward_line_anchor_includes_trailing_empty_line() {
        let buffer = TextBuffer::from_str("a\n");
        let query = SearchQuery::compile("^").expect("compile regex");

        // The trailing newline creates an empty final line with its own start anchor.
        assert_eq!(
            query.find_forward(&buffer, 1),
            Some(SearchMatch { start: 2, end: 2 })
        );
    }

    #[test]
    /// Backward search should step across per-line `^` matches instead of file-only matches.
    fn test_find_backward_line_start_anchor_steps_across_lines() {
        let buffer = TextBuffer::from_str("alpha\nbeta\ngamma");
        let query = SearchQuery::compile("^").expect("compile regex");

        // Backward scans should return earlier per-line anchors as the boundary moves.
        assert_eq!(
            query.find_backward(&buffer, buffer.chars_count()),
            Some(SearchMatch { start: 11, end: 11 })
        );
        assert_eq!(
            query.find_backward(&buffer, 11),
            Some(SearchMatch { start: 6, end: 6 })
        );
    }

    #[test]
    /// Anchored patterns with surrounding text should match at per-line boundaries.
    fn test_find_forward_anchored_patterns_with_text() {
        let buffer = TextBuffer::from_str("foo first\nfirst\nlast bar\nlast");
        let line_start = SearchQuery::compile("^first").expect("compile regex");
        let line_end = SearchQuery::compile("last$").expect("compile regex");

        // `^first` should match only at the start of the second line.
        assert_eq!(
            line_start.find_forward(&buffer, 0),
            Some(SearchMatch { start: 10, end: 15 })
        );
        // `last$` should match only at the end of the fourth line.
        assert_eq!(
            line_end.find_forward(&buffer, 0),
            Some(SearchMatch { start: 25, end: 29 })
        );
    }
}
