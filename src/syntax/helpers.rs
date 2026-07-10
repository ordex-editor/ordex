//! Shared syntax helper predicates.
//!
//! These helpers keep the generic lexer and profile modules focused on behavior
//! instead of repeating low-level character scanning logic.

use std::str::Chars;

use crate::syntax::profile::{
    DigitSeparator, IdentifierCharSet, IdentifierPattern, NumberPattern, NumberSuffixGroup,
};

/// One saved cursor position inside a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorMark {
    /// Byte offset into the original line.
    pub(crate) byte_pos: usize,
    /// Character column at that byte offset.
    col: usize,
}

/// Shared forward-only cursor for left-to-right line lexing.
#[derive(Debug, Clone)]
pub(crate) struct LineCursor<'a> {
    /// Full source line that the cursor walks over.
    line: &'a str,
    /// Remaining characters beginning at the current cursor position.
    chars: Chars<'a>,
    /// Current character column.
    col: usize,
    /// Character immediately before the current cursor position, if any.
    prev: Option<char>,
}

impl<'a> LineCursor<'a> {
    /// Build one cursor positioned at the start of `line`.
    pub(crate) fn new(line: &'a str) -> Self {
        Self {
            line,
            chars: line.chars(),
            col: 0,
            prev: None,
        }
    }

    /// Return the current character column.
    pub(crate) fn col(&self) -> usize {
        self.col
    }

    /// Return the unconsumed suffix of the line.
    pub(crate) fn remaining(&self) -> &'a str {
        self.chars.as_str()
    }

    /// Return the previous character before the cursor, if any.
    pub(crate) fn prev(&self) -> Option<char> {
        self.prev
    }

    /// Return whether the cursor has reached the end of the line.
    pub(crate) fn is_empty(&self) -> bool {
        self.remaining().is_empty()
    }

    /// Return the current character without consuming it.
    pub(crate) fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }

    /// Return the second character ahead without consuming it.
    pub(crate) fn peek_second(&self) -> Option<char> {
        let mut chars = self.chars.clone();
        chars.next()?;
        chars.next()
    }

    /// Return the third character ahead without consuming it.
    pub(crate) fn peek_third(&self) -> Option<char> {
        let mut chars = self.chars.clone();
        chars.next()?;
        chars.next()?;
        chars.next()
    }

    /// Return whether the remaining text begins with `pattern`.
    pub(crate) fn starts_with(&self, pattern: &str) -> bool {
        self.remaining().starts_with(pattern)
    }

    /// Save the current cursor position so callers can later measure or slice it.
    pub(crate) fn mark(&self) -> CursorMark {
        CursorMark {
            byte_pos: self.byte_pos(),
            col: self.col,
        }
    }

    /// Borrow the text consumed since `mark`.
    pub(crate) fn slice_since(&self, mark: CursorMark) -> &'a str {
        &self.line[mark.byte_pos..self.byte_pos()]
    }

    /// Borrow the text before the current cursor position.
    pub(crate) fn prefix(&self) -> &'a str {
        &self.line[..self.byte_pos()]
    }

    /// Advance by one character and return it.
    pub(crate) fn advance_char(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        self.col += 1;
        self.prev = Some(ch);
        Some(ch)
    }

    /// Advance over `pattern` when it matches at the current cursor position.
    ///
    /// Returns `true` when the full pattern was consumed and `false` when the
    /// cursor was left unchanged because the prefix did not match.
    pub(crate) fn advance_if_starts_with(&mut self, pattern: &str) -> bool {
        if !self.starts_with(pattern) {
            return false;
        }

        // Prefixes are short lexer delimiters, so consuming them character by
        // character keeps column tracking correct without allocating buffers.
        for expected in pattern.chars() {
            let actual = self
                .advance_char()
                .expect("matching prefix should remain available while consuming");
            debug_assert_eq!(actual, expected);
        }
        true
    }

    /// Advance while `predicate` accepts the current character.
    pub(crate) fn advance_while(&mut self, predicate: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&predicate) {
            self.advance_char();
        }
    }

    /// Advance to the end of the current line.
    pub(crate) fn advance_to_end(&mut self) {
        while self.advance_char().is_some() {}
    }

    /// Return the current byte position inside the original line.
    fn byte_pos(&self) -> usize {
        self.line.len() - self.remaining().len()
    }
}

/// One successfully consumed numeric core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConsumedNumberCore {
    /// Whether the number already uses floating-point syntax.
    is_float: bool,
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

