//! Rust-specific indentation helpers.

use crate::syntax::HighlightSpan;

/// Return whether one Rust trailing-comma anchor should suppress continuation indent.
///
/// Returns `true` when `line` ends with a significant comma and represents a
/// complete match arm or one brace-block member/pattern anchor. Returns `false`
/// for every other line shape.
pub(crate) fn skip_c_like_continuation_indent_after_trailing_comma(
    line: &str,
    spans: &[HighlightSpan],
    previous_same_indent_anchors: &[(&str, &[HighlightSpan])],
    enclosing_less_indent_anchor: Option<(&str, &[HighlightSpan])>,
) -> bool {
    if crate::indent::significant_last_char(line, spans) != Some(',') {
        return false;
    }
    let significant = significant_code_text(line, spans);
    is_match_arm_trailing_comma(&significant)
        || is_member_trailing_comma(&significant)
        || is_match_arm_block_closer_trailing_comma(&significant, previous_same_indent_anchors)
        || is_brace_block_comma_anchor(
            &significant,
            previous_same_indent_anchors,
            enclosing_less_indent_anchor,
        )
}

/// Return whether `line` is one Rust attribute anchor (`#[..]` or `#![..]`).
///
/// Returns `true` when the non-comment significant text begins with one Rust
/// attribute introducer; returns `false` for every other line shape.
pub(crate) fn is_attribute_anchor(line: &str, spans: &[HighlightSpan]) -> bool {
    let significant = significant_code_text(line, spans);
    let trimmed = significant.trim_start();
    trimmed.starts_with("#[") || trimmed.starts_with("#![")
}

/// Return whether `line` is one terminated block closer anchor for Rust scans.
///
/// Returns `true` when the significant text ends with `;` and the preceding
/// significant suffix resolves to one block closer `}` (optionally followed by
/// `)` or `]` before `;`); returns `false` otherwise.
pub(crate) fn is_terminated_block_closer_anchor(line: &str, spans: &[HighlightSpan]) -> bool {
    let significant = significant_code_text(line, spans);
    let mut chars = significant.chars().rev().filter(|ch| !ch.is_whitespace());
    if chars.next() != Some(';') {
        return false;
    }
    for ch in chars {
        if matches!(ch, ')' | ']') {
            continue;
        }
        return ch == '}';
    }
    false
}

/// Return one line stripped of non-code span characters and trailing whitespace.
fn significant_code_text(line: &str, spans: &[HighlightSpan]) -> String {
    let mut text = String::with_capacity(line.len());
    for (byte_off, ch) in line.char_indices() {
        let col = line[..byte_off].chars().count();
        // Drop non-code characters while preserving remaining syntax tokens.
        if !crate::indent::structural_token_is_code_column(spans, col) {
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
/// same-indentation anchor context indicates one match arm head; returns
/// `false` for every other closer-comma anchor.
///
/// Two anchors are required because match-arm heads can be split across lines:
/// one line may contain only `{` while the preceding same-indentation line
/// contains the `=>` token. Looking at only the nearest anchor would miss that
/// shape and incorrectly keep continuation indentation for `},`.
fn is_match_arm_block_closer_trailing_comma(
    line: &str,
    previous_same_indent_anchors: &[(&str, &[HighlightSpan])],
) -> bool {
    if line.trim_start() != "}," {
        return false;
    }
    let Some((first_line, first_spans)) = previous_same_indent_anchors.first() else {
        return false;
    };
    let first_significant = significant_code_text(first_line, first_spans);
    if first_significant.contains("=>") {
        return true;
    }
    if first_significant.trim() != "{" {
        return false;
    }
    let Some((second_line, second_spans)) = previous_same_indent_anchors.get(1) else {
        return false;
    };
    let second_significant = significant_code_text(second_line, second_spans);
    second_significant.contains("=>")
}

/// Return whether one trailing-comma anchor belongs to a Rust brace-block body.
///
/// Returns `true` when `line` ends with a trailing comma and the surrounding
/// anchor context indicates the current indentation level sits under one `{`
/// opener. Returns `false` when no brace-body context is detected.
fn is_brace_block_comma_anchor(
    line: &str,
    previous_same_indent_anchors: &[(&str, &[HighlightSpan])],
    enclosing_less_indent_anchor: Option<(&str, &[HighlightSpan])>,
) -> bool {
    // A trailing comma is required for this suppression family.
    let Some(without_comma) = line.strip_suffix(',') else {
        return false;
    };
    // Empty payloads like `,` do not express a member or pattern anchor.
    if without_comma.trim().is_empty() {
        return false;
    }
    // Suppress continuation only when the surrounding indentation context
    // confirms the anchor is nested under one brace opener.
    has_brace_block_context(previous_same_indent_anchors, enclosing_less_indent_anchor)
}

/// Return whether anchor context exposes one containing `{` opener.
///
/// Returns `true` when either the nearest same-indentation or enclosing
/// less-indented anchor ends with `{`; returns `false` otherwise.
fn has_brace_block_context(
    previous_same_indent_anchors: &[(&str, &[HighlightSpan])],
    enclosing_less_indent_anchor: Option<(&str, &[HighlightSpan])>,
) -> bool {
    previous_same_indent_anchors
        .first()
        .is_some_and(|(line, spans)| significant_code_text(line, spans).trim_end().ends_with('{'))
        || enclosing_less_indent_anchor.is_some_and(|(line, spans)| {
            significant_code_text(line, spans).trim_end().ends_with('{')
        })
}

#[cfg(test)]
mod tests {
    use super::skip_c_like_continuation_indent_after_trailing_comma;
    use crate::syntax::{HighlightSpan, SyntaxClass};

    /// String-contained commas do not trigger Rust trailing-comma continuation suppression.
    #[test]
    fn trailing_comma_suppression_ignores_string_commas() {
        let line = "const string: &str = r#\"hello,";
        let string_start = line.find("r#\"").expect("string opener should exist");
        let spans = vec![HighlightSpan {
            start_col: string_start,
            end_col: line.chars().count(),
            class: SyntaxClass::String,
            modifier: None,
        }];

        assert!(!skip_c_like_continuation_indent_after_trailing_comma(
            line,
            &spans,
            &[],
            None,
        ));
    }
}
