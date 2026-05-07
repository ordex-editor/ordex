//! Word navigation logic
//!
//! Provides functions for moving the cursor by words, respecting word
//! boundaries defined by whitespace and punctuation characters.

use crate::text_buffer::TextBuffer;

/// Distinguish Vim-style `word` and `WORD` boundary rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordStyle {
    Small,
    Big,
}

/// Return whether a character belongs to one identifier-like word segment.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Return whether a character participates in the requested word style.
pub(crate) fn is_word_style_char(c: char, style: WordStyle) -> bool {
    match style {
        WordStyle::Small => is_word_char(c),
        WordStyle::Big => !c.is_whitespace(),
    }
}

fn is_blank_line(buffer: &TextBuffer, line_idx: usize) -> bool {
    buffer
        .line_for_display(line_idx)
        .is_none_or(|line| line.chars().all(char::is_whitespace))
}

/// Find the inclusive/exclusive span of an "inner word" for `iw`-style operations.
///
/// If the cursor is on non-word content, this prefers the next word to the right,
/// and falls back to the previous word to the left.
#[cfg(test)]
pub(crate) fn find_inner_word_span(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
) -> Option<(usize, usize)> {
    find_inner_word_span_with_style(buffer, cursor_char_idx, WordStyle::Small)
}

/// Find the inclusive/exclusive span of an inner word or WORD using `style`.
///
/// If the cursor is on non-word content, this prefers the next matching run to
/// the right and falls back to the previous one on the left.
pub(crate) fn find_inner_word_span_with_style(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    style: WordStyle,
) -> Option<(usize, usize)> {
    let total = buffer.chars_count();
    // Empty buffer: there is no object to select/delete.
    if total == 0 {
        return None;
    }

    // Clamp to the last valid char so callers can safely pass "cursor at/past EOL".
    let idx = cursor_char_idx.min(total.saturating_sub(1));

    // Fast path: if we're already on a word char, return that whole contiguous word.
    if buffer
        .char_at(idx)
        .is_some_and(|ch| is_word_style_char(ch, style))
    {
        let mut start = idx;
        // Expand left to the first non-word boundary.
        while start > 0
            && buffer
                .char_at(start - 1)
                .is_some_and(|ch| is_word_style_char(ch, style))
        {
            start -= 1;
        }

        let mut end = idx + 1;
        // Expand right to the first non-word boundary (exclusive end).
        while end < total
            && buffer
                .char_at(end)
                .is_some_and(|ch| is_word_style_char(ch, style))
        {
            end += 1;
        }
        return Some((start, end));
    }

    // Vim-like preference for `iw` on non-word chars: pick the next word to the right first.
    // This keeps behavior deterministic when cursor is on whitespace/punctuation.
    let mut right = idx;
    while right < total {
        if buffer
            .char_at(right)
            .is_some_and(|ch| is_word_style_char(ch, style))
        {
            // `right` is already at the first char of that next word.
            let mut end = right + 1;
            while end < total
                && buffer
                    .char_at(end)
                    .is_some_and(|ch| is_word_style_char(ch, style))
            {
                end += 1;
            }
            return Some((right, end));
        }
        right += 1;
    }

    // If nothing exists to the right, fall back to the nearest word on the left.
    // This mirrors "nearest viable object" behavior while still preferring right side first.
    let mut left = idx;
    loop {
        if buffer
            .char_at(left)
            .is_some_and(|ch| is_word_style_char(ch, style))
        {
            let mut start = left;
            // Walk backward to the start of the discovered word.
            while start > 0
                && buffer
                    .char_at(start - 1)
                    .is_some_and(|ch| is_word_style_char(ch, style))
            {
                start -= 1;
            }
            let mut end = left + 1;
            // Walk forward to compute exclusive end for removal slicing.
            while end < total
                && buffer
                    .char_at(end)
                    .is_some_and(|ch| is_word_style_char(ch, style))
            {
                end += 1;
            }
            return Some((start, end));
        }
        if left == 0 {
            break;
        }
        left -= 1;
    }

    None
}

