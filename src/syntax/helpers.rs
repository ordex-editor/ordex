//! Shared syntax helper predicates.
//!
//! These helpers keep the generic lexer and profile modules focused on behavior
//! instead of repeating low-level character scanning logic.

use crate::syntax::profile::{EscapeMode, IdentifierCharSet, IdentifierPattern, NumberPattern};

/// Return whether `chars[start..]` begins with `pattern`.
pub(crate) fn starts_with(chars: &[char], start: usize, pattern: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    chars
        .get(start..start.saturating_add(pattern_chars.len()))
        .is_some_and(|slice| slice == pattern_chars.as_slice())
}

/// Return whether `c` matches one identifier character set.
pub(crate) fn matches_identifier_char(set: IdentifierCharSet, c: char) -> bool {
    match set {
        IdentifierCharSet::LetterOrUnderscore => c == '_' || c.is_ascii_alphabetic(),
        IdentifierCharSet::AlnumOrUnderscore => c == '_' || c.is_ascii_alphanumeric(),
        IdentifierCharSet::AlnumUnderscoreOrDash => {
            c == '_' || c == '-' || c.is_ascii_alphanumeric()
        }
    }
}

/// Return whether `c` can start one identifier for `pattern`.
pub(crate) fn identifier_can_start(pattern: IdentifierPattern, c: char) -> bool {
    matches_identifier_char(pattern.start, c)
}

/// Return whether `c` can continue one identifier for `pattern`.
pub(crate) fn identifier_can_continue(pattern: IdentifierPattern, c: char) -> bool {
    matches_identifier_char(pattern.continue_chars, c)
}

/// Scan one identifier-like token and return its exclusive end index.
pub(crate) fn scan_identifier(chars: &[char], start: usize, pattern: IdentifierPattern) -> usize {
    let mut idx = start;
    while idx < chars.len() && identifier_can_continue(pattern, chars[idx]) {
        idx += 1;
    }
    idx
}

/// Return whether `c` may continue a generic number literal.
pub(crate) fn is_number_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')
}

/// Return whether `chars[start]` may begin a generic number literal.
pub(crate) fn number_can_start(chars: &[char], start: usize, pattern: NumberPattern) -> bool {
    let Some(ch) = chars.get(start).copied() else {
        return false;
    };
    // Signed numbers only accept `+` or `-` when a digit immediately follows so
    // punctuation like `->` and `+=` does not become part of a number token.
    ch.is_ascii_digit()
        || (pattern.allow_leading_sign
            && matches!(ch, '+' | '-')
            && chars
                .get(start + 1)
                .is_some_and(|next| next.is_ascii_digit()))
}

/// Scan a numeric-looking token and return its exclusive end index.
pub(crate) fn scan_number(chars: &[char], start: usize) -> usize {
    let mut idx = start;
    while idx < chars.len() && is_number_continue(chars[idx]) {
        idx += 1;
    }
    idx
}

/// Return the previous character before `idx`, if any.
pub(crate) fn previous_char(chars: &[char], idx: usize) -> Option<char> {
    idx.checked_sub(1).and_then(|prev| chars.get(prev).copied())
}

/// Return the byte index that corresponds to `char_idx` inside `text`.
pub(crate) fn byte_index_for_char(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

/// Find the closing delimiter for one fixed-delimiter string.
pub(crate) fn find_delimited_close(
    chars: &[char],
    start: usize,
    close: &str,
    escape: EscapeMode,
) -> Option<usize> {
    let mut idx = start;
    let mut escaped = false;

    // Backslash-aware scanning is generic enough for the common code-like string
    // formats used by Rust, TOML basic strings, and D.
    while idx < chars.len() {
        let ch = chars[idx];
        if escape == EscapeMode::Backslash && !escaped && ch == '\\' {
            escaped = true;
            idx += 1;
            continue;
        }
        if escaped {
            escaped = false;
            idx += 1;
            continue;
        }
        if starts_with(chars, idx, close) {
            return Some(idx + close.chars().count());
        }
        idx += 1;
    }

    None
}

/// Find the closing delimiter for one raw hash-delimited string.
pub(crate) fn find_hash_string_close(
    chars: &[char],
    start: usize,
    quote: char,
    marker: char,
    repeats: usize,
) -> Option<usize> {
    let mut idx = start;
    while idx < chars.len() {
        // Raw strings close on the first quote followed by the exact marker run
        // count captured from the opener.
        if chars[idx] == quote
            && (0..repeats).all(|offset| chars.get(idx + 1 + offset).copied() == Some(marker))
        {
            return Some(idx + 1 + repeats);
        }
        idx += 1;
    }
    None
}

/// Return whether a Markdown delimiter can open emphasis conservatively.
pub(crate) fn markdown_can_open(prev: Option<char>, next: Option<char>) -> bool {
    let Some(next) = next else {
        return false;
    };
    if next.is_whitespace() {
        return false;
    }
    !prev.is_some_and(|c| c.is_ascii_alphanumeric()) || !next.is_ascii_alphanumeric()
}

/// Return whether a Markdown delimiter can close emphasis conservatively.
pub(crate) fn markdown_can_close(prev: Option<char>, next: Option<char>) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    if prev.is_whitespace() {
        return false;
    }
    !next.is_some_and(|c| c.is_ascii_alphanumeric()) || !prev.is_ascii_alphanumeric()
}

