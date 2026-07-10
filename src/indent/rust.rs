//! Rust-specific indentation helpers.

use crate::syntax::{HighlightSpan, SyntaxClass};

/// Return whether one Rust trailing-comma anchor should suppress continuation indent.
///
/// Returns `true` when `line` ends with a significant comma and represents a
/// complete match arm or member-style `name: value,` anchor. Returns `false`
/// for every other line shape.
pub(crate) fn skip_c_like_continuation_indent_after_trailing_comma(
    line: &str,
    spans: &[HighlightSpan],
) -> bool {
    if significant_last_char(line, spans) != Some(',') {
        return false;
    }
    let significant = significant_code_text(line, spans);
    is_match_arm_trailing_comma(&significant) || is_member_trailing_comma(&significant)
}

/// Return the last significant character in one line.
fn significant_last_char(line: &str, spans: &[HighlightSpan]) -> Option<char> {
    line.char_indices()
        .map(|(byte_off, ch)| {
            let col = line[..byte_off].chars().count();
            (col, ch)
        })
        .rev()
        .filter(|(col, ch)| {
            if ch.is_whitespace() {
                return false;
            }
            // Skip comment-class spans so only syntax-relevant code remains.
            !spans
                .iter()
                .any(|span| span.class == SyntaxClass::Comment && span.covers(*col))
        })
        .map(|(_, ch)| ch)
        .next()
}

/// Return one line stripped of comment-class characters and trailing whitespace.
fn significant_code_text(line: &str, spans: &[HighlightSpan]) -> String {
    let mut text = String::with_capacity(line.len());
    for (byte_off, ch) in line.char_indices() {
        let col = line[..byte_off].chars().count();
        // Drop comment characters while preserving remaining syntax tokens.
        if spans
            .iter()
            .any(|span| span.class == SyntaxClass::Comment && span.covers(col))
        {
            continue;
        }
        text.push(ch);
    }
    text.trim_end().to_string()
}

/// Return whether `line` is a complete Rust match arm ending with `,`.
fn is_match_arm_trailing_comma(line: &str) -> bool {
    let Some(without_comma) = line.strip_suffix(',') else {
        return false;
    };
    let trimmed = without_comma.trim_end();
    let Some((left, right)) = trimmed.split_once("=>") else {
        return false;
    };
    !left.trim().is_empty() && !right.trim().is_empty()
}

/// Return whether `line` is a member-style `name: value,` Rust anchor.
fn is_member_trailing_comma(line: &str) -> bool {
    let Some(without_comma) = line.strip_suffix(',') else {
        return false;
    };
    let trimmed = without_comma.trim_end();
    // Split once at the first member separator to keep right-side colons intact.
    let Some(colon_idx) = trimmed.find(':') else {
        return false;
    };
    // Path separators must not be treated as member separators.
    if colon_idx + 1 < trimmed.len() && trimmed[colon_idx + 1..].starts_with(':') {
        return false;
    }
    if colon_idx > 0 && trimmed[..colon_idx].ends_with(':') {
        return false;
    }
    let left = trimmed[..colon_idx].trim();
    let right = trimmed[colon_idx + 1..].trim();
    !left.is_empty() && !right.is_empty()
}
