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
    previous_non_blank_anchor: Option<(&str, &[HighlightSpan])>,
) -> bool {
    if crate::indent::significant_last_char(line, spans) != Some(',') {
        return false;
    }
    let significant = significant_code_text(line, spans);
    is_match_arm_trailing_comma(&significant)
        || is_member_trailing_comma(&significant)
        || is_match_arm_block_closer_trailing_comma(&significant, previous_non_blank_anchor)
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
///
/// Returns `true` when the non-comment significant text contains `=>` with a
/// non-empty right-hand expression before the trailing comma; returns `false`
/// for partial arms and all non-match-arm shapes.
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
///
/// Returns `true` when the non-comment significant text ends with a comma and
/// has non-empty `name: value` sections outside obvious path separators (`::`);
/// returns `false` when no member-style split is present.
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

/// Return whether one `},` closer belongs to a Rust match-arm block body.
///
/// Returns `true` when the current anchor is `},` and the nearest previous
/// non-blank anchor line is a match-arm opener ending with `=> {`; returns
/// `false` for every other closer-comma anchor.
fn is_match_arm_block_closer_trailing_comma(
    line: &str,
    previous_non_blank_anchor: Option<(&str, &[HighlightSpan])>,
) -> bool {
    if line.trim_start() != "}," {
        return false;
    }
    let Some((prev_line, prev_spans)) = previous_non_blank_anchor else {
        return false;
    };
    let prev_significant = significant_code_text(prev_line, prev_spans);
    prev_significant.trim_end().ends_with("=> {")
}