/// Find the inclusive/exclusive span for one "around word" or "around WORD".
///
/// This keeps the core word span and prefers trailing horizontal whitespace so
/// repeated word-object deletions keep later words aligned at the same cursor site.
pub(crate) fn find_around_word_span(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    style: WordStyle,
) -> Option<(usize, usize)> {
    // Start from the core text object so the "around" form only decides which
    // adjacent separator run should travel with that word.
    let (mut start, mut end) = find_inner_word_span_with_style(buffer, cursor_char_idx, style)?;
    let total = buffer.chars_count();

    // Prefer trailing spaces so `daw` keeps consuming the word under the cursor
    // and its separator before falling back to leading whitespace at line ends.
    while end < total {
        match buffer.char_at(end) {
            Some(c) if c.is_whitespace() && c != '\n' => end += 1,
            _ => break,
        }
    }
    if end > start
        && buffer
            .char_at(end.saturating_sub(1))
            .is_some_and(char::is_whitespace)
    {
        // A trailing separator was found, so keep the original word start and
        // include that separator run in the returned around-word span.
        return Some((start, end));
    }

    // No trailing spaces were available, which usually means the word sits at a
    // line end. In that case, borrow leading horizontal whitespace instead so the
    // selection still behaves like an "around" text object.
    while start > 0 {
        match buffer.char_at(start - 1) {
            Some(c) if c.is_whitespace() && c != '\n' => start -= 1,
            _ => break,
        }
    }
    Some((start, end))
}

/// Find the inclusive/exclusive span for the smallest surrounding balanced delimiter pair.
/// The returned span includes both delimiters.
pub(crate) fn find_around_delimiter_span(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    open_delimiter: char,
    close_delimiter: char,
) -> Option<(usize, usize)> {
    let total = buffer.chars_count();
    // Empty buffer has no balanced pair.
    if total == 0 {
        return None;
    }

    // Clamp cursor to a valid char index to avoid boundary edge handling in callers.
    let idx = cursor_char_idx.min(total.saturating_sub(1));
    // Stores the best enclosing range as (open_idx, close_idx_exclusive).
    let mut best: Option<(usize, usize)> = None;

    // Scan candidate open delimiters from cursor-left outward and compute each
    // balanced match so nested pairs choose the smallest enclosure.
    for open in (0..=idx).rev() {
        if buffer.char_at(open) != Some(open_delimiter) {
            continue;
        }

        // Local depth relative to this `open`.
        let mut depth = 0usize;
        let mut close = None;
        for i in open..total {
            match buffer.char_at(i) {
                Some(c) if c == open_delimiter => {
                    // Count nested openings from this candidate outward so the
                    // matching close is the one that brings this local depth to zero.
                    depth += 1;
                }
                Some(c) if c == close_delimiter => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(close) = close else {
            continue;
        };

        // Keep only pairs that actually contain the cursor.
        if open <= idx && idx <= close {
            let candidate = (open, close + 1);
            // Convert the inclusive close position into the exclusive end index
            // used by selection ranges and other buffer-slicing helpers.
            match best {
                Some((best_open, best_close)) => {
                    // Prefer the shortest enclosing span => smallest.
                    if close - open < best_close - best_open {
                        best = Some(candidate);
                    }
                }
                None => {
                    best = Some(candidate);
                }
            }
        }
    }

    best
}

/// Find the inclusive/exclusive span inside the smallest surrounding delimiter pair.
pub(crate) fn find_inner_delimiter_span(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    open_delimiter: char,
    close_delimiter: char,
) -> Option<(usize, usize)> {
    let (start, end) =
        find_around_delimiter_span(buffer, cursor_char_idx, open_delimiter, close_delimiter)?;
    let inner_start = start.saturating_add(1);
    let inner_end = end.saturating_sub(1);
    if inner_start >= inner_end {
        return None;
    }
    Some((inner_start, inner_end))
}

/// Find the start of the next word from the given position
/// Returns the character index of the next word start, or the end of the buffer
pub(crate) fn find_next_word_start(buffer: &TextBuffer, char_idx: usize) -> usize {
    find_next_word_start_with_style(buffer, char_idx, WordStyle::Small)
}

/// Find the start of the next word using the requested Vim word style.
pub(crate) fn find_next_word_start_with_style(
    buffer: &TextBuffer,
    char_idx: usize,
    style: WordStyle,
) -> usize {
    let total_chars = buffer.chars_count();
    if char_idx >= total_chars {
        return total_chars;
    }

    let mut idx = char_idx;

    // First consume the current word-like run when the cursor already starts
    // inside it so motions advance to the next boundary instead of staying put.
    let current_char = buffer.char_at(idx);
    let in_word = current_char.is_some_and(|ch| is_word_style_char(ch, style));

    if in_word {
        // Skip rest of current word
        while idx < total_chars {
            match buffer.char_at(idx) {
                Some(c) if is_word_style_char(c, style) => idx += 1,
                _ => break,
            }
        }
    }

    // Then skip separators until the next word-like run begins. Small-word
    // motions stop at newlines, while big-word motions treat punctuation as part
    // of the same WORD and only stop on whitespace.
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if !is_word_style_char(c, style) && c != '\n' => idx += 1,
            Some('\n') => {
                // Stop at newline, move past it, and let the final whitespace pass
                // land on the first word character of the following line.
                idx += 1;
                break;
            }
            _ => break,
        }
    }

    // Finally skip any remaining horizontal whitespace at the new site.
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if c.is_whitespace() && c != '\n' => idx += 1,
            _ => break,
        }
    }

    idx
}