/// Return whether the current cursor position may begin one number literal.
pub(crate) fn number_can_start(cursor: &LineCursor<'_>, pattern: NumberPattern) -> bool {
    let Some(ch) = cursor.peek() else {
        return false;
    };

    if pattern.allow_decimal_integer && ch.is_ascii_digit() {
        return true;
    }
    if pattern.allow_leading_dot && ch == '.' {
        return leading_dot_can_start(cursor);
    }

    // Leading signs are only valid in expression slots that clearly permit a
    // fresh literal, which keeps subtraction and compound operators plain.
    pattern.allow_leading_sign
        && matches!(ch, '+' | '-')
        && sign_can_start_number(cursor.prev())
        && match (cursor.peek_second(), cursor.peek_third()) {
            (Some(next), _) if next.is_ascii_digit() => true,
            (Some('.'), Some(third)) if pattern.allow_leading_dot && third.is_ascii_digit() => true,
            _ => false,
        }
}

/// Advance over one identifier-like token.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the start of the identifier.
/// - `pattern`: Identifier character rules for the active language.
pub(crate) fn consume_identifier(cursor: &mut LineCursor<'_>, pattern: IdentifierPattern) {
    cursor.advance_while(|ch| identifier_can_continue(pattern, ch));
}

/// Advance over one numeric-looking token.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the start of the number token.
/// - `pattern`: Number grammar for the active language.
pub(crate) fn consume_number(cursor: &mut LineCursor<'_>, pattern: NumberPattern) {
    if pattern.allow_leading_sign
        && cursor.peek().is_some_and(|ch| matches!(ch, '+' | '-'))
        && sign_can_start_number(cursor.prev())
    {
        cursor.advance_char();
    }
    let after_sign = cursor.clone();

    // Radix-prefixed numbers need to run before decimal parsing so `0xff` and
    // friends are recognized as one token without letting invalid prefixes leak.
    if let Some(core) = consume_prefixed_number(cursor, pattern) {
        consume_number_suffix(cursor, pattern, core);
        return;
    }
    *cursor = after_sign.clone();

    if let Some(core) = consume_legacy_octal_number(cursor, pattern) {
        consume_number_suffix(cursor, pattern, core);
        return;
    }
    *cursor = after_sign.clone();

    if let Some(core) = consume_decimal_number(cursor, pattern) {
        consume_number_suffix(cursor, pattern, core);
        return;
    }

    *cursor = after_sign;
}

/// Return whether the previous character allows a leading sign to start a number.
fn sign_can_start_number(prev: Option<char>) -> bool {
    prev.is_none_or(|ch| {
        ch.is_ascii_whitespace() || matches!(ch, '=' | ':' | '[' | '(' | '{' | ',')
    })
}

/// Return whether a leading decimal point can begin one number token here.
fn leading_dot_can_start(cursor: &LineCursor<'_>) -> bool {
    cursor.peek() == Some('.')
        && cursor
            .peek_second()
            .is_some_and(|next| next.is_ascii_digit())
        && cursor
            .prev()
            .is_none_or(|prev| !prev.is_ascii_alphanumeric() && !matches!(prev, '_' | '.'))
}

/// Return the separator character used by `separator`, if any.
fn separator_char(separator: DigitSeparator) -> Option<char> {
    match separator {
        DigitSeparator::None => None,
        DigitSeparator::Underscore => Some('_'),
        DigitSeparator::Apostrophe => Some('\''),
    }
}

/// Try to consume one radix-prefixed number.
fn consume_prefixed_number(
    cursor: &mut LineCursor<'_>,
    pattern: NumberPattern,
) -> Option<ConsumedNumberCore> {
    if pattern.allow_hex && (cursor.starts_with("0x") || cursor.starts_with("0X")) {
        return consume_hex_number(cursor, pattern);
    }
    if pattern.allow_binary && (cursor.starts_with("0b") || cursor.starts_with("0B")) {
        return consume_simple_prefixed_number(
            cursor,
            pattern.digit_separator,
            "0b",
            is_binary_digit,
        )
        .or_else(|| {
            consume_simple_prefixed_number(cursor, pattern.digit_separator, "0B", is_binary_digit)
        });
    }
    if pattern.allow_octal_prefix && (cursor.starts_with("0o") || cursor.starts_with("0O")) {
        return consume_simple_prefixed_number(
            cursor,
            pattern.digit_separator,
            "0o",
            is_octal_digit,
        )
        .or_else(|| {
            consume_simple_prefixed_number(cursor, pattern.digit_separator, "0O", is_octal_digit)
        });
    }
    None
}

