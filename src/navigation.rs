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

/// Distinguish the segment families that form one Vim "word" for navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordSegmentKind {
    Keyword,
    NonBlankPunctuation,
    NonBlank,
}

/// Return whether a character belongs to one identifier-like word segment.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Return the segment classification for one character under `style`.
///
/// Returns `Some(kind)` when the character is part of a word segment for the
/// requested style, and `None` when the character is whitespace.
fn word_segment_kind(c: char, style: WordStyle) -> Option<WordSegmentKind> {
    if c.is_whitespace() {
        return None;
    }
    Some(match style {
        WordStyle::Small if is_word_char(c) => WordSegmentKind::Keyword,
        WordStyle::Small => WordSegmentKind::NonBlankPunctuation,
        WordStyle::Big => WordSegmentKind::NonBlank,
    })
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

    // Fast path: if the cursor already sits on one word segment class, expand
    // over only that class so small-word motions split keyword and punctuation.
    let cursor_kind = buffer
        .char_at(idx)
        .and_then(|ch| word_segment_kind(ch, style));
    if let Some(cursor_kind) = cursor_kind {
        let mut start = idx;
        // Expand left to the first character that changes segment class.
        while start > 0
            && buffer
                .char_at(start - 1)
                .and_then(|ch| word_segment_kind(ch, style))
                .is_some_and(|kind| kind == cursor_kind)
        {
            start -= 1;
        }

        let mut end = idx + 1;
        // Expand right to the first character that changes segment class.
        while end < total
            && buffer
                .char_at(end)
                .and_then(|ch| word_segment_kind(ch, style))
                .is_some_and(|kind| kind == cursor_kind)
        {
            end += 1;
        }
        return Some((start, end));
    }

    // Vim-like preference for `iw` on non-word chars: pick the next word to the right first.
    // This keeps behavior deterministic when cursor is on whitespace/punctuation.
    let mut right = idx;
    while right < total {
        if let Some(right_kind) = buffer
            .char_at(right)
            .and_then(|ch| word_segment_kind(ch, style))
        {
            // `right` is already at the first char of that next word.
            let mut end = right + 1;
            while end < total
                && buffer
                    .char_at(end)
                    .and_then(|ch| word_segment_kind(ch, style))
                    .is_some_and(|kind| kind == right_kind)
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
        if let Some(left_kind) = buffer
            .char_at(left)
            .and_then(|ch| word_segment_kind(ch, style))
        {
            let mut start = left;
            // Walk backward to the start of the discovered word.
            while start > 0
                && buffer
                    .char_at(start - 1)
                    .and_then(|ch| word_segment_kind(ch, style))
                    .is_some_and(|kind| kind == left_kind)
            {
                start -= 1;
            }
            let mut end = left + 1;
            // Walk forward to compute exclusive end for removal slicing.
            while end < total
                && buffer
                    .char_at(end)
                    .and_then(|ch| word_segment_kind(ch, style))
                    .is_some_and(|kind| kind == left_kind)
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

    // First consume the current segment run when the cursor already starts
    // inside it so `w` can stop at the next word-class boundary.
    if let Some(current_kind) = buffer
        .char_at(idx)
        .and_then(|ch| word_segment_kind(ch, style))
    {
        while idx < total_chars
            && buffer
                .char_at(idx)
                .and_then(|ch| word_segment_kind(ch, style))
                .is_some_and(|kind| kind == current_kind)
        {
            idx += 1;
        }
    }

    // Then skip separators until the next segment run begins.
    // Small words and WORDs both use whitespace as a separator between words.
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if c.is_whitespace() && c != '\n' => idx += 1,
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

    // Skip separators until the cursor lands on the next segment run.
    while idx < total_chars {
        match buffer.char_at(idx) {
            Some(c) if word_segment_kind(c, style).is_none() && c != '\n' => idx += 1,
            _ => break,
        }
    }

    // Then walk to the inclusive end of the landed segment class.
    let Some(target_kind) = buffer
        .char_at(idx)
        .and_then(|ch| word_segment_kind(ch, style))
    else {
        return idx;
    };
    while idx + 1 < total_chars {
        match buffer.char_at(idx + 1) {
            Some(c) if word_segment_kind(c, style).is_some_and(|kind| kind == target_kind) => {
                idx += 1
            }
            _ => break,
        }
    }

    idx
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

    // Skip separators backwards until we land inside the previous segment run.
    while idx > 0 {
        match buffer.char_at(idx) {
            Some(c) if word_segment_kind(c, style).is_none() => idx -= 1,
            _ => break,
        }
    }

    // Then walk to the start boundary of that segment class.
    let Some(target_kind) = buffer
        .char_at(idx)
        .and_then(|ch| word_segment_kind(ch, style))
    else {
        return idx;
    };
    while idx > 0 {
        match buffer.char_at(idx - 1) {
            Some(c) if word_segment_kind(c, style).is_some_and(|kind| kind == target_kind) => {
                idx -= 1
            }
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
    let cursor_kind = if char_idx < buffer.chars_count() {
        buffer
            .char_at(char_idx)
            .and_then(|ch| word_segment_kind(ch, style))
    } else {
        buffer
            .char_at(idx)
            .and_then(|ch| word_segment_kind(ch, style))
    };

    // If the cursor sits inside one segment class, skip that class first so
    // `ge`/`gE` land on the previous word end. When the cursor is already at a
    // word start, the character to the left is the desired prior word end.
    if let Some(cursor_kind) = cursor_kind
        && buffer
            .char_at(idx)
            .and_then(|ch| word_segment_kind(ch, style))
            .is_some_and(|kind| kind == cursor_kind)
    {
        while idx > 0
            && buffer
                .char_at(idx - 1)
                .and_then(|ch| word_segment_kind(ch, style))
                .is_some_and(|kind| kind == cursor_kind)
        {
            idx -= 1;
        }
        if idx == 0 {
            return 0;
        }
        idx -= 1;
    }

    // Skip separators backward until the scan lands on a segment character.
    // That landing point is the inclusive word end we need to return.
    while idx > 0 {
        if buffer
            .char_at(idx)
            .and_then(|ch| word_segment_kind(ch, style))
            .is_some()
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

/// Return whether the character at `idx` in `buffer` is preceded by an odd
/// number of backslashes, making it an escaped character.
///
/// Returns `true` when the character is escaped (preceded by an odd count of
/// `\`), and `false` when it is unescaped (zero or even count of `\`).
fn is_escaped_at(buffer: &TextBuffer, idx: usize) -> bool {
    // Count consecutive backslashes immediately before `idx`.
    let mut backslash_count = 0usize;
    let mut scan = idx;
    while scan > 0 {
        scan -= 1;
        if buffer.char_at(scan) == Some('\\') {
            backslash_count += 1;
        } else {
            break;
        }
    }
    // An odd number means the character is escaped; even (including zero) means unescaped.
    !backslash_count.is_multiple_of(2)
}

/// Find the inclusive/exclusive span for the smallest surrounding quote pair.
///
/// Unlike bracket delimiters, a quote character is its own open and close, so
/// depth-based nesting does not apply.
///
/// Search strategy (applied in order, returning on first success):
///
/// 1. **Parity check from buffer start**: count all unescaped occurrences of
///    `quote` from the buffer start up to (but not including) `cursor_char_idx`.
///    - Odd count: cursor is inside an open string. The most-recently-seen quote
///      is the opener (may be on a previous line). Scan forward from it to find
///      the closer.
///    - Even count and cursor is ON a quote: cursor is an opener. Scan right for
///      the closer.
///    - Even count and cursor is NOT on a quote: cursor is outside all strings.
///      Scan forward on the same line only for the nearest complete pair to the
///      right. No subsequent-line scanning.
///
/// Backslash-escaped quote characters (e.g. `\"`) are skipped.
/// The returned span is `(open, close + 1)` — both quotes included.
pub(crate) fn find_around_quote_span(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let total = buffer.chars_count();
    if total == 0 {
        return None;
    }

    let idx = cursor_char_idx.min(total.saturating_sub(1));

    // Count unescaped quote characters from the buffer start up to (not
    // including) the cursor position. This determines parity.
    let mut open_idx: Option<usize> = None;
    let mut count = 0usize;
    for i in 0..idx {
        if buffer.char_at(i) == Some(quote) && !is_escaped_at(buffer, i) {
            count += 1;
            // Every odd-numbered quote (1st, 3rd, …) is an opener.
            if !count.is_multiple_of(2) {
                open_idx = Some(i);
            }
        }
    }

    if !count.is_multiple_of(2) {
        // Odd parity: cursor is inside a string whose opener is `open_idx`.
        // The opener may be on a previous line. Scan forward from it for the
        // closer anywhere in the buffer.
        let open = open_idx?;
        let close = find_unescaped_quote_after_with_limit(buffer, open + 1, total, quote)?;
        return Some((open, close + 1));
    }

    // Even parity: cursor is outside all strings (or on an opener quote).
    if buffer.char_at(idx) == Some(quote) && !is_escaped_at(buffer, idx) {
        // Cursor is on a quote and parity before it is even → this is an opener.
        // Find the matching closer anywhere to the right.
        let close = find_unescaped_quote_after_with_limit(buffer, idx + 1, total, quote)?;
        return Some((idx, close + 1));
    }

    // Cursor is outside all strings and not on a quote: scan right on the same
    // line only for the nearest complete pair. Subsequent lines are not scanned.
    let line_idx = buffer.char_to_line(idx);
    let line_end = if line_idx + 1 < buffer.lines_count() {
        buffer.line_to_char(line_idx + 1)
    } else {
        total
    };
    find_next_quote_span_after_with_limit(buffer, idx + 1, line_end, quote)
}

/// Find the inclusive/exclusive span inside the smallest surrounding quote pair.
///
/// Returns `None` when no enclosing pair exists or when the inner span is empty
/// (i.e. the string literal contains no characters between the quotes).
pub(crate) fn find_inner_quote_span(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let (start, end) = find_around_quote_span(buffer, cursor_char_idx, quote)?;
    let inner_start = start.saturating_add(1);
    let inner_end = end.saturating_sub(1);
    // An empty string literal (open immediately followed by close) has no inner
    // content to select, so the operation is a no-op.
    if inner_start >= inner_end {
        return None;
    }
    Some((inner_start, inner_end))
}

/// Scan forward from `start` (inclusive) up to `limit` (exclusive) and return
/// the index of the first unescaped occurrence of `quote`, or `None`.
fn find_unescaped_quote_after_with_limit(
    buffer: &TextBuffer,
    start: usize,
    limit: usize,
    quote: char,
) -> Option<usize> {
    let mut idx = start;
    while idx < limit {
        if buffer.char_at(idx) == Some(quote) && !is_escaped_at(buffer, idx) {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

/// Scan forward from `start` and return the `(open, close + 1)` span of the
/// next complete unescaped quote pair within `[start, limit)`, or `None`.
fn find_next_quote_span_after_with_limit(
    buffer: &TextBuffer,
    start: usize,
    limit: usize,
    quote: char,
) -> Option<(usize, usize)> {
    // Find the opening quote of the next string within range.
    let open = find_unescaped_quote_after_with_limit(buffer, start, limit, quote)?;
    // Find the closing quote for that opening within range.
    let close = find_unescaped_quote_after_with_limit(buffer, open + 1, limit, quote)?;
    Some((open, close + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_next_word_start_simple() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'h', should go to 'w'
        assert_eq!(
            find_next_word_start_with_style(&buffer, 0, WordStyle::Small),
            6
        );
    }

    #[test]
    fn test_find_next_word_start_from_middle_of_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'e', should go to 'w'
        assert_eq!(
            find_next_word_start_with_style(&buffer, 1, WordStyle::Small),
            6
        );
    }

    #[test]
    fn test_find_next_word_start_at_last_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'w', should go to end
        assert_eq!(
            find_next_word_start_with_style(&buffer, 6, WordStyle::Small),
            11
        );
    }

    #[test]
    fn test_find_next_word_start_at_end() {
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(
            find_next_word_start_with_style(&buffer, 5, WordStyle::Small),
            5
        );
    }

    #[test]
    fn test_find_next_word_start_with_newline() {
        let buffer = TextBuffer::from_str("hello\nworld");
        // From 'h', should stop at newline boundary, then 'w'
        assert_eq!(
            find_next_word_start_with_style(&buffer, 0, WordStyle::Small),
            6
        );
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
        assert_eq!(
            find_prev_word_start_with_style(&buffer, 6, WordStyle::Small),
            0
        );
    }

    #[test]
    fn test_find_prev_word_start_from_middle_of_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'o' in world, should go to 'w'
        assert_eq!(
            find_prev_word_start_with_style(&buffer, 8, WordStyle::Small),
            6
        );
    }

    #[test]
    fn test_find_prev_word_start_from_end() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from end, should go to 'w'
        assert_eq!(
            find_prev_word_start_with_style(&buffer, 11, WordStyle::Small),
            6
        );
    }

    #[test]
    fn test_find_prev_word_start_at_beginning() {
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(
            find_prev_word_start_with_style(&buffer, 0, WordStyle::Small),
            0
        );
    }

    #[test]
    fn test_find_prev_word_start_from_first_char() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'e', should go to 'h'
        assert_eq!(
            find_prev_word_start_with_style(&buffer, 1, WordStyle::Small),
            0
        );
    }

    #[test]
    fn test_find_word_end_simple() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'h', should go to 'o' (end of hello)
        assert_eq!(find_word_end_with_style(&buffer, 0, WordStyle::Small), 4);
    }

    #[test]
    fn test_find_word_end_from_middle() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'e', should go to 'o' (end of hello)
        assert_eq!(find_word_end_with_style(&buffer, 1, WordStyle::Small), 4);
    }

    #[test]
    fn test_find_word_end_at_word_end() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'o' (end of hello), should go to 'd' (end of world)
        assert_eq!(find_word_end_with_style(&buffer, 4, WordStyle::Small), 10);
    }

    #[test]
    fn test_find_word_end_at_last_word() {
        let buffer = TextBuffer::from_str("hello world");
        // Starting from 'w', should go to 'd'
        assert_eq!(find_word_end_with_style(&buffer, 6, WordStyle::Small), 10);
    }

    #[test]
    fn test_find_word_end_at_end() {
        let buffer = TextBuffer::from_str("hello");
        // At end of buffer, stay at last char
        assert_eq!(find_word_end_with_style(&buffer, 4, WordStyle::Small), 4);
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
    /// Small-word `iw` should treat contiguous punctuation as one word segment.
    fn test_find_inner_word_span_on_punctuation_run() {
        let buffer = TextBuffer::from_str("//! Cool");
        assert_eq!(find_inner_word_span(&buffer, 0), Some((0, 3)));
    }

    #[test]
    /// Small-word `w` should stop at punctuation-word boundaries without skipping them.
    fn test_find_next_word_start_stops_at_punctuation_word() {
        let buffer = TextBuffer::from_str("foo-bar baz");
        assert_eq!(
            find_next_word_start_with_style(&buffer, 0, WordStyle::Small),
            3
        );
    }

    #[test]
    /// Small-word `b` should land on the previous punctuation-word start.
    fn test_find_prev_word_start_lands_on_punctuation_word() {
        let buffer = TextBuffer::from_str("foo-bar baz");
        assert_eq!(
            find_prev_word_start_with_style(&buffer, 4, WordStyle::Small),
            3
        );
    }

    #[test]
    /// Small-word `e` from punctuation should stop at the punctuation-word end.
    fn test_find_word_end_on_doc_comment_punctuation_word() {
        let buffer = TextBuffer::from_str("//! Cool");
        assert_eq!(find_word_end_with_style(&buffer, 0, WordStyle::Small), 2);
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

    // Quote span unit tests

    #[test]
    /// Basic around-quote span: cursor inside a double-quoted string.
    fn test_find_around_quote_span_basic() {
        // "hello" — cursor on 'l' at index 2.
        let buffer = TextBuffer::from_str("\"hello\"");
        assert_eq!(find_around_quote_span(&buffer, 2, '"'), Some((0, 7)));
    }

    #[test]
    /// Basic inner-quote span: cursor inside a double-quoted string.
    fn test_find_inner_quote_span_basic() {
        // "hello" — cursor on 'l' at index 2; inner span excludes the quotes.
        let buffer = TextBuffer::from_str("\"hello\"");
        assert_eq!(find_inner_quote_span(&buffer, 2, '"'), Some((1, 6)));
    }

    #[test]
    /// Cursor on the opening quote selects the pair starting at that quote.
    fn test_find_around_quote_span_cursor_on_open_quote() {
        let buffer = TextBuffer::from_str("\"hello\"");
        assert_eq!(find_around_quote_span(&buffer, 0, '"'), Some((0, 7)));
    }

    #[test]
    /// Cursor on the closing quote still finds the enclosing pair.
    fn test_find_around_quote_span_cursor_on_close_quote() {
        let buffer = TextBuffer::from_str("\"hello\"");
        assert_eq!(find_around_quote_span(&buffer, 6, '"'), Some((0, 7)));
    }

    #[test]
    /// An empty string literal has no inner content; find_inner_quote_span returns None.
    fn test_find_inner_quote_span_empty_string() {
        let buffer = TextBuffer::from_str("\"\"");
        assert_eq!(find_inner_quote_span(&buffer, 0, '"'), None);
    }

    #[test]
    /// A backslash-escaped quote inside a string should not end the span.
    fn test_find_around_quote_span_escaped_quote() {
        // "fo\"bar" — the \" at index 3 must be skipped; span covers indices 0..8.
        let buffer = TextBuffer::from_str("\"fo\\\"bar\"");
        // Cursor on 'b' at index 5 (inside the string).
        assert_eq!(find_around_quote_span(&buffer, 5, '"'), Some((0, 9)));
    }

    #[test]
    /// Two consecutive backslashes before a quote mean the quote is unescaped.
    fn test_find_around_quote_span_two_backslashes_before_quote() {
        // "a\\" — the \\ is two chars; the closing " is unescaped.
        // Buffer chars: " a \ \ " → indices 0..5
        let buffer = TextBuffer::from_str("\"a\\\\\"");
        assert_eq!(find_around_quote_span(&buffer, 1, '"'), Some((0, 5)));
    }

    #[test]
    /// No enclosing quote pair returns None.
    fn test_find_around_quote_span_no_enclosing() {
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(find_around_quote_span(&buffer, 2, '"'), None);
    }

    #[test]
    /// When cursor is between two string literals, find the next string to the right.
    fn test_find_around_quote_span_fallback_to_right() {
        // "a" x "b" — cursor on 'x' at index 4; left pair encloses indices 0..3 but
        // does not contain cursor (4 > 2), so fallback finds the right pair at 6..9.
        let buffer = TextBuffer::from_str("\"a\" x \"b\"");
        assert_eq!(find_around_quote_span(&buffer, 4, '"'), Some((6, 9)));
    }

    #[test]
    /// Single-quote text object works identically to double-quote.
    fn test_find_around_quote_span_single_quote() {
        let buffer = TextBuffer::from_str("'test'");
        assert_eq!(find_around_quote_span(&buffer, 2, '\''), Some((0, 6)));
    }

    #[test]
    /// Backtick text object works identically to double-quote.
    fn test_find_around_quote_span_backtick() {
        let buffer = TextBuffer::from_str("`cmd`");
        assert_eq!(find_around_quote_span(&buffer, 1, '`'), Some((0, 5)));
    }

    #[test]
    /// A single-quote inside a double-quoted string does not interfere.
    fn test_find_around_quote_span_nested_different_quote() {
        // "it's great" — cursor at index 4.
        let buffer = TextBuffer::from_str("\"it's great\"");
        assert_eq!(find_around_quote_span(&buffer, 4, '"'), Some((0, 12)));
    }

    #[test]
    /// Cursor inside a multi-line string finds the enclosing pair across lines.
    fn test_find_around_quote_span_multiline() {
        // Buffer: "line1\nline2"
        // Open quote at index 0, close quote at index 12.
        // Cursor on 'l' at index 1 is inside the string (one quote before it → odd).
        let buffer = TextBuffer::from_str("\"line1\nline2\"");
        assert_eq!(find_around_quote_span(&buffer, 1, '"'), Some((0, 13)));
    }

    #[test]
    /// Cursor on the closing quote of a pair is treated as inside the string
    /// (odd parity before cursor), so the enclosing pair is returned.
    fn test_find_around_quote_span_no_cross_line_merge() {
        // var("key");\nprintln!("Hello!");
        // Quotes at indices 4 (open) and 8 (close) on line 0.
        // Cursor on `"` at index 8: parity before index 8 is 1 (odd) → inside
        // the string opened at 4. Close found at index 8 → span (4, 9).
        let buffer = TextBuffer::from_str("var(\"key\");\nprintln!(\"Hello!\");");
        assert_eq!(find_around_quote_span(&buffer, 8, '"'), Some((4, 9)));
    }

    #[test]
    /// When the cursor is outside all strings and the same line has no quote pair,
    /// subsequent lines are not scanned — returns None.
    fn test_find_around_quote_span_no_fallback_to_next_line() {
        // Line 0: no quotes
        // Line 1: "hello"
        let buffer = TextBuffer::from_str("abc\n\"hello\"");
        // Cursor on 'a' at index 0 (line 0, no quotes on this line).
        // No same-line pair exists, and subsequent lines are not scanned.
        assert_eq!(find_around_quote_span(&buffer, 0, '"'), None);
    }

    #[test]
    /// A single-character string `"x"` selects only `x` with the inner object.
    fn test_find_inner_quote_span_single_char_string() {
        let buffer = TextBuffer::from_str("\"x\"");
        assert_eq!(find_inner_quote_span(&buffer, 1, '"'), Some((1, 2)));
    }

    #[test]
    /// Empty buffer returns None.
    fn test_find_around_quote_span_empty_buffer() {
        let buffer = TextBuffer::from_str("");
        assert_eq!(find_around_quote_span(&buffer, 0, '"'), None);
    }

    #[test]
    /// An escaped backslash immediately before a quote (even count) leaves the
    /// quote unescaped, so the span closes there.
    fn test_find_around_quote_span_escaped_backslash_before_close() {
        // "a\\" where \\ represents two literal backslash chars then close quote.
        // The close quote is NOT escaped.
        // Buffer: " a \ \ " → chars at 0,1,2,3,4
        let buffer = TextBuffer::from_str("\"a\\\\\"");
        assert_eq!(find_inner_quote_span(&buffer, 1, '"'), Some((1, 4)));
    }

    #[test]
    /// Cursor at buffer start with a multi-line string followed by a second
    /// string on a later line. `di"` must target the first string (the one
    /// the cursor is inside), not the second string.
    fn test_find_around_quote_span_multiline_not_second_string() {
        // const string: &str = "hello,\n    world";\n\nconst string2: &str = "hello2";
        // The first string opens at index 21 (`"hello,`) and closes at index 39 (`world"`).
        // The second string opens later in the buffer.
        // Cursor at index 0: even parity, not on a quote, same-line scan right.
        // Line 0 is `const string: &str = "hello,` — one quote at index 21, no pair
        // on this line (no closing quote before the newline). Same-line scan → None.
        let buffer = TextBuffer::from_str(
            "const string: &str = \"hello,\n    world\";\n\nconst string2: &str = \"hello2\";",
        );
        assert_eq!(find_around_quote_span(&buffer, 0, '"'), None);
    }

    #[test]
    /// Cursor inside the first word of a multi-line string finds the enclosing pair.
    fn test_find_around_quote_span_inside_multiline_string() {
        // const string: &str = "hello,\n    world";
        // Open quote at index 21, close quote at index 38.
        // Cursor on 'h' of "hello," at index 22: one quote before it (index 21) → odd.
        let buffer = TextBuffer::from_str("const string: &str = \"hello,\n    world\";");
        // open=21, close=38 → span (21, 39)
        assert_eq!(find_around_quote_span(&buffer, 22, '"'), Some((21, 39)));
    }
}
