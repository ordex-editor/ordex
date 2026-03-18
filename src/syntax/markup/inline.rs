//! Inline markup span scanning.

use crate::syntax::engine::HighlightSpan;
use crate::syntax::helpers::LineCursor;
use crate::syntax::profile::{
    InlineBalancedPairRule, InlineBracketLinkRule, InlineDelimitedMarkupRule,
    InlineDelimiterBoundary, InlinePrefixedBracketSpanRule, MarkupRules, SpanStyle, SyntaxClass,
    SyntaxModifier,
};

/// Collect conservative inline markup spans for one line.
pub(super) fn push_inline_markup_spans(
    line: &str,
    rules: MarkupRules,
    spans: &mut Vec<HighlightSpan>,
) {
    let mut cursor = LineCursor::new(line);

    // Unsupported or ambiguous constructs stay plain, while unmistakable inline
    // runs get semantic markup spans.
    while !cursor.is_empty() {
        let start_col = cursor.col();

        if let Some((end, style)) =
            find_inline_delimited_span(&cursor, rules.inline_delimited_rules)
        {
            spans.push(HighlightSpan::styled(start_col, end.col(), style));
            cursor = end;
            continue;
        }
        if let Some(end) = find_bracket_link_span(&cursor, rules.inline_bracket_links) {
            spans.push(HighlightSpan::styled(
                start_col,
                end.col(),
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Link)),
            ));
            cursor = end;
            continue;
        }
        if let Some(end) = find_prefixed_bracket_span(&cursor, rules.inline_prefixed_bracket_spans)
            .or_else(|| find_balanced_pair_span(&cursor, rules.inline_balanced_pair_spans))
        {
            spans.push(HighlightSpan::styled(
                start_col,
                end.col(),
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Link)),
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

/// Find the first matching configured inline delimited span.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rules`: Candidate delimited inline rules checked in order.
fn find_inline_delimited_span<'a>(
    cursor: &LineCursor<'a>,
    rules: &[InlineDelimitedMarkupRule],
) -> Option<(LineCursor<'a>, SpanStyle)> {
    for rule in rules {
        if let Some(end) = find_delimited_span(cursor, *rule) {
            return Some((
                end,
                SpanStyle::new(SyntaxClass::Markup, Some(rule.modifier)),
            ));
        }
    }
    None
}

/// Find one delimited inline span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the opening delimiter.
/// - `rule`: Delimited inline rule to test.
fn find_delimited_span<'a>(
    cursor: &LineCursor<'a>,
    rule: InlineDelimitedMarkupRule,
) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    if !end.advance_if_starts_with(rule.delimiter) {
        return None;
    }
    if rule.boundary == InlineDelimiterBoundary::EmphasisLike
        && !markup_can_open(cursor.prev(), end.peek())
    {
        return None;
    }

    // Delimited inline constructs stay conservative: only balanced one-line
    // spans with valid closing context become semantic markup.
    while !end.is_empty() {
        let mut close = end.clone();
        if close.advance_if_starts_with(rule.delimiter) {
            let boundary_ok = rule.boundary == InlineDelimiterBoundary::None
                || markup_can_close(end.prev(), close.peek());
            if boundary_ok && close.col().saturating_sub(cursor.col()) >= rule.min_span_len {
                return Some(close);
            }
        }
        end.advance_char();
    }
    None
}

/// Find the first matching configured bracketed link span.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rules`: Candidate bracketed link rules checked in order.
fn find_bracket_link_span<'a>(
    cursor: &LineCursor<'a>,
    rules: &[InlineBracketLinkRule],
) -> Option<LineCursor<'a>> {
    for rule in rules {
        if let Some(end) = find_one_bracket_link_span(cursor, *rule) {
            return Some(end);
        }
    }
    None
}

/// Find one bracketed link span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rule`: Bracketed link rule to test.
fn find_one_bracket_link_span<'a>(
    cursor: &LineCursor<'a>,
    rule: InlineBracketLinkRule,
) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    if !end.advance_if_starts_with(rule.opener) {
        return None;
    }

    // The shared markup lexer recognizes only one-line bracketed links, so
    // nested labels stay plain and conservative.
    while let Some(ch) = end.advance_char() {
        if ch == rule.label_close {
            break;
        }
    }
    if end.prev() != Some(rule.label_close) || end.peek() != Some(rule.target_open) {
        return None;
    }
    end.advance_char();
    while let Some(ch) = end.advance_char() {
        if ch == rule.target_close {
            return Some(end);
        }
    }
    None
}

/// Find the first matching configured prefixed bracket span.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rules`: Candidate prefixed bracket rules checked in order.
fn find_prefixed_bracket_span<'a>(
    cursor: &LineCursor<'a>,
    rules: &[InlinePrefixedBracketSpanRule],
) -> Option<LineCursor<'a>> {
    for rule in rules {
        if let Some(end) = find_one_prefixed_bracket_span(cursor, *rule) {
            return Some(end);
        }
    }
    None
}

/// Find one prefixed bracket span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rule`: Prefixed bracket rule to test.
fn find_one_prefixed_bracket_span<'a>(
    cursor: &LineCursor<'a>,
    rule: InlinePrefixedBracketSpanRule,
) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    let mut matched_prefix = false;
    for prefix in rule.prefixes {
        let mut probe = cursor.clone();
        if probe.advance_if_starts_with(prefix) {
            end = probe;
            matched_prefix = true;
            break;
        }
    }
    if !matched_prefix {
        return None;
    }

    // Prefixed bracket spans keep the target simple: scan to the first bracket
    // opener without crossing whitespace so malformed constructs stay plain.
    while let Some(ch) = end.peek() {
        if ch == rule.bracket_open {
            end.advance_char();
            while let Some(ch) = end.advance_char() {
                if ch == rule.bracket_close {
                    return Some(end);
                }
            }
            return None;
        }
        if ch.is_whitespace() {
            return None;
        }
        end.advance_char();
    }
    None
}

/// Find the first matching configured balanced-pair span.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rules`: Candidate balanced-pair rules checked in order.
fn find_balanced_pair_span<'a>(
    cursor: &LineCursor<'a>,
    rules: &[InlineBalancedPairRule],
) -> Option<LineCursor<'a>> {
    for rule in rules {
        if let Some(end) = find_one_balanced_pair_span(cursor, *rule) {
            return Some(end);
        }
    }
    None
}

/// Find one balanced-pair span and return the ending cursor.
///
/// # Parameters
/// - `cursor`: Cursor positioned at the candidate opener.
/// - `rule`: Balanced-pair rule to test.
fn find_one_balanced_pair_span<'a>(
    cursor: &LineCursor<'a>,
    rule: InlineBalancedPairRule,
) -> Option<LineCursor<'a>> {
    let mut end = cursor.clone();
    if !end.advance_if_starts_with(rule.open) {
        return None;
    }

    while !end.is_empty() {
        if end.advance_if_starts_with(rule.close) {
            return Some(end);
        }
        end.advance_char();
    }
    None
}