/// Try to consume one binary or octal prefixed number.
fn consume_simple_prefixed_number(
    cursor: &mut LineCursor<'_>,
    separator: DigitSeparator,
    prefix: &str,
    is_digit: fn(char) -> bool,
) -> Option<ConsumedNumberCore> {
    let checkpoint = cursor.clone();
    cursor.advance_if_starts_with(prefix);
    if consume_digit_sequence(cursor, is_digit, separator) == 0 {
        *cursor = checkpoint;
        return None;
    }
    Some(ConsumedNumberCore { is_float: false })
}

/// Try to consume one hexadecimal integer or float.
fn consume_hex_number(
    cursor: &mut LineCursor<'_>,
    pattern: NumberPattern,
) -> Option<ConsumedNumberCore> {
    let checkpoint = cursor.clone();
    cursor.advance_char();
    cursor.advance_char();

    let digits_before = consume_digit_sequence(cursor, is_hex_digit, pattern.digit_separator);
    if digits_before == 0 && cursor.peek() != Some('.') {
        *cursor = checkpoint;
        return None;
    }

    let mut core = ConsumedNumberCore { is_float: false };
    let fraction_checkpoint = cursor.clone();

    // Hex floats only stay valid when a later `p` / `P` exponent appears, so
    // malformed fractional forms roll back to the integer token boundary.
    if pattern.allow_fraction && cursor.peek() == Some('.') && cursor.peek_second() != Some('.') {
        cursor.advance_char();
        let digits_after = consume_digit_sequence(cursor, is_hex_digit, pattern.digit_separator);
        if digits_before > 0 || digits_after > 0 {
            core.is_float = true;
        } else {
            *cursor = fraction_checkpoint.clone();
        }
    }

    let exponent_checkpoint = cursor.clone();
    if pattern.allow_hex_exponent && consume_exponent(cursor, "pP") {
        core.is_float = true;
        return Some(core);
    }

    if core.is_float {
        *cursor = fraction_checkpoint;
        core.is_float = false;
        return Some(core);
    }

    *cursor = exponent_checkpoint;
    Some(core)
}

/// Try to consume one legacy leading-zero octal number.
fn consume_legacy_octal_number(
    cursor: &mut LineCursor<'_>,
    pattern: NumberPattern,
) -> Option<ConsumedNumberCore> {
    if !pattern.allow_legacy_octal
        || cursor.peek() != Some('0')
        || !cursor.peek_second().is_some_and(is_octal_digit)
    {
        return None;
    }

    consume_digit_sequence(cursor, is_octal_digit, pattern.digit_separator);
    Some(ConsumedNumberCore { is_float: false })
}

/// Try to consume one decimal integer or float.
fn consume_decimal_number(
    cursor: &mut LineCursor<'_>,
    pattern: NumberPattern,
) -> Option<ConsumedNumberCore> {
    let checkpoint = cursor.clone();
    let mut core = ConsumedNumberCore { is_float: false };

    // Decimal parsing supports both ordinary leading digits and `.5`-style
    // literals when the active language opts into that grammar.
    if cursor.peek() == Some('.') {
        if !pattern.allow_leading_dot || !leading_dot_can_start(cursor) {
            return None;
        }
        cursor.advance_char();
        if consume_digit_sequence(cursor, is_decimal_digit, pattern.digit_separator) == 0 {
            *cursor = checkpoint;
            return None;
        }
        core.is_float = true;
    } else {
        if consume_digit_sequence(cursor, is_decimal_digit, pattern.digit_separator) == 0 {
            return None;
        }
        let fraction_checkpoint = cursor.clone();
        // Only keep a trailing `.` when the grammar allows a fractional part or
        // explicitly accepts whole-number literals such as `1.`.
        if pattern.allow_fraction && cursor.peek() == Some('.') && cursor.peek_second() != Some('.')
        {
            cursor.advance_char();
            let digits_after =
                consume_digit_sequence(cursor, is_decimal_digit, pattern.digit_separator);
            if digits_after > 0 || pattern.allow_trailing_dot {
                core.is_float = true;
            } else {
                *cursor = fraction_checkpoint;
            }
        }
    }

    let exponent_checkpoint = cursor.clone();
    // Exponents stay part of the token only when the full marker/sign/digits
    // sequence succeeds; otherwise the decimal core rolls back unchanged.
    if pattern.allow_decimal_exponent && consume_exponent(cursor, "eE") {
        core.is_float = true;
    } else {
        *cursor = exponent_checkpoint;
    }

    Some(core)
}

