//! Inline markup span scanning.

use crate::syntax::engine::HighlightSpan;
use crate::syntax::helpers::{previous_char, starts_with};
use crate::syntax::profile::{SpanStyle, SyntaxClass, SyntaxModifier};

/// Collect conservative inline markup spans for one line.
pub(super) fn push_inline_markup_spans(chars: &[char], spans: &mut Vec<HighlightSpan>) {
    let mut idx = 0;

    // Unsupported or ambiguous constructs stay plain, while unmistakable inline
    // runs get semantic markup spans.
    while idx < chars.len() {
        if chars[idx] == '`'
            && let Some(end) = find_inline_code(chars, idx)
        {
            spans.push(HighlightSpan::styled(
                idx,
                end,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::InlineCode)),
            ));
            idx = end;
            continue;
        }
        if let Some(end) = find_link(chars, idx) {
            spans.push(HighlightSpan::styled(
                idx,
                end,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Link)),
            ));
            idx = end;
            continue;
        }
        if let Some(end) = find_markup_delimited_span(chars, idx, "**")
            .or_else(|| find_markup_delimited_span(chars, idx, "__"))
        {
            spans.push(HighlightSpan::styled(
                idx,
                end,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Strong)),
            ));
            idx = end;
            continue;
        }
        if let Some(end) = find_markup_delimited_span(chars, idx, "*")
            .or_else(|| find_markup_delimited_span(chars, idx, "_"))
        {
            spans.push(HighlightSpan::styled(
                idx,
                end,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Emphasis)),
            ));
            idx = end;
            continue;
        }
        idx += 1;
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

/// Find a simple same-delimiter markup span and return the closing index.
///
/// # Parameters
/// - `chars`: Full line as a character slice.
/// - `start`: Column where the opening delimiter begins.
/// - `delimiter`: Delimiter text to match on both sides of the span.
fn find_markup_delimited_span(chars: &[char], start: usize, delimiter: &str) -> Option<usize> {
    let delimiter_len = delimiter.chars().count();
    let next = chars.get(start + delimiter_len).copied();
    if !markup_can_open(previous_char(chars, start), next) {
        return None;
    }

    let mut idx = start + delimiter_len;
    // Markup emphasis stays conservative: only matching delimiters with valid
    // closing context become spans, otherwise the text stays plain.
    while idx + delimiter_len <= chars.len() {
        if starts_with(chars, idx, delimiter)
            && markup_can_close(
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

/// Find a one-line inline-code span and return its exclusive end column.
///
/// # Parameters
/// - `chars`: Full line as a character slice.
/// - `start`: Column where the opening backtick begins.
fn find_inline_code(chars: &[char], start: usize) -> Option<usize> {
    let end = chars[start + 1..]
        .iter()
        .position(|&ch| ch == '`')
        .map(|offset| start + 1 + offset + 1)?;
    (end > start + 2).then_some(end)
}

/// Find a simple inline link or image span.
///
/// # Parameters
/// - `chars`: Full line as a character slice.
/// - `start`: Column where the candidate link or image begins.
fn find_link(chars: &[char], start: usize) -> Option<usize> {
    let offset = usize::from(chars.get(start) == Some(&'!'));
    if chars.get(start + offset) != Some(&'[') {
        return None;
    }

    // The shared markup lexer recognizes only one-line inline links and images,
    // so nested labels or reference-style links stay plain and conservative.
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
