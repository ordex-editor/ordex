//! Shared syntax helper predicates.
//!
//! These helpers keep the profile modules focused on language rules instead of
//! repeating low-level boundary and scanning logic.

/// Return whether `chars[start..]` begins with `pattern`.
pub(crate) fn starts_with(chars: &[char], start: usize, pattern: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    chars
        .get(start..start.saturating_add(pattern_chars.len()))
        .is_some_and(|slice| slice == pattern_chars.as_slice())
}

/// Return whether `c` can start an identifier-like token.
pub(crate) fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

/// Return whether `c` can continue an identifier-like token.
pub(crate) fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Scan one identifier-like token and return its exclusive end index.
pub(crate) fn scan_identifier(chars: &[char], start: usize) -> usize {
    let mut idx = start;
    while idx < chars.len() && is_ident_continue(chars[idx]) {
        idx += 1;
    }
    idx
}

/// Return whether a character may continue a numeric literal.
pub(crate) fn is_number_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')
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

/// Return whether a Markdown delimiter can open emphasis conservatively.
pub(crate) fn markdown_can_open(prev: Option<char>, next: Option<char>) -> bool {
    // Conservative Markdown highlighting prefers false negatives over
    // miscoloring prose, so we require non-whitespace content on the right and
    // reject obvious mid-word openings.
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
    // Closing runs need non-whitespace content on the left and should avoid
    // splitting ordinary words where `_` and `*` appear as punctuation.
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
pub(crate) fn fenced_marker(text: &str) -> Option<(char, usize)> {
    let mut chars = text.chars();
    let marker = chars.next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = 1 + chars.take_while(|&c| c == marker).count();
    (count >= 3).then_some((marker, count))
}
