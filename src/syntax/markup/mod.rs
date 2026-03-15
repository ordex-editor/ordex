//! Markup-oriented syntax lexing.
//!
//! This module owns the shared markup lexer so markup-like profiles can keep
//! their configuration data-only while reusing one conservative implementation.

use crate::syntax::engine::{HighlightSpan, LineLexMode, LineParseResult};
use crate::syntax::helpers::byte_index_for_char;
use crate::syntax::profile::{MarkupRules, SpanStyle, SyntaxClass, SyntaxModifier};

mod inline;

/// Lex one markup-like line from the supplied entry mode.
///
/// # Parameters
/// - `line`: Source line text with any trailing line break already removed.
/// - `entry_mode`: Multiline state inherited from the previous logical line.
/// - `rules`: Markup profile rules for headings, lists, and fences.
pub(crate) fn lex_markup_line(
    line: &str,
    entry_mode: LineLexMode,
    rules: MarkupRules,
) -> LineParseResult {
    let line_len = line.chars().count();
    let trimmed_start = leading_whitespace_len(line);
    let trimmed = &line[byte_index_for_char(line, trimmed_start)..];

    // Fence bodies stay intentionally simple: every line inside the fence keeps
    // one code-fence style until a closing fence is reached.
    if let LineLexMode::MarkupFence { marker, count } = entry_mode {
        let exit_mode = if fence_closes(trimmed, marker, count) {
            LineLexMode::Plain
        } else {
            LineLexMode::MarkupFence { marker, count }
        };
        return LineParseResult {
            spans: vec![HighlightSpan::styled(
                0,
                line_len,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::CodeFence)),
            )],
            exit_mode,
        };
    }

    if is_thematic_break(trimmed) {
        return LineParseResult {
            spans: vec![HighlightSpan::styled(
                0,
                line_len,
                SpanStyle::new(SyntaxClass::Markup, None),
            )],
            exit_mode: LineLexMode::Plain,
        };
    }

    if let Some((marker, count)) = fenced_marker(trimmed, rules.fence_markers) {
        return LineParseResult {
            spans: vec![HighlightSpan::styled(
                trimmed_start,
                line_len,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::CodeFence)),
            )],
            exit_mode: LineLexMode::MarkupFence { marker, count },
        };
    }

    if heading_prefix_len(trimmed).is_some() {
        return LineParseResult {
            spans: vec![HighlightSpan::styled(
                trimmed_start,
                line_len,
                SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Heading)),
            )],
            exit_mode: LineLexMode::Plain,
        };
    }

    let mut spans = Vec::new();
    if let Some(quote_len) = block_quote_prefix_len(trimmed) {
        spans.push(HighlightSpan::styled(
            trimmed_start,
            trimmed_start + quote_len,
            SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Quote)),
        ));
    } else if let Some(list_len) = list_marker_len(trimmed, rules.unordered_list_markers) {
        spans.push(HighlightSpan::styled(
            trimmed_start,
            trimmed_start + list_len,
            SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::ListMarker)),
        ));
    }

    inline::push_inline_markup_spans(line, &mut spans);
    LineParseResult {
        spans,
        exit_mode: LineLexMode::Plain,
    }
}

/// Return whether a fenced-code line closes the current markup fence.
///
/// # Parameters
/// - `text`: Trimmed line text beginning at the first non-whitespace column.
/// - `marker`: Fence marker character currently in effect.
/// - `count`: Minimum repeated marker count required to close the fence.
fn fence_closes(text: &str, marker: char, count: usize) -> bool {
    let trimmed_start = text.trim_start();
    if !trimmed_start.starts_with(marker) {
        return false;
    }
    let run = trimmed_start.chars().take_while(|&c| c == marker).count();
    run >= count
}

/// Count leading ASCII whitespace columns before the first non-space character.
fn leading_whitespace_len(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// Return an ordered-list marker length when `text` begins with one.
fn ordered_list_marker_len(text: &str) -> Option<usize> {
    let mut idx = 0;

    // Collect the leading digits first, then require the `". "` shape so plain
    // dotted numbers like version strings do not become list markers.
    while text
        .as_bytes()
        .get(idx)
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        idx += 1;
    }
    if idx == 0
        || text.as_bytes().get(idx) != Some(&b'.')
        || text.as_bytes().get(idx + 1) != Some(&b' ')
    {
        return None;
    }
    Some(idx + 2)
}

/// Return whether the trimmed line is an unmistakable thematic break.
fn is_thematic_break(text: &str) -> bool {
    let mut marker = None;
    let mut count = 0;

    // Ignore whitespace and require at least three identical marker characters.
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        match marker {
            None if matches!(ch, '-' | '*' | '_') => marker = Some(ch),
            Some(expected) if ch == expected => {}
            _ => return false,
        }
        count += 1;
    }

    count >= 3
}

/// Return fenced-code marker details from a trimmed line prefix.
///
/// # Parameters
/// - `text`: Trimmed line text beginning at the first non-whitespace column.
/// - `allowed_markers`: Marker characters permitted by the markup profile.
fn fenced_marker(text: &str, allowed_markers: &[char]) -> Option<(char, usize)> {
    let mut chars = text.chars();
    let marker = chars.next()?;
    if !allowed_markers.contains(&marker) {
        return None;
    }
    let count = 1 + chars.take_while(|&c| c == marker).count();
    (count >= 3).then_some((marker, count))
}

/// Return the heading-marker length for a simple ATX heading.
fn heading_prefix_len(text: &str) -> Option<usize> {
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
fn block_quote_prefix_len(text: &str) -> Option<usize> {
    if text.starts_with("> ") {
        Some(2)
    } else if text.starts_with('>') {
        Some(1)
    } else {
        None
    }
}

/// Return the list-marker length for a line.
fn list_marker_len(text: &str, unordered_markers: &[char]) -> Option<usize> {
    // Unordered markers win first because they are fixed-width; ordered markers
    // need a slightly more expensive digit scan.
    let mut chars = text.chars();
    if let (Some(marker), Some(' ')) = (chars.next(), chars.next())
        && unordered_markers.contains(&marker)
    {
        return Some(marker.len_utf8() + 1);
    }
    ordered_list_marker_len(text)
}
