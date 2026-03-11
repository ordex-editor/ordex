//! Conservative-core Markdown profile and lexing rules.

use crate::syntax::engine::{HighlightSpan, LineLexMode, LineParseResult};
use crate::syntax::helpers::{
    fenced_marker, find_markdown_delimited_span, is_thematic_break, leading_whitespace_len,
    ordered_list_marker_len, starts_with,
};
use crate::syntax::profile::{
    LanguageDetection, LanguageId, LanguageProfile, NestedLanguageHook, SyntaxClass, SyntaxModifier,
};

const NESTED_HOOKS: &[NestedLanguageHook] = &[];

/// Static Markdown language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Markdown,
    display_name: "Markdown",
    detection: LanguageDetection {
        exact_filenames: &["README.md"],
        extensions: &["md", "markdown"],
    },
    comment_styles: &[],
    nested_hooks: NESTED_HOOKS,
    lex_line: lex_markdown_line,
};

/// Lex one Markdown line from the supplied entry mode.
pub(crate) fn lex_markdown_line(line: &str, entry_mode: LineLexMode) -> LineParseResult {
    let chars: Vec<char> = line.chars().collect();
    let trimmed_start = leading_whitespace_len(line);
    let trimmed = &line[byte_index_for_char(line, trimmed_start)..];

    // Fence body lines stay deliberately simple in phase 1: the whole line keeps
    // one code-fence style until a matching closing fence is reached.
    if let LineLexMode::MarkdownFence { marker, count } = entry_mode {
        let exit_mode = if fence_closes(trimmed, marker, count) {
            LineLexMode::Plain
        } else {
            LineLexMode::MarkdownFence { marker, count }
        };
        return LineParseResult {
            spans: vec![markup_span(0, chars.len(), Some(SyntaxModifier::CodeFence))],
            exit_mode,
        };
    }

    if is_thematic_break(trimmed) {
        return LineParseResult {
            spans: vec![markup_span(0, chars.len(), None)],
            exit_mode: LineLexMode::Plain,
        };
    }

    if let Some((marker, count)) = fenced_marker(trimmed) {
        return LineParseResult {
            spans: vec![markup_span(
                trimmed_start,
                chars.len(),
                Some(SyntaxModifier::CodeFence),
            )],
            exit_mode: LineLexMode::MarkdownFence { marker, count },
        };
    }

    if let Some(heading_end) = heading_prefix_len(trimmed) {
        return LineParseResult {
            spans: vec![markup_span(
                trimmed_start,
                trimmed_start + heading_end.max(chars.len().saturating_sub(trimmed_start)),
                Some(SyntaxModifier::Heading),
            )],
            exit_mode: LineLexMode::Plain,
        };
    }

    let mut spans = Vec::new();
    if let Some(quote_len) = block_quote_prefix_len(trimmed) {
        spans.push(markup_span(
            trimmed_start,
            trimmed_start + quote_len,
            Some(SyntaxModifier::Quote),
        ));
    } else if let Some(list_len) = list_marker_len(trimmed) {
        spans.push(markup_span(
            trimmed_start,
            trimmed_start + list_len,
            Some(SyntaxModifier::ListMarker),
        ));
    }

    spans.extend(inline_markdown_spans(&chars));
    LineParseResult {
        spans,
        exit_mode: LineLexMode::Plain,
    }
}

/// Return the closing state for a fenced-code line.
fn fence_closes(text: &str, marker: char, count: usize) -> bool {
    let trimmed_start = text.trim_start();
    if !trimmed_start.starts_with(marker) {
        return false;
    }
    let run = trimmed_start.chars().take_while(|&c| c == marker).count();
    run >= count
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
fn list_marker_len(text: &str) -> Option<usize> {
    if starts_with(&text.chars().collect::<Vec<_>>(), 0, "- ")
        || starts_with(&text.chars().collect::<Vec<_>>(), 0, "* ")
        || starts_with(&text.chars().collect::<Vec<_>>(), 0, "+ ")
    {
        return Some(2);
    }
    ordered_list_marker_len(text)
}

/// Collect conservative inline Markdown spans for one line.
fn inline_markdown_spans(chars: &[char]) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut idx = 0;
    while idx < chars.len() {
        if chars[idx] == '`'
            && let Some(end) = find_inline_code(chars, idx)
        {
            spans.push(markup_span(idx, end, Some(SyntaxModifier::InlineCode)));
            idx = end;
            continue;
        }
        if let Some(end) = find_link(chars, idx) {
            spans.push(markup_span(idx, end, Some(SyntaxModifier::Link)));
            idx = end;
            continue;
        }
        if let Some(end) = find_markdown_delimited_span(chars, idx, "**")
            .or_else(|| find_markdown_delimited_span(chars, idx, "__"))
        {
            spans.push(markup_span(idx, end, Some(SyntaxModifier::Strong)));
            idx = end;
            continue;
        }
        if let Some(end) = find_markdown_delimited_span(chars, idx, "*")
            .or_else(|| find_markdown_delimited_span(chars, idx, "_"))
        {
            spans.push(markup_span(idx, end, Some(SyntaxModifier::Emphasis)));
            idx = end;
            continue;
        }
        idx += 1;
    }
    spans
}

/// Find a one-line inline-code span and return its exclusive end column.
fn find_inline_code(chars: &[char], start: usize) -> Option<usize> {
    let end = chars[start + 1..]
        .iter()
        .position(|&ch| ch == '`')
        .map(|offset| start + 1 + offset + 1)?;
    (end > start + 2).then_some(end)
}

/// Find a simple inline link or image span.
fn find_link(chars: &[char], start: usize) -> Option<usize> {
    let offset = usize::from(chars.get(start) == Some(&'!'));
    if chars.get(start + offset) != Some(&'[') {
        return None;
    }

    // Phase 1 only supports one-line inline links/images without nested bracket
    // structures so unsupported variants naturally fall back to plain text.
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

/// Convert a character index into a byte index for `line`.
fn byte_index_for_char(line: &str, char_idx: usize) -> usize {
    line.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(line.len())
}

/// Build a Markdown markup span.
fn markup_span(
    start_col: usize,
    end_col: usize,
    modifier: Option<SyntaxModifier>,
) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Markup,
        modifier,
    }
}
