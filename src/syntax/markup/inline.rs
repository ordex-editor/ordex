//! Inline markup span scanning.

use crate::syntax::engine::HighlightSpan;
use crate::syntax::helpers::LineCursor;
use crate::syntax::profile::{SpanStyle, SyntaxClass, SyntaxModifier};

/// Collect conservative inline markup spans for one line.
pub(super) fn push_inline_markup_spans(line: &str, spans: &mut Vec<HighlightSpan>) {
    let mut cursor = LineCursor::new(line);

    // Unsupported or ambiguous constructs stay plain, while unmistakable inline
    // runs get semantic markup spans.
    while !cursor.is_empty() {
        let start_col = cursor.col();

        if let Some(end) = find_inline_code(&cursor) {
            spans.push(HighlightSpan::styled(
                start_col,
                end.col(),
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::InlineCode)),
            ));
            cursor = end;
            continue;
        }
        if let Some(end) = find_link(&cursor) {
            spans.push(HighlightSpan::styled(
                start_col,
                end.col(),
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Link)),
            ));
            cursor = end;
            continue;
        }
        if let Some(end) = find_markup_delimited_span(&cursor, "**")
            .or_else(|| find_markup_delimited_span(&cursor, "__"))
        {
            spans.push(HighlightSpan::styled(
                start_col,
                end.col(),
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Strong)),
            ));
            cursor = end;
            continue;
        }
        if let Some(end) = find_markup_delimited_span(&cursor, "*")
            .or_else(|| find_markup_delimited_span(&cursor, "_"))
        {
            spans.push(HighlightSpan::styled(
                start_col,
                end.col(),
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Emphasis)),
            ));
            cursor = end;
            continue;
        }
        cursor.advance_char();
    }
}

/// Return whether a markup delimiter can open emphasis conservatively.
///
/// # Parameters
/// - `prev`: Character immediately before the delimiter, if any.
/// - `next`: Character immediately after the delimiter, if any.
fn markup_can_open(prev: Option<char>, next: Option<char>) -> bool {
    let Some(next) = next else {
        return false;
    };
    if next.is_whitespace() {
        return false;
    }
    !prev.is_some_and(|c| c.is_ascii_alphanumeric()) || !next.is_ascii_alphanumeric()
}

/// Return whether a markup delimiter can close emphasis conservatively.
///
/// # Parameters
/// - `prev`: Character immediately before the delimiter, if any.
/// - `next`: Character immediately after the delimiter, if any.
fn markup_can_close(prev: Option<char>, next: Option<char>) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    if prev.is_whitespace() {
        return false;
    }
    !next.is_some_and(|c| c.is_ascii_alphanumeric()) || !prev.is_ascii_alphanumeric()
}

/// Find a simple same-delimiter markup span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the opening delimiter.
/// - `delimiter`: Delimiter text to match on both sides of the span.
fn find_markup_delimited_span<'a>(
    cursor: &LineCursor<'a>,
    delimiter: &str,
) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    if !end.advance_if_starts_with(delimiter) {
        return None;
    }
    if !markup_can_open(cursor.prev(), end.peek()) {
        return None;
    }

    // Markup emphasis stays conservative: only matching delimiters with valid
    // closing context become spans, otherwise the text stays plain.
    while !end.is_empty() {
        if end.starts_with(delimiter) {
            let mut close = end.clone();
            close.advance_if_starts_with(delimiter);
            if markup_can_close(end.prev(), close.peek()) {
                return Some(close);
            }
        }
        end.advance_char();
    }
    None
}

/// Find a one-line inline-code span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the opening backtick.
fn find_inline_code<'a>(cursor: &LineCursor<'a>) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    if end.peek() != Some('`') {
        return None;
    }

    // Inline code stays one-line only, so scanning forward once is enough to
    // find the next backtick without materializing an intermediate char buffer.
    end.advance_char();
    while let Some(ch) = end.advance_char() {
        if ch == '`' {
            return (end.col() > cursor.col() + 2).then_some(end);
        }
    }
    None
}

/// Find a simple inline link or image span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate link or image.
fn find_link<'a>(cursor: &LineCursor<'a>) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    if end.starts_with("![") {
        end.advance_if_starts_with("![");
    } else if end.peek() == Some('[') {
        end.advance_char();
    } else {
        return None;
    }

    // The shared markup lexer recognizes only one-line inline links and images,
    // so nested labels or reference-style links stay plain and conservative.
    while let Some(ch) = end.advance_char() {
        if ch == ']' {
            break;
        }
    }
    if end.prev() != Some(']') || end.peek() != Some('(') {
        return None;
    }
    end.advance_char();
    while let Some(ch) = end.advance_char() {
        if ch == ')' {
            return Some(end);
        }
    }
    None
}
