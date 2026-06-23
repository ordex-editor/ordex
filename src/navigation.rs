//! Word navigation logic
//!
//! Provides functions for moving the cursor by words, respecting word
//! boundaries defined by whitespace and punctuation characters.

use crate::syntax::engine::LineLexMode;
use crate::syntax::{HighlightSpan, SyntaxClass, SyntaxEngine};
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

/// Find the deletion start for a Ctrl-w backward-delete in insert mode.
///
/// The rules differ from the normal-mode `b` motion:
///
/// - When `char_idx` is zero, there is nothing to delete: returns `0`.
/// - When the character immediately before `char_idx` is a newline (i.e. the
///   cursor is at the start of a line), returns `char_idx - 1` so that the
///   newline itself is deleted, joining the current line with the previous one.
/// - Otherwise the scan stays within the current line: it never crosses a `\n`
///   boundary.  If only horizontal whitespace precedes the cursor on the current
///   line the deletion reaches back to the first character of that line (stopping
///   before the `\n`).  If a word precedes optional whitespace, the word segment
///   is deleted (whitespace between the cursor and the word is included in the
///   deletion, matching Vim's CTRL-W semantics).
///
/// Returns the index of the first character that should be deleted; the range
/// `[result, char_idx)` is the deletion range.
pub(crate) fn find_prev_word_start_insert_mode(buffer: &TextBuffer, char_idx: usize) -> usize {
    if char_idx == 0 {
        // Already at the very beginning of the buffer: nothing to delete.
        return 0;
    }

    // When the cursor is at the start of a line (the previous character is a
    // newline), delete the newline to join with the line above.
    if buffer.char_at(char_idx - 1) == Some('\n') {
        return char_idx - 1;
    }

    let mut idx = char_idx;

    // Move back one position to begin the scan.
    idx -= 1;

    // Skip horizontal whitespace (spaces, tabs, etc.) but stop at a newline.
    while idx > 0 {
        match buffer.char_at(idx) {
            // Whitespace that is not a newline: keep scanning backward.
            Some(c) if c.is_whitespace() && c != '\n' => idx -= 1,
            // Newline: the scan would cross into the previous line — stop here
            // and include the whitespace characters already skipped.
            Some('\n') => return idx + 1,
            _ => break,
        }
    }
    // Check whether the very first character (idx == 0) is horizontal
    // whitespace; if so, include it in the deletion.
    if let Some(c) = buffer.char_at(idx)
        && c.is_whitespace()
        && c != '\n'
    {
        // idx is 0 and it is a space/tab; include it.
        return idx;
    }

    // Walk back over the word segment that `idx` now sits inside.
    let Some(target_kind) = buffer
        .char_at(idx)
        .and_then(|ch| word_segment_kind(ch, WordStyle::Small))
    else {
        // Not inside any word segment (e.g. we landed on whitespace or a
        // character that word_segment_kind does not classify): return the
        // current position so the call site deletes only the whitespace already
        // identified.
        return idx + 1;
    };

    while idx > 0 {
        match buffer.char_at(idx - 1) {
            // Stay in the same word segment class; keep going.
            Some(c)
                if word_segment_kind(c, WordStyle::Small)
                    .is_some_and(|kind| kind == target_kind) =>
            {
                idx -= 1;
            }
            // Newline boundary: stop here; do not include the `\n`.
            Some('\n') => break,
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
/// When a language profile is active the algorithm delegates entirely to the
/// syntax engine — no character scanning from buffer start is needed:
///
/// 1. **Cursor inside a same-line string** (`SyntaxClass::String` span covers
///    the cursor column): read `open` and `close` directly from the span's
///    `start_col` / `end_col`. The span's opener must contain `quote`.
///
/// 2. **Cursor inside a multi-line string** (`exact_entry_mode_for_line`
///    returns `LineLexMode::String`): walk lines backward to find the opening
///    line, use the last `String` span on that line as the opener, then walk
///    lines forward (via entry-mode checks) to find the closing line and read
///    `end_col` from its span. Entirely span-based — no character scanning.
///
/// 3. **Cursor outside all strings** (no covering `String` span, entry mode is
///    `Plain`): scan the current line's spans for the first `String` span whose
///    opener lies to the right of the cursor. A pair is only returned when both
///    delimiters are on the same line.
///
/// When no language profile is active (plain-text fallback) the algorithm falls
/// back to character-by-character parity counting from the buffer start, which
/// is always correct for plain text but is O(cursor position).
///
/// The returned span is `(open, close + 1)` — both quote characters included.
pub(crate) fn find_around_quote_span(
    buffer: &TextBuffer,
    syntax: &SyntaxEngine,
    cursor_char_idx: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let total = buffer.chars_count();
    if total == 0 {
        return None;
    }

    let idx = cursor_char_idx.min(total.saturating_sub(1));

    // Syntax-aware path: only taken when a language profile is active.
    if syntax.has_active_profile() {
        return find_around_quote_span_syntax(buffer, syntax, idx, total, quote);
    }

    // Plain-text fallback: parity counting from the buffer start.
    find_around_quote_span_plain(buffer, idx, total, quote)
}

/// Find the inclusive/exclusive span inside the smallest surrounding quote pair.
///
/// Returns `None` when no enclosing pair exists or when the inner span is empty
/// (i.e. the string literal contains no characters between the quotes).
pub(crate) fn find_inner_quote_span(
    buffer: &TextBuffer,
    syntax: &SyntaxEngine,
    cursor_char_idx: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let (start, end) = find_around_quote_span(buffer, syntax, cursor_char_idx, quote)?;
    let inner_start = start.saturating_add(1);
    let inner_end = end.saturating_sub(1);
    // An empty string literal (open immediately followed by close) has no inner
    // content to select, so the operation is a no-op.
    if inner_start >= inner_end {
        return None;
    }
    Some((inner_start, inner_end))
}

/// Syntax-aware implementation of `find_around_quote_span`.
///
/// Requires an active language profile.
fn find_around_quote_span_syntax(
    buffer: &TextBuffer,
    syntax: &SyntaxEngine,
    idx: usize,
    total: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let cursor_line = buffer.char_to_line(idx);
    let cursor_line_start = buffer.line_to_char(cursor_line);
    let cursor_col = idx - cursor_line_start;

    let cursor_spans = syntax.compute_spans_for_line(buffer, cursor_line);

    // Case 1a: cursor is inside a same-line string whose closer is on the same
    // line — read both delimiters directly from the span.
    // Case 1b: cursor is on the opening line of a multi-line string (span
    // covers cursor but closer is not on this line) — hand off to the multiline
    // path which walks forward to find the closing line.
    if let Some(span) = string_span_covering(&cursor_spans, cursor_col)
        && span_opener_contains_quote(buffer, cursor_line_start, span, quote)
    {
        if let Some(bounds) =
            span_quote_bounds(buffer, cursor_line_start, span, cursor_line_start, quote)
        {
            return Some(bounds);
        }
        // Closer is on a later line — cursor is on the opening line itself.
        return find_around_multiline_quote_span(buffer, syntax, cursor_line, total, quote);
    }

    // Case 2: cursor is inside a multi-line string (current line starts inside
    // an open string from a previous line).
    if matches!(
        syntax.exact_entry_mode_for_line(buffer, cursor_line),
        LineLexMode::String { .. }
    ) {
        return find_around_multiline_quote_span(buffer, syntax, cursor_line, total, quote);
    }

    // Case 3: cursor is outside all strings — scan spans to the right on the
    // same line only.
    find_quote_span_right_of_cursor(buffer, &cursor_spans, cursor_line_start, cursor_col, quote)
}

/// Find the `(open, close + 1)` span for a multi-line string using `quote`,
/// given that the cursor is on either the opening line of the string or a
/// continuation line.
///
/// When `cursor_line`'s entry mode is `Plain`, the opener is on `cursor_line`
/// itself. When the entry mode is `LineLexMode::String`, walk backward to the
/// actual opening line first.
///
/// Uses the last `String` span on the opening line, because an earlier
/// complete string on the same line exits `String` mode before the opener that
/// actually carries into the next line.
///
/// Detects the closing line by comparing `span.end_col` against the line
/// length: a span that ends before the end of its line closed the string on
/// that line; one that reaches the end of its line continues onto the next.
fn find_around_multiline_quote_span(
    buffer: &TextBuffer,
    syntax: &SyntaxEngine,
    cursor_line: usize,
    total: usize,
    quote: char,
) -> Option<(usize, usize)> {
    // Resolve the opening line.
    let open_line = if !matches!(
        syntax.exact_entry_mode_for_line(buffer, cursor_line),
        LineLexMode::String { .. }
    ) {
        // Entry mode is Plain — opener is on cursor_line.
        cursor_line
    } else {
        // Walk backward until reaching a line whose entry mode is not String.
        let mut line = cursor_line.checked_sub(1)?;
        loop {
            if !matches!(
                syntax.exact_entry_mode_for_line(buffer, line),
                LineLexMode::String { .. }
            ) {
                break;
            }
            line = line.checked_sub(1)?;
        }
        line
    };

    let open_line_start = buffer.line_to_char(open_line);
    let open_spans = syntax.compute_spans_for_line(buffer, open_line);

    // Use the *last* String span on the opening line whose opener contains
    // `quote`. An earlier complete pair on the same line would exit String
    // mode and re-enter Plain before the actual multi-line opener.
    let opener_span = open_spans.iter().rev().find(|s| {
        s.class == SyntaxClass::String
            && span_opener_contains_quote(buffer, open_line_start, s, quote)
    })?;

    // Anchor `open` at the `quote` character within the opener, not the prefix.
    let open = span_opener_quote_idx(buffer, open_line_start, opener_span, quote)?;

    // Walk forward from the opening line to find the line where the string
    // closes. A span that ends before the end of its line means the closer was
    // consumed on that line. A span that reaches the end of its line means the
    // string continues on the next line.
    //
    // Using span.end_col vs line length (rather than the next line's entry
    // mode) correctly handles the case where a string ends mid-line and a new
    // string begins on the same line before the line ends.
    let last_line = buffer.lines_count().saturating_sub(1);
    for close_line in open_line..=last_line {
        let close_spans = syntax.compute_spans_for_line(buffer, close_line);
        // The continuation span starts at column 0 on lines after the opener.
        // On the opener line itself, use the opener span already found.
        let close_span = if close_line == open_line {
            opener_span
        } else {
            // Continuation lines have the String span starting at column 0.
            close_spans
                .iter()
                .find(|s| s.class == SyntaxClass::String && s.start_col == 0)?
        };

        let line_len = buffer.line_len(close_line);
        if close_span.end_col < line_len || close_line == last_line {
            // String ends on `close_line`. Anchor `close` at the `quote`
            // character within the closer (scan backward from `end_col - 1`).
            let close_line_start = buffer.line_to_char(close_line);
            let close = span_closer_quote_idx(buffer, close_line_start, close_span, quote)?;
            return Some((open, close + 1));
        }
    }

    // Reached end of buffer without finding a closer (unclosed string).
    Some((open, total))
}

/// Scan the spans of the cursor's line for the first `String` span that starts
/// strictly to the right of `cursor_col` and whose opener contains `quote`.
/// Returns the `(open, close + 1)` span anchored at `quote` on both sides,
/// or `None` when the string is multi-line (closer not on this line).
fn find_quote_span_right_of_cursor(
    buffer: &TextBuffer,
    line_spans: &[HighlightSpan],
    line_start: usize,
    cursor_col: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let span = line_spans.iter().find(|s| {
        s.class == SyntaxClass::String
            && s.start_col > cursor_col
            && span_opener_contains_quote(buffer, line_start, s, quote)
    })?;
    span_quote_bounds(buffer, line_start, span, line_start, quote)
}

/// Plain-text fallback: count unescaped occurrences of `quote` from the buffer
/// start to determine whether the cursor is inside a string, then return the
/// enclosing `(open, close + 1)` span.
///
/// Used only when no language profile is active. O(cursor position).
fn find_around_quote_span_plain(
    buffer: &TextBuffer,
    idx: usize,
    total: usize,
    quote: char,
) -> Option<(usize, usize)> {
    // Count unescaped quote characters from buffer start up to (not including)
    // the cursor. An odd count means the cursor is inside a string.
    let mut open_idx: Option<usize> = None;
    let mut count = 0usize;
    for i in 0..idx {
        if buffer.char_at(i) == Some(quote) && !is_escaped_at(buffer, i) {
            count += 1;
            if !count.is_multiple_of(2) {
                open_idx = Some(i);
            }
        }
    }

    if !count.is_multiple_of(2) {
        // Odd parity: cursor is inside a string whose opener is `open_idx`.
        let open = open_idx?;
        let close = find_unescaped_quote_after(buffer, open + 1, total, quote)?;
        return Some((open, close + 1));
    }

    // Even parity and cursor is on a quote: treat as opener.
    if buffer.char_at(idx) == Some(quote) && !is_escaped_at(buffer, idx) {
        let close = find_unescaped_quote_after(buffer, idx + 1, total, quote)?;
        return Some((idx, close + 1));
    }

    // Even parity, cursor not on a quote: scan right on the same line only.
    let line_idx = buffer.char_to_line(idx);
    let line_end = if line_idx + 1 < buffer.lines_count() {
        buffer.line_to_char(line_idx + 1)
    } else {
        total
    };
    let open = find_unescaped_quote_after(buffer, idx + 1, line_end, quote)?;
    let close = find_unescaped_quote_after(buffer, open + 1, line_end, quote)?;
    Some((open, close + 1))
}

/// Return the `SyntaxClass::String` span that covers `col`, if any.
///
/// Returns `Some` when a string span's `[start_col, end_col)` range contains
/// `col`; `None` otherwise.
fn string_span_covering(spans: &[HighlightSpan], col: usize) -> Option<&HighlightSpan> {
    spans
        .iter()
        .find(|s| s.class == SyntaxClass::String && s.covers(col))
}

/// Return the absolute buffer index of the `quote` character within the opener
/// of `span`. Scans forward from `span.start_col` up to (but not including)
/// `span.end_col`, stopping at the first occurrence of `quote`.
///
/// Returns `None` when `quote` is not found in the opener region, which means
/// the span uses a different quote character.
fn span_opener_quote_idx(
    buffer: &TextBuffer,
    line_start: usize,
    span: &HighlightSpan,
    quote: char,
) -> Option<usize> {
    // Cap the scan at a small constant so we do not scan into content on very
    // long strings. 20 characters is sufficient for any known prefix/hash
    // combination without being language-specific.
    let scan_end = (span.start_col + 20).min(span.end_col);
    for col in span.start_col..scan_end {
        if buffer.char_at(line_start + col) == Some(quote) {
            return Some(line_start + col);
        }
    }
    None
}

/// Return whether the opener of `span` contains `quote`.
///
/// Implemented as `span_opener_quote_idx(...).is_some()`.
fn span_opener_contains_quote(
    buffer: &TextBuffer,
    line_start: usize,
    span: &HighlightSpan,
    quote: char,
) -> bool {
    span_opener_quote_idx(buffer, line_start, span, quote).is_some()
}

/// Return the absolute buffer index of the last `quote` character within the
/// closer of `span` on `close_line_start`. Scans backward from `end_col - 1`.
///
/// For `"hello"` the closer is `"` at `end_col - 1`.
/// For `r#"hello"#` the closer is `"#`; the `"` is at `end_col - 2`.
/// Returns `None` when `quote` is not found within the last 20 characters of
/// the span.
fn span_closer_quote_idx(
    buffer: &TextBuffer,
    close_line_start: usize,
    span: &HighlightSpan,
    quote: char,
) -> Option<usize> {
    if span.end_col == 0 {
        return None;
    }
    let scan_start = span.end_col.saturating_sub(20).max(span.start_col);
    for col in (scan_start..span.end_col).rev() {
        if buffer.char_at(close_line_start + col) == Some(quote) {
            return Some(close_line_start + col);
        }
    }
    None
}

/// Return `(open, close + 1)` anchored at the `quote` characters within
/// `span`'s opener and closer, or `None` when:
/// - the opener does not contain `quote` (different quote type), or
/// - the closer does not contain `quote` (multi-line span with no close on
///   this line).
///
/// `open_line_start` and `close_line_start` may differ for multi-line spans
/// but are the same for single-line ones.
fn span_quote_bounds(
    buffer: &TextBuffer,
    open_line_start: usize,
    span: &HighlightSpan,
    close_line_start: usize,
    quote: char,
) -> Option<(usize, usize)> {
    let open = span_opener_quote_idx(buffer, open_line_start, span, quote)?;
    let close = span_closer_quote_idx(buffer, close_line_start, span, quote)?;
    // Reject multi-line spans where opener and closer are the same index
    // (would mean the span has no content) or closer precedes opener.
    if close <= open {
        return None;
    }
    Some((open, close + 1))
}

/// Scan forward from `start` (inclusive) up to `limit` (exclusive) and return
/// the index of the first unescaped occurrence of `quote`, or `None`.
fn find_unescaped_quote_after(
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
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 2, '"'),
            Some((0, 7))
        );
    }

    #[test]
    /// Basic inner-quote span: cursor inside a double-quoted string.
    fn test_find_inner_quote_span_basic() {
        // "hello" — cursor on 'l' at index 2; inner span excludes the quotes.
        let buffer = TextBuffer::from_str("\"hello\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_inner_quote_span(&buffer, &syntax, 2, '"'),
            Some((1, 6))
        );
    }

    #[test]
    /// Cursor on the opening quote selects the pair starting at that quote.
    fn test_find_around_quote_span_cursor_on_open_quote() {
        let buffer = TextBuffer::from_str("\"hello\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 0, '"'),
            Some((0, 7))
        );
    }

    #[test]
    /// Cursor on the closing quote still finds the enclosing pair.
    fn test_find_around_quote_span_cursor_on_close_quote() {
        let buffer = TextBuffer::from_str("\"hello\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 6, '"'),
            Some((0, 7))
        );
    }

    #[test]
    /// An empty string literal has no inner content; find_inner_quote_span returns None.
    fn test_find_inner_quote_span_empty_string() {
        let buffer = TextBuffer::from_str("\"\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(find_inner_quote_span(&buffer, &syntax, 0, '"'), None);
    }

    #[test]
    /// A backslash-escaped quote inside a string should not end the span.
    fn test_find_around_quote_span_escaped_quote() {
        // "fo\"bar" — the \" at index 3 must be skipped; span covers indices 0..8.
        let buffer = TextBuffer::from_str("\"fo\\\"bar\"");
        let syntax = SyntaxEngine::new();
        // Cursor on 'b' at index 5 (inside the string).
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 5, '"'),
            Some((0, 9))
        );
    }

    #[test]
    /// Two consecutive backslashes before a quote mean the quote is unescaped.
    fn test_find_around_quote_span_two_backslashes_before_quote() {
        // "a\\" — the \\ is two chars; the closing " is unescaped.
        // Buffer chars: " a \ \ " → indices 0..5
        let buffer = TextBuffer::from_str("\"a\\\\\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 1, '"'),
            Some((0, 5))
        );
    }

    #[test]
    /// No enclosing quote pair returns None.
    fn test_find_around_quote_span_no_enclosing() {
        let buffer = TextBuffer::from_str("hello");
        let syntax = SyntaxEngine::new();
        assert_eq!(find_around_quote_span(&buffer, &syntax, 2, '"'), None);
    }

    #[test]
    /// When cursor is between two string literals, find the next string to the right.
    fn test_find_around_quote_span_fallback_to_right() {
        // "a" x "b" — cursor on 'x' at index 4; left pair encloses indices 0..3 but
        // does not contain cursor (4 > 2), so fallback finds the right pair at 6..9.
        let buffer = TextBuffer::from_str("\"a\" x \"b\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 4, '"'),
            Some((6, 9))
        );
    }

    #[test]
    /// Single-quote text object works identically to double-quote.
    fn test_find_around_quote_span_single_quote() {
        let buffer = TextBuffer::from_str("'test'");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 2, '\''),
            Some((0, 6))
        );
    }

    #[test]
    /// Backtick text object works identically to double-quote.
    fn test_find_around_quote_span_backtick() {
        let buffer = TextBuffer::from_str("`cmd`");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 1, '`'),
            Some((0, 5))
        );
    }

    #[test]
    /// A single-quote inside a double-quoted string does not interfere.
    fn test_find_around_quote_span_nested_different_quote() {
        // "it's great" — cursor at index 4.
        let buffer = TextBuffer::from_str("\"it's great\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 4, '"'),
            Some((0, 12))
        );
    }

    #[test]
    /// Cursor inside a multi-line string finds the enclosing pair across lines.
    /// Uses the plain-text parity fallback (no language profile loaded).
    fn test_find_around_quote_span_multiline() {
        // Buffer: "line1\nline2"
        // Open quote at index 0, close quote at index 12.
        // Plain-text parity: one quote before cursor at index 1 → odd → inside string.
        let buffer = TextBuffer::from_str("\"line1\nline2\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 1, '"'),
            Some((0, 13))
        );
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
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 8, '"'),
            Some((4, 9))
        );
    }

    #[test]
    /// When the cursor is outside all strings and the same line has no quote pair,
    /// subsequent lines are not scanned — returns None.
    fn test_find_around_quote_span_no_fallback_to_next_line() {
        // Line 0: no quotes
        // Line 1: "hello"
        let buffer = TextBuffer::from_str("abc\n\"hello\"");
        let syntax = SyntaxEngine::new();
        // Cursor on 'a' at index 0 (line 0, no quotes on this line).
        // No same-line pair exists, and subsequent lines are not scanned.
        assert_eq!(find_around_quote_span(&buffer, &syntax, 0, '"'), None);
    }

    #[test]
    /// A single-character string `"x"` selects only `x` with the inner object.
    fn test_find_inner_quote_span_single_char_string() {
        let buffer = TextBuffer::from_str("\"x\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_inner_quote_span(&buffer, &syntax, 1, '"'),
            Some((1, 2))
        );
    }

    #[test]
    /// Empty buffer returns None.
    fn test_find_around_quote_span_empty_buffer() {
        let buffer = TextBuffer::from_str("");
        let syntax = SyntaxEngine::new();
        assert_eq!(find_around_quote_span(&buffer, &syntax, 0, '"'), None);
    }

    #[test]
    /// An escaped backslash immediately before a quote (even count) leaves the
    /// quote unescaped, so the span closes there.
    fn test_find_around_quote_span_escaped_backslash_before_close() {
        // "a\\" where \\ represents two literal backslash chars then close quote.
        // The close quote is NOT escaped.
        // Buffer: " a \ \ " → chars at 0,1,2,3,4
        let buffer = TextBuffer::from_str("\"a\\\\\"");
        let syntax = SyntaxEngine::new();
        assert_eq!(
            find_inner_quote_span(&buffer, &syntax, 1, '"'),
            Some((1, 4))
        );
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
        let syntax = SyntaxEngine::new();
        assert_eq!(find_around_quote_span(&buffer, &syntax, 0, '"'), None);
    }

    #[test]
    /// Cursor inside the first word of a multi-line string finds the enclosing pair.
    /// Uses the plain-text parity fallback (no language profile loaded).
    fn test_find_around_quote_span_inside_multiline_string() {
        // const string: &str = "hello,\n    world";
        // Open quote at index 21, close quote at index 38.
        // Plain-text parity: one quote before cursor at index 22 (index 21) → odd.
        let buffer = TextBuffer::from_str("const string: &str = \"hello,\n    world\";");
        let syntax = SyntaxEngine::new();
        // open=21, close=38 → span (21, 39)
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 22, '"'),
            Some((21, 39))
        );
    }

    #[test]
    /// Without an active language profile the plain-text parity fallback is used,
    /// which has no concept of comments. A `"` on a `//` line shifts parity and
    /// causes the wrong pair to be returned. This is expected degraded behavior;
    /// real language files use the syntax-span path, which reads string boundaries
    /// directly from `SyntaxClass::String` spans and never inspects comment lines.
    fn test_find_around_quote_span_comment_quote_no_profile() {
        // // "\nconst x: &str = "hello";
        // Without a profile: plain-text parity sees quote at index 3, making
        // parity odd before cursor at index 5. Open = 3, close at index 21.
        let buffer = TextBuffer::from_str("// \"\nconst x: &str = \"hello\";");
        let syntax = SyntaxEngine::new();
        // Cursor on 'c' of "const" at index 5 — line 1.
        // Parity before index 5: quote at index 3 → count 1 (odd).
        // Open = 3, close found after 3 at index 21 → span (3, 22).
        assert_eq!(
            find_around_quote_span(&buffer, &syntax, 5, '"'),
            Some((3, 22))
        );
    }

    /// At char_idx 0 there is nothing to delete; returns 0.
    #[test]
    fn test_find_prev_word_start_insert_mode_at_start_of_buffer() {
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 0), 0);
    }

    /// Cursor immediately after a word on a single line: deletes the word.
    #[test]
    fn test_find_prev_word_start_insert_mode_simple_word() {
        // "hello" — cursor after 'o' (char_idx 5): should return 0.
        let buffer = TextBuffer::from_str("hello");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 5), 0);
    }

    /// Cursor after trailing spaces: skips the spaces and deletes the preceding
    /// word segment (matching Vim's CTRL-W: skip whitespace then delete word).
    #[test]
    fn test_find_prev_word_start_insert_mode_spaces_after_word() {
        // "foo   " — cursor at idx 6 (after three spaces): whitespace is skipped
        // and "foo" is deleted, so the result is 0 (start of "foo").
        let buffer = TextBuffer::from_str("foo   ");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 6), 0);
    }

    /// When the cursor is at column 0 (char immediately before is a newline),
    /// returns char_idx - 1 so the newline is deleted (joins lines).
    #[test]
    fn test_find_prev_word_start_insert_mode_at_line_start_joins_lines() {
        // "prev\nnext" — cursor at idx 5 ('n' of "next"): char at idx 4 is '\n'.
        let buffer = TextBuffer::from_str("prev\nnext");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 5), 4);
    }

    /// Cursor after spaces-only content on a line: deletes only the spaces,
    /// does not cross the preceding newline.
    #[test]
    fn test_find_prev_word_start_insert_mode_spaces_only_line() {
        // "prev\n   " — cursor at idx 8 (after three spaces on line 2).
        // Expected: idx 5 (first space on line 2), not 0 or 4.
        let buffer = TextBuffer::from_str("prev\n   ");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 8), 5);
    }

    /// Cursor inside a word preceded by spaces on the same line: deletes the
    /// word segment only, leaving the spaces.
    #[test]
    fn test_find_prev_word_start_insert_mode_word_after_spaces() {
        // "   word" — cursor at idx 7 (after 'd'): should return 3 (start of
        // "word"), not 0 (start of the spaces).
        let buffer = TextBuffer::from_str("   word");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 7), 3);
    }

    /// Cursor inside leading spaces on a line should delete only the spaces
    /// up to the line start, without crossing the preceding newline.
    #[test]
    fn test_find_prev_word_start_insert_mode_cursor_inside_leading_spaces() {
        // "prev\n    word" — cursor at idx 7 (third space, 0-based col 2).
        // Spaces start at idx 5; the scan must stop at idx 5, not at idx 4 (\n).
        let buffer = TextBuffer::from_str("prev\n    word");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 7), 5);
    }

    /// Cursor after punctuation on a line deletes the punctuation segment.
    #[test]
    fn test_find_prev_word_start_insert_mode_punctuation_segment() {
        // "prev\n!!!" — cursor at idx 8 (after three '!'): should return 5
        // (start of '!!!'), not cross the '\n'.
        let buffer = TextBuffer::from_str("prev\n!!!");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 8), 5);
    }

    /// Mixed punctuation and word on same line: deletes only the current segment.
    #[test]
    fn test_find_prev_word_start_insert_mode_mixed_segments_same_line() {
        // "foo!!!" — cursor at idx 6: the '!!!' segment starts at idx 3.
        let buffer = TextBuffer::from_str("foo!!!");
        assert_eq!(find_prev_word_start_insert_mode(&buffer, 6), 3);
    }
}