/// Consume one exponent marker with an optional sign and required digits.
///
/// # Returns
/// - `true` when a complete exponent is consumed and `cursor` advances past it.
/// - `false` when the exponent is malformed and `cursor` is restored.
fn consume_exponent(cursor: &mut LineCursor<'_>, markers: &str) -> bool {
    let checkpoint = cursor.clone();
    let Some(marker) = cursor.peek() else {
        return false;
    };
    if !markers.contains(marker) {
        return false;
    }

    cursor.advance_char();
    if cursor.peek().is_some_and(|ch| matches!(ch, '+' | '-')) {
        cursor.advance_char();
    }
    if consume_digit_sequence(cursor, is_decimal_digit, DigitSeparator::None) == 0 {
        *cursor = checkpoint;
        return false;
    }
    true
}

/// Consume the suffix pattern allowed after `core`.
fn consume_number_suffix(
    cursor: &mut LineCursor<'_>,
    pattern: NumberPattern,
    core: ConsumedNumberCore,
) {
    let (exact, groups) = if core.is_float {
        (
            pattern.suffix_pattern.float_exact,
            pattern.suffix_pattern.float_groups,
        )
    } else {
        (
            pattern.suffix_pattern.integer_exact,
            pattern.suffix_pattern.integer_groups,
        )
    };
    if !consume_exact_suffix(cursor, exact) {
        consume_suffix_groups(cursor, groups);
    }
}

/// Consume the longest exact suffix from `suffixes`, if any.
fn consume_exact_suffix(cursor: &mut LineCursor<'_>, suffixes: &[&str]) -> bool {
    if let Some((_, probe)) = suffixes
        .iter()
        .filter_map(|suffix| {
            let mut probe = cursor.clone();
            probe
                .advance_if_starts_with(suffix)
                .then_some((suffix, probe))
        })
        .max_by_key(|(suffix, _)| suffix.len())
    {
        *cursor = probe;
        return true;
    }
    false
}

/// Consume one configurable sequence of optional suffix groups.
fn consume_suffix_groups(cursor: &mut LineCursor<'_>, groups: &[NumberSuffixGroup]) {
    debug_assert!(groups.len() <= u64::BITS as usize);
    let mut consumed_groups = 0u64;

    // Each group may appear at most once, so the scan repeatedly picks the
    // longest matching unused group until no full suffix remains.
    loop {
        let mut match_index = None;
        let mut match_suffix: Option<&str> = None;
        for (index, group) in groups.iter().enumerate() {
            if consumed_groups & (1u64 << index) != 0 {
                continue;
            }
            let Some(suffix) = group
                .spellings
                .iter()
                .filter(|suffix| cursor.starts_with(suffix))
                .max_by_key(|suffix| suffix.len())
            else {
                continue;
            };
            if match_suffix.is_none_or(|current| suffix.len() > current.len()) {
                match_index = Some(index);
                match_suffix = Some(*suffix);
            }
        }

        let (Some(index), Some(suffix)) = (match_index, match_suffix) else {
            return;
        };
        cursor.advance_if_starts_with(suffix);
        consumed_groups |= 1u64 << index;
    }
}

/// Consume one digit sequence with exact separator placement rules.
fn consume_digit_sequence(
    cursor: &mut LineCursor<'_>,
    is_digit: fn(char) -> bool,
    separator: DigitSeparator,
) -> usize {
    let mut digits = consume_plain_digits(cursor, is_digit);
    let Some(separator) = separator_char(separator) else {
        return digits;
    };

    // Separators are only accepted between digit runs, which prevents malformed
    // forms such as `1__2`, `_1`, or `1_` from being over-highlighted.
    loop {
        let checkpoint = cursor.clone();
        if cursor.peek() != Some(separator) {
            return digits;
        }
        cursor.advance_char();
        let consumed = consume_plain_digits(cursor, is_digit);
        if consumed == 0 {
            *cursor = checkpoint;
            return digits;
        }
        digits += consumed;
    }
}

/// Consume one uninterrupted run of digits accepted by `is_digit`.
///
/// # Returns
/// - The number of consecutive digits consumed from the current cursor position.
fn consume_plain_digits(cursor: &mut LineCursor<'_>, is_digit: fn(char) -> bool) -> usize {
    let start = cursor.col();
    cursor.advance_while(is_digit);
    cursor.col() - start
}

/// Return whether `ch` is a decimal digit.
fn is_decimal_digit(ch: char) -> bool {
    ch.is_ascii_digit()
}

/// Return whether `ch` is a binary digit.
fn is_binary_digit(ch: char) -> bool {
    matches!(ch, '0' | '1')
}

