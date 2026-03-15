//! Shared syntax helper predicates.
//!
//! These helpers keep the generic lexer and profile modules focused on behavior
//! instead of repeating low-level character scanning logic.

use std::str::Chars;

use crate::syntax::profile::{IdentifierCharSet, IdentifierPattern, NumberPattern};

/// One saved cursor position inside a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorMark {
    /// Byte offset into the original line.
    byte_pos: usize,
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
    #[cfg(test)]
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

/// Return whether the current cursor position may begin a generic number literal.
pub(crate) fn number_can_start(cursor: &LineCursor<'_>, pattern: NumberPattern) -> bool {
    let Some(ch) = cursor.peek() else {
        return false;
    };

    // Signed numbers only accept `+` or `-` when a digit immediately follows so
    // punctuation like `->` and `+=` does not become part of a number token.
    ch.is_ascii_digit()
        || (pattern.allow_leading_sign
            && matches!(ch, '+' | '-')
            && cursor
                .peek_second()
                .is_some_and(|next| next.is_ascii_digit()))
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
pub(crate) fn consume_number(cursor: &mut LineCursor<'_>) {
    if cursor.peek().is_some_and(|ch| matches!(ch, '+' | '-')) {
        cursor.advance_char();
    }

    // Radix-prefixed literals use one dedicated digit class each so `0xff` and
    // friends stay highlighted without letting unrelated identifiers attach.
    if cursor.starts_with("0x") || cursor.starts_with("0X") {
        cursor.advance_char();
        cursor.advance_char();
        consume_prefixed_digits(cursor, |ch| ch.is_ascii_hexdigit());
        return;
    }
    if cursor.starts_with("0b") || cursor.starts_with("0B") {
        cursor.advance_char();
        cursor.advance_char();
        consume_prefixed_digits(cursor, |ch| matches!(ch, '0' | '1'));
        return;
    }
    if cursor.starts_with("0o") || cursor.starts_with("0O") {
        cursor.advance_char();
        cursor.advance_char();
        consume_prefixed_digits(cursor, |ch| matches!(ch, '0'..='7'));
        return;
    }

    // Decimal numbers accept one fractional part only when the dot is followed
    // by another digit, which keeps ranges like `0..count` from swallowing the
    // identifier after the range operator.
    consume_digit_run(cursor);
    if cursor.peek() == Some('.')
        && cursor
            .peek_second()
            .is_some_and(|next| next.is_ascii_digit())
    {
        cursor.advance_char();
        consume_digit_run(cursor);
    }

    // Exponents require at least one digit and may carry one sign, so partial
    // constructs like `1e+` stop before the `e` instead of over-highlighting.
    if cursor.peek().is_some_and(|ch| matches!(ch, 'e' | 'E')) {
        let checkpoint = cursor.clone();
        cursor.advance_char();
        if cursor.peek().is_some_and(|ch| matches!(ch, '+' | '-')) {
            cursor.advance_char();
        }
        let exponent_start = cursor.col();
        consume_digit_run(cursor);
        if cursor.col() == exponent_start {
            *cursor = checkpoint;
        }
    }
}

/// Return the byte index that corresponds to `char_idx` inside `text`.
pub(crate) fn byte_index_for_char(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

/// Advance over one decimal digit-or-underscore run.
fn consume_digit_run(cursor: &mut LineCursor<'_>) {
    cursor.advance_while(|ch| ch.is_ascii_digit() || ch == '_');
}

/// Advance over one radix-prefixed digit run.
fn consume_prefixed_digits(cursor: &mut LineCursor<'_>, is_digit: fn(char) -> bool) {
    cursor.advance_while(|ch| is_digit(ch) || ch == '_');
}

#[cfg(test)]
mod tests {
    use super::{LineCursor, consume_number};
    use crate::syntax::profile::SIGNED_NUMBER;

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
        assert!(!super::number_can_start(&cursor, SIGNED_NUMBER));
    }

    /// Verify that incomplete exponents roll back to the token boundary.
    #[test]
    fn test_consume_number_rolls_back_incomplete_exponent() {
        let mut cursor = LineCursor::new("1e+ tail");
        consume_number(&mut cursor);
        assert_eq!(cursor.col(), 1);
        assert_eq!(cursor.remaining(), "e+ tail");
    }
}
