//! Shared syntax helper predicates.
//!
//! These helpers keep the generic lexer and profile modules focused on behavior
//! instead of repeating low-level character scanning logic.

use crate::syntax::profile::{EscapeMode, IdentifierCharSet, IdentifierPattern, NumberPattern};

/// Return whether `chars[start..]` begins with `pattern`.
pub(crate) fn starts_with(chars: &[char], start: usize, pattern: &str) -> bool {
    let mut idx = start;
    for expected in pattern.chars() {
        if chars.get(idx).copied() != Some(expected) {
            return false;
        }
        idx += 1;
    }
    true
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
///
/// # Parameters
/// - `chars`: Current line as a character slice.
/// - `start`: Column where the identifier begins.
/// - `pattern`: Identifier character rules for the active language.
pub(crate) fn scan_identifier(chars: &[char], start: usize, pattern: IdentifierPattern) -> usize {
    let mut idx = start;
    while idx < chars.len() && identifier_can_continue(pattern, chars[idx]) {
        idx += 1;
    }
    idx
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
    if chars.get(idx).is_some_and(|ch| matches!(ch, '+' | '-')) {
        idx += 1;
    }

    // Radix-prefixed literals use one dedicated digit class each so `0xff` and
    // friends stay highlighted without letting unrelated identifiers attach.
    if chars.get(idx) == Some(&'0') {
        if matches!(chars.get(idx + 1).copied(), Some('x' | 'X')) {
            return scan_prefixed_digits(chars, idx + 2, |ch| ch.is_ascii_hexdigit());
        }
        if matches!(chars.get(idx + 1).copied(), Some('b' | 'B')) {
            return scan_prefixed_digits(chars, idx + 2, |ch| matches!(ch, '0' | '1'));
        }
        if matches!(chars.get(idx + 1).copied(), Some('o' | 'O')) {
            return scan_prefixed_digits(chars, idx + 2, |ch| matches!(ch, '0'..='7'));
        }
    }

    // Decimal numbers accept one fractional part only when the dot is followed
    // by another digit, which keeps ranges like `0..count` from swallowing the
    // identifier after the range operator.
    idx = scan_digit_run(chars, idx);
    if chars.get(idx) == Some(&'.') && chars.get(idx + 1).is_some_and(|next| next.is_ascii_digit())
    {
        idx = scan_digit_run(chars, idx + 1);
    }

    // Exponents require at least one digit and may carry one sign, so partial
    // constructs like `1e+` stop before the `e` instead of over-highlighting.
    if matches!(chars.get(idx), Some('e' | 'E')) {
        let exponent_start = idx;
        idx += 1;
        if matches!(chars.get(idx), Some('+' | '-')) {
            idx += 1;
        }
        let exponent_end = scan_digit_run(chars, idx);
        if exponent_end > idx {
            idx = exponent_end;
        } else {
            idx = exponent_start;
        }
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
///
/// # Parameters
/// - `chars`: Current line as a character slice.
/// - `start`: Column immediately after the opener.
/// - `close`: Closing delimiter text to search for.
/// - `escape`: Escape handling mode for the string style.
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
///
/// # Parameters
/// - `chars`: Current line as a character slice.
/// - `start`: Column immediately after the opener.
/// - `quote`: Quote character that starts the closer.
/// - `marker`: Repeated raw-string marker character.
/// - `repeats`: Exact marker repetition count captured from the opener.
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

/// Scan one digit-or-underscore run and return its exclusive end index.
fn scan_digit_run(chars: &[char], start: usize) -> usize {
    let mut idx = start;
    while chars
        .get(idx)
        .is_some_and(|ch| ch.is_ascii_digit() || *ch == '_')
    {
        idx += 1;
    }
    idx
}

/// Scan one radix-prefixed digit run and return its exclusive end index.
fn scan_prefixed_digits(chars: &[char], start: usize, is_digit: fn(char) -> bool) -> usize {
    let mut idx = start;
    while chars.get(idx).is_some_and(|ch| is_digit(*ch) || *ch == '_') {
        idx += 1;
    }
    idx
}