/// Find the end of the current/next word from the given position
/// Returns the character index of the word end (last char of word)
pub(crate) fn find_word_end(buffer: &TextBuffer, char_idx: usize) -> usize {
    find_word_end_with_style(buffer, char_idx, WordStyle::Small)
}

/// Find the end of the current or next word using the requested Vim word style.
pub(crate) fn find_word_end_with_style(
    buffer: &TextBuffer,
    char_idx: usize,
    style: WordStyle,
) -> usize {
    let total_chars = buffer.chars_count();
    if char_idx >= total_chars {
        return total_chars.saturating_sub(1);
    }

    let mut idx = char_idx;

    // `e`/`E` begin their search one character to the right before scanning for
    // the next end boundary, which matches Vim's "move to end of current/next word".
    if idx + 1 < total_chars {
        idx += 1;
    }

    // Skip separators until the cursor lands on the next word-like run.
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if !is_word_style_char(c, style) && c != '\n' => idx += 1,
            _ => break,
        }
    }

    // Then walk to the inclusive end of that run.
    while idx + 1 < total_chars {
        match buffer.char_at(idx + 1) {
            Some(c) if is_word_style_char(c, style) => idx += 1,
            _ => break,
        }
    }

    idx
}

/// Find the start of the previous word from the given position
/// Returns the character index of the previous word start, or 0
pub(crate) fn find_prev_word_start(buffer: &TextBuffer, char_idx: usize) -> usize {
    find_prev_word_start_with_style(buffer, char_idx, WordStyle::Small)
}

/// Find the start of the previous word using the requested Vim word style.
pub(crate) fn find_prev_word_start_with_style(
    buffer: &TextBuffer,
    char_idx: usize,
    style: WordStyle,
) -> usize {
    if char_idx == 0 {
        return 0;
    }

    let mut idx = char_idx;

    // Move back one position to start
    idx = idx.saturating_sub(1);

    // Skip separators backwards until we land inside the previous word-like run.
    while idx > 0 {
        match buffer.char_at(idx) {
            Some(c) if !is_word_style_char(c, style) => idx -= 1,
            _ => break,
        }
    }

    // Then walk to the start boundary of that run.
    while idx > 0 {
        match buffer.char_at(idx - 1) {
            Some(c) if is_word_style_char(c, style) => idx -= 1,
            _ => break,
        }
    }

    idx
}

/// Find the end of the previous word from the given position.
/// Returns the character index of the previous word end, or 0.
pub(crate) fn find_prev_word_end(buffer: &TextBuffer, char_idx: usize) -> usize {
    find_prev_word_end_with_style(buffer, char_idx, WordStyle::Small)
}

/// Find the end of the previous word using the requested Vim word style.
pub(crate) fn find_prev_word_end_with_style(
    buffer: &TextBuffer,
    char_idx: usize,
    style: WordStyle,
) -> usize {
    if char_idx == 0 {
        return 0;
    }

    let mut idx = char_idx.saturating_sub(1);

    // Step left through the current word run first so `ge` and `gE` always
    // target the preceding word end rather than the word containing the cursor.
    if buffer
        .char_at(idx)
        .is_some_and(|ch| is_word_style_char(ch, style))
    {
        while idx > 0
            && buffer
                .char_at(idx - 1)
                .is_some_and(|ch| is_word_style_char(ch, style))
        {
            idx -= 1;
        }
        if idx == 0 {
            return 0;
        }
        idx -= 1;
    }

    // Skip separators backward until the scan lands on the previous word-like
    // run. That landing point is already the inclusive word end we want.
    while idx > 0 {
        if buffer
            .char_at(idx)
            .is_some_and(|ch| is_word_style_char(ch, style))
        {
            break;
        }
        idx -= 1;
    }

    idx
}

/// Find the first line index of the next paragraph.
///
/// Paragraphs are separated by one or more blank lines.
pub(crate) fn find_next_paragraph_line(buffer: &TextBuffer, current_line: usize) -> usize {
    let total_lines = buffer.lines_count();
    if total_lines == 0 {
        return 0;
    }

    // Start searching strictly below the current line.
    let mut line = current_line.saturating_add(1);
    // If we're already at/after the last line, keep the cursor clamped there.
    if line >= total_lines {
        return total_lines.saturating_sub(1);
    }

    // First blank line encountered is the next paragraph separator target.
    while line < total_lines {
        if is_blank_line(buffer, line) {
            return line;
        }
        line += 1;
    }

    // No separator below: clamp to the last line.
    total_lines.saturating_sub(1)
}