/// Find a simple same-delimiter Markdown span and return the closing index.
pub(crate) fn find_markdown_delimited_span(
    chars: &[char],
    start: usize,
    delimiter: &str,
) -> Option<usize> {
    let delimiter_len = delimiter.chars().count();
    let next = chars.get(start + delimiter_len).copied();
    if !markdown_can_open(previous_char(chars, start), next) {
        return None;
    }

    let mut idx = start + delimiter_len;
    // Markdown emphasis stays conservative: only matching delimiters with valid
    // closing context become spans, otherwise the text stays plain.
    while idx + delimiter_len <= chars.len() {
        if starts_with(chars, idx, delimiter)
            && markdown_can_close(
                previous_char(chars, idx),
                chars.get(idx + delimiter_len).copied(),
            )
        {
            return Some(idx + delimiter_len);
        }
        idx += 1;
    }
    None
}

/// Count leading ASCII whitespace columns before the first non-space character.
pub(crate) fn leading_whitespace_len(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Return an ordered-list marker length when `text` begins with one.
pub(crate) fn ordered_list_marker_len(text: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut idx = 0;
    // Collect the leading digits first, then require the `". "` shape so plain
    // dotted numbers like version strings do not become list markers.
    while idx < chars.len() && chars[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == 0 || idx + 1 >= chars.len() || chars[idx] != '.' || chars[idx + 1] != ' ' {
        return None;
    }
    Some(idx + 2)
}

/// Return whether the trimmed line is an unmistakable thematic break.
pub(crate) fn is_thematic_break(text: &str) -> bool {
    let trimmed: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if trimmed.len() < 3 {
        return false;
    }
    let marker = trimmed[0];
    if !matches!(marker, '-' | '*' | '_') {
        return false;
    }
    trimmed.iter().all(|&c| c == marker)
}

/// Return fenced-code marker details from a trimmed line prefix.
pub(crate) fn fenced_marker(text: &str, allowed_markers: &[char]) -> Option<(char, usize)> {
    let mut chars = text.chars();
    let marker = chars.next()?;
    if !allowed_markers.contains(&marker) {
        return None;
    }
    let count = 1 + chars.take_while(|&c| c == marker).count();
    (count >= 3).then_some((marker, count))
}

/// Return the heading-marker length for a simple ATX heading.
pub(crate) fn heading_prefix_len(text: &str) -> Option<usize> {
    let hashes = text.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    text.chars()
        .nth(hashes)
        .is_some_and(|c| c == ' ')
        .then_some(text.chars().count())
}

/// Return the block-quote marker length for a line.
pub(crate) fn block_quote_prefix_len(text: &str) -> Option<usize> {
    if text.starts_with("> ") {
        Some(2)
    } else if text.starts_with('>') {
        Some(1)
    } else {
        None
    }
}

/// Return the list-marker length for a line.
pub(crate) fn list_marker_len(text: &str, unordered_markers: &[char]) -> Option<usize> {
    // Unordered markers win first because they are fixed-width; ordered markers
    // need a slightly more expensive digit scan.
    if unordered_markers
        .iter()
        .any(|&marker| text.starts_with(format!("{marker} ").as_str()))
    {
        return Some(2);
    }
    ordered_list_marker_len(text)
}

/// Find a one-line inline-code span and return its exclusive end column.
pub(crate) fn find_inline_code(chars: &[char], start: usize) -> Option<usize> {
    let end = chars[start + 1..]
        .iter()
        .position(|&ch| ch == '`')
        .map(|offset| start + 1 + offset + 1)?;
    (end > start + 2).then_some(end)
}

/// Find a simple inline link or image span.
pub(crate) fn find_link(chars: &[char], start: usize) -> Option<usize> {
    let offset = usize::from(chars.get(start) == Some(&'!'));
    if chars.get(start + offset) != Some(&'[') {
        return None;
    }
    // Phase 1 recognizes only one-line inline links and images so nested label
    // structures or reference-style forms naturally fall back to plain text.
    let label_end = chars[start + offset + 1..]
        .iter()
        .position(|&ch| ch == ']')
        .map(|idx| start + offset + 1 + idx)?;
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let target_end = chars[label_end + 2..]
        .iter()
        .position(|&ch| ch == ')')
        .map(|idx| label_end + 2 + idx + 1)?;
    Some(target_end)
}