/// Return whether `ch` is an octal digit.
fn is_octal_digit(ch: char) -> bool {
    matches!(ch, '0'..='7')
}

/// Return whether `ch` is a hexadecimal digit.
fn is_hex_digit(ch: char) -> bool {
    ch.is_ascii_hexdigit()
}

#[cfg(test)]
mod tests {
    use super::{LineCursor, consume_number, number_can_start};
    use crate::syntax::profile::SIGNED_NUMBER;
    use crate::syntax::profiles::{
        cpp::NUMBER_PATTERN as CPP_NUMBER, javascript::NUMBER_PATTERN as JAVASCRIPT_NUMBER,
        rust::NUMBER_PATTERN as RUST_NUMBER,
    };

    /// Verify that the shared cursor can peek ahead without consuming characters.
    #[test]
    fn test_line_cursor_fixed_peeks_preserve_position() {
        let cursor = LineCursor::new("aé🙂z");
        assert_eq!(cursor.peek(), Some('a'));
        assert_eq!(cursor.peek_second(), Some('é'));
        assert_eq!(cursor.peek_third(), Some('🙂'));
        assert_eq!(cursor.peek(), Some('a'));
        assert_eq!(cursor.col(), 0);
    }

    /// Verify that signed numbers still require a digit after the sign.
    #[test]
    fn test_number_can_start_rejects_bare_sign() {
        let cursor = LineCursor::new("+=");
        assert!(!number_can_start(&cursor, SIGNED_NUMBER));
    }

    /// Verify that subtraction sites do not begin a signed literal.
    #[test]
    fn test_number_can_start_rejects_sign_after_digit() {
        let mut cursor = LineCursor::new("1-2");
        cursor.advance_char();
        assert!(!number_can_start(&cursor, SIGNED_NUMBER));
    }

    /// Verify that incomplete exponents roll back to the token boundary.
    #[test]
    fn test_consume_number_rolls_back_incomplete_exponent() {
        let mut cursor = LineCursor::new("1e+ tail");
        consume_number(&mut cursor, JAVASCRIPT_NUMBER);
        assert_eq!(cursor.col(), 1);
        assert_eq!(cursor.remaining(), "e+ tail");
    }

    /// Verify that malformed separators stop before the invalid region.
    #[test]
    fn test_consume_number_rejects_repeated_separators() {
        let mut cursor = LineCursor::new("1__2 tail");
        consume_number(&mut cursor, JAVASCRIPT_NUMBER);
        assert_eq!(cursor.col(), 1);
        assert_eq!(cursor.remaining(), "__2 tail");
    }

    /// Verify that JavaScript BigInt suffixes remain part of the token.
    #[test]
    fn test_consume_number_keeps_javascript_bigint_suffix() {
        let mut cursor = LineCursor::new("123n;");
        consume_number(&mut cursor, JAVASCRIPT_NUMBER);
        assert_eq!(cursor.col(), 4);
        assert_eq!(cursor.remaining(), ";");
    }

    /// Verify that Rust integer suffixes remain part of the token.
    #[test]
    fn test_consume_number_keeps_rust_suffix() {
        let mut cursor = LineCursor::new("42usize;");
        consume_number(&mut cursor, RUST_NUMBER);
        assert_eq!(cursor.col(), 7);
        assert_eq!(cursor.remaining(), ";");
    }

    /// Verify that leading-dot decimals are accepted when the grammar allows them.
    #[test]
    fn test_consume_number_accepts_leading_dot_decimal() {
        let mut cursor = LineCursor::new(".5 + rest");
        consume_number(&mut cursor, JAVASCRIPT_NUMBER);
        assert_eq!(cursor.col(), 2);
        assert_eq!(cursor.remaining(), " + rest");
    }

    /// Verify that malformed hexadecimal floats fall back to the integer core.
    #[test]
    fn test_consume_number_rolls_back_invalid_hex_float_without_exponent() {
        let mut cursor = LineCursor::new("0x1.fp tail");
        consume_number(&mut cursor, CPP_NUMBER);
        assert_eq!(cursor.col(), 3);
        assert_eq!(cursor.remaining(), ".fp tail");
    }

    /// Verify that valid hexadecimal floats keep their exponent and suffix.
    #[test]
    fn test_consume_number_accepts_cpp_hex_float_suffix() {
        let mut cursor = LineCursor::new("0x1.fp2f;");
        consume_number(&mut cursor, CPP_NUMBER);
        assert_eq!(cursor.col(), 8);
        assert_eq!(cursor.remaining(), ";");
    }
}