/// Find the first line index of the previous paragraph.
///
/// Paragraphs are separated by one or more blank lines.
pub(crate) fn find_prev_paragraph_line(buffer: &TextBuffer, current_line: usize) -> usize {
    let total_lines = buffer.lines_count();
    if total_lines == 0 {
        return 0;
    }

    // Start searching strictly above the current line.
    let mut line = current_line.saturating_sub(1);
    loop {
        // First blank line encountered is the previous paragraph separator target.
        if is_blank_line(buffer, line) {
            return line;
        }
        if line == 0 {
            break;
        }
        // Walk up until a separator line is found.
        line -= 1;
    }

    // No separator above: clamp to the first line.
    0
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
    /// Previous-word-end lookup should land on the prior word from the next word start.
    fn test_find_prev_word_end_from_next_word_start() {
        let buffer = TextBuffer::from_str("hello world");

        assert_eq!(find_prev_word_end(&buffer, 6), 4);
    }

    #[test]
    /// Previous-word-end lookup should skip back beyond the current word run.
    fn test_find_prev_word_end_from_middle_of_word() {
        let buffer = TextBuffer::from_str("hello world");

        assert_eq!(find_prev_word_end(&buffer, 8), 4);
    }

    #[test]
    /// Big-word previous-end lookup should treat punctuation as part of one WORD.
    fn test_find_prev_word_end_with_big_word_style() {
        let buffer = TextBuffer::from_str("one two-three");

        assert_eq!(
            find_prev_word_end_with_style(&buffer, 13, WordStyle::Big),
            2
        );
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

    #[test]
    fn test_find_word_end_simple() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'h', should go to 'o' (end of hello)
        assert_eq!(find_word_end(&buffer, 0), 4);
    }

    #[test]
    fn test_find_word_end_from_middle() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'e', should go to 'o' (end of hello)
        assert_eq!(find_word_end(&buffer, 1), 4);
    }

    #[test]
    fn test_find_word_end_at_word_end() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'o' (end of hello), should go to 'd' (end of world)
        assert_eq!(find_word_end(&buffer, 4), 10);
    }

    #[test]
    fn test_find_word_end_at_last_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'w', should go to 'd'
        assert_eq!(find_word_end(&buffer, 6), 10);
    }

    #[test]
    fn test_find_word_end_at_end() {
        let buffer = TextBuffer::from_str("hello");
        // At end of buffer, stay at last char
        assert_eq!(find_word_end(&buffer, 4), 4);
    }

    #[test]
    fn test_find_inner_word_span_on_word_char() {
        let buffer = TextBuffer::from_str("alpha beta");
        assert_eq!(find_inner_word_span(&buffer, 2), Some((0, 5)));
    }

    #[test]
    fn test_find_inner_word_span_from_whitespace_picks_next_word() {
        let buffer = TextBuffer::from_str("alpha beta");
        assert_eq!(find_inner_word_span(&buffer, 5), Some((6, 10)));
    }

    #[test]
    fn test_find_inner_word_span_none_when_no_word() {
        let buffer = TextBuffer::from_str("   ");
        assert_eq!(find_inner_word_span(&buffer, 0), None);
    }

    #[test]
    fn test_find_around_paren_span_smallest_surrounding() {
        let buffer = TextBuffer::from_str("x(a(b)c)y");
        // cursor on `b` should pick "(b)".
        assert_eq!(
            find_around_delimiter_span(&buffer, 4, '(', ')'),
            Some((3, 6))
        );
    }

    #[test]
    fn test_find_around_paren_span_none_when_not_enclosed() {
        let buffer = TextBuffer::from_str("abc def");
        assert_eq!(find_around_delimiter_span(&buffer, 2, '(', ')'), None);
    }

    #[test]
    fn test_find_next_paragraph_line_skips_separator() {
        let buffer = TextBuffer::from_str("p1a\np1b\n\np2\n");
        assert_eq!(find_next_paragraph_line(&buffer, 0), 2);
    }

    #[test]
    fn test_find_next_paragraph_line_from_blank_line() {
        let buffer = TextBuffer::from_str("p1\n\n\np2\n");
        assert_eq!(find_next_paragraph_line(&buffer, 1), 2);
    }

    #[test]
    fn test_find_prev_paragraph_line_skips_separator() {
        let buffer = TextBuffer::from_str("p1\n\np2a\np2b\n");
        assert_eq!(find_prev_paragraph_line(&buffer, 3), 1);
    }

    #[test]
    fn test_find_prev_paragraph_line_from_blank_line() {
        let buffer = TextBuffer::from_str("p1\n\n\np2\n");
        assert_eq!(find_prev_paragraph_line(&buffer, 2), 1);
    }
}
