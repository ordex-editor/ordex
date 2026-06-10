//! Markup-oriented syntax lexing.
//!
//! This module owns the shared markup lexer so markup-like profiles can keep
//! their configuration data-only while reusing one conservative implementation.

use crate::syntax::engine::{HighlightSpan, LineLexMode, LineParseResult};
use crate::syntax::helpers::LineCursor;
use crate::syntax::profile::{
    COMMENT_STYLE, LanguageProfile, MarkupHeadingRule, MarkupListRule, MarkupRules,
    MarkupThematicBreakRule, SpanStyle, SyntaxClass, SyntaxModifier,
};

mod inline;

/// Lex one markup-like line from the supplied entry mode.
///
/// # Parameters
/// - `line`: Source line text with any trailing line break already removed.
/// - `entry_mode`: Multiline state inherited from the previous logical line.
/// - `rules`: Markup profile rules for headings, lists, and fences.
pub(crate) fn lex_markup_line(
    _profile: &LanguageProfile,
    line: &str,
    entry_mode: LineLexMode,
    rules: MarkupRules,
) -> LineParseResult {
    let line_len = line.chars().count();
    let mut trim_cursor = LineCursor::new(line);
    trim_cursor.advance_while(|ch| ch.is_whitespace());
    let trimmed_start = trim_cursor.col();
    let trimmed = trim_cursor.remaining();

    // Fence bodies stay intentionally simple: every line inside the fence keeps
    // one code-fence style until a closing fence is reached.
    if let LineLexMode::MarkupFence {
        marker,
        count,
        style,
    } = entry_mode
    {
        let exit_mode = if fence_closes(trimmed, marker, count, rules.min_fence_len) {
            LineLexMode::Plain
        } else {
            LineLexMode::MarkupFence {
                marker,
                count,
                style,
            }
        };
        return LineParseResult {
            spans: vec![HighlightSpan::styled(0, line_len, style)],
            exit_mode,
        };
    }

    if is_thematic_break(trimmed, rules.thematic_break) {
        return LineParseResult {
            spans: vec![HighlightSpan::styled(
                0,
                line_len,
                SpanStyle::new(SyntaxClass::Markup, None),
            )],
            exit_mode: LineLexMode::Plain,
        };
    }

    if let Some((marker, count, style)) = fenced_marker(trimmed, rules) {
        return LineParseResult {
            spans: vec![HighlightSpan::styled(trimmed_start, line_len, style)],
            exit_mode: LineLexMode::MarkupFence {
                marker,
                count,
                style,
            },
        };
    }

    if heading_prefix_len(trimmed, rules.heading_rules).is_some() {
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
    if let Some(quote_len) = block_quote_prefix_len(trimmed, rules.block_quote_prefixes) {
        spans.push(HighlightSpan::styled(
            trimmed_start,
            trimmed_start + quote_len,
            SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::Quote)),
        ));
    } else if let Some(list_len) = list_marker_len(trimmed, rules.list_rules) {
        spans.push(HighlightSpan::styled(
            trimmed_start,
            trimmed_start + list_len,
            SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::ListMarker)),
        ));
    }

    inline::push_inline_markup_spans(line, rules, &mut spans);
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
/// - `min_fence_len`: Smallest repeated marker count that can form a fence.
fn fence_closes(text: &str, marker: char, count: usize, min_fence_len: usize) -> bool {
    let trimmed_start = text.trim_start();
    if !trimmed_start.starts_with(marker) {
        return false;
    }
    let run = trimmed_start.chars().take_while(|&c| c == marker).count();
    run >= count.max(min_fence_len)
        // Reject any line with non-whitespace content after the marker run.
        // In AsciiDoc, a fenced block delimiter line must consist solely of repeated markers.
        && trimmed_start[run..].chars().all(|c| c.is_whitespace())
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
fn is_thematic_break(text: &str, rule: Option<MarkupThematicBreakRule>) -> bool {
    let Some(rule) = rule else {
        return false;
    };
    let mut marker = None;
    let mut count = 0;

    // Ignore whitespace and require at least three identical marker characters.
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        match marker {
            None if rule.markers.contains(&ch) => marker = Some(ch),
            Some(expected) if ch == expected => {}
            _ => return false,
        }
        count += 1;
    }

    count >= rule.min_repeat
}

/// Return fenced-code marker details from a trimmed line prefix.
///
/// # Parameters
/// - `text`: Trimmed line text beginning at the first non-whitespace column.
/// - `rules`: Markup profile rules for delimited blocks.
fn fenced_marker(text: &str, rules: MarkupRules) -> Option<(char, usize, SpanStyle)> {
    let mut chars = text.chars();
    let marker = chars.next()?;
    if !rules.fence_markers.contains(&marker) {
        return None;
    }
    let count = 1 + chars.by_ref().take_while(|&c| c == marker).count();
    if count < rules.min_fence_len {
        return None;
    }
    // Reject any line with non-whitespace content after the marker run.
    // In AsciiDoc, a fenced block delimiter line must consist solely of repeated markers.
    if chars.any(|c| !c.is_whitespace()) {
        return None;
    }

    let style = if rules.comment_fence_markers.contains(&marker) {
        COMMENT_STYLE
    } else {
        SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::CodeFence))
    };
    Some((marker, count, style))
}

/// Return the heading-marker length for a simple ATX heading.
fn heading_prefix_len(text: &str, rules: &[MarkupHeadingRule]) -> Option<usize> {
    for rule in rules {
        let repeat = text.chars().take_while(|&c| c == rule.marker).count();
        if repeat < rule.min_repeat || repeat > rule.max_repeat {
            continue;
        }
        if text.chars().nth(repeat).is_some_and(|c| c == ' ') {
            return Some(text.chars().count());
        }
    }
    None
}

/// Return the block-quote marker length for a line.
fn block_quote_prefix_len(text: &str, prefixes: &[&str]) -> Option<usize> {
    for prefix in prefixes {
        if text.starts_with(prefix) {
            return Some(prefix.chars().count());
        }
    }
    None
}

/// Return the list-marker length for a line.
fn list_marker_len(text: &str, rules: &[MarkupListRule]) -> Option<usize> {
    for rule in rules {
        match rule {
            MarkupListRule::RepeatedMarker { marker, min_repeat } => {
                if let Some(list_len) = repeated_marker_list_len(text, *marker, *min_repeat) {
                    return Some(list_len);
                }
            }
            MarkupListRule::DecimalDot => {
                if let Some(list_len) = ordered_list_marker_len(text) {
                    return Some(list_len);
                }
            }
        }
    }
    None
}

/// Return the list-marker length for one repeated-marker list rule.
fn repeated_marker_list_len(text: &str, marker: char, min_repeat: usize) -> Option<usize> {
    let mut chars = text.chars();
    if chars.next()? != marker {
        return None;
    }
    let run = 1 + chars.take_while(|&ch| ch == marker).count();
    if run < min_repeat {
        return None;
    }
    text.chars()
        .nth(run)
        .is_some_and(|ch| ch == ' ')
        .then_some(run + 1)
}
