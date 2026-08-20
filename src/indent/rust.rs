//! Rust-specific indentation helpers.

use crate::syntax::HighlightSpan;

/// Return whether `line` is one Rust attribute anchor (`#[..]` or `#![..]`).
///
/// Returns `true` when the non-comment significant text begins with one Rust
/// attribute introducer; returns `false` for every other line shape.
pub(crate) fn is_attribute_anchor(line: &str, spans: &[HighlightSpan]) -> bool {
    let significant = significant_code_text(line, spans);
    let trimmed = significant.trim_start();
    trimmed.starts_with("#[") || trimmed.starts_with("#![")
}

/// Return whether `line` heads one `where` clause.
///
/// A `where` clause belongs to the signature above it and keeps that
/// signature's indent. Returns `true` for such a line; returns `false` for
/// every other line shape.
pub(crate) fn is_where_clause(line: &str, spans: &[HighlightSpan]) -> bool {
    let significant = significant_code_text(line, spans);
    starts_with_where_keyword(significant.trim_start())
}

/// Return whether `line` opens with one `|` alternative.
///
/// Returns `true` for a lone leading `|`, which introduces either the next
/// alternative of a pattern or the next operand of a bitwise-or expression.
/// Returns `false` for the other tokens that begin with `|`: `||` continues a
/// boolean expression and `|=` assigns, and both take ordinary continuation
/// indent. Returns `false` for every other line shape as well.
pub(crate) fn starts_pipe_alternative(line: &str, spans: &[HighlightSpan]) -> bool {
    let significant = significant_code_text(line, spans);
    let Some(remainder) = significant.trim_start().strip_prefix('|') else {
        return false;
    };
    !remainder.starts_with(['|', '='])
}

/// Return whether `text` begins with the standalone `where` keyword.
///
/// Returns `true` only when `where` is followed by the end of the text or by a
/// character that cannot continue an identifier; returns `false` otherwise, so
/// an identifier such as `whereabouts` is not mistaken for the keyword.
fn starts_with_where_keyword(text: &str) -> bool {
    text.strip_prefix("where").is_some_and(|remainder| {
        remainder
            .chars()
            .next()
            .is_none_or(|character| !character.is_alphanumeric() && character != '_')
    })
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

#[cfg(test)]
mod tests {
    use super::{is_attribute_anchor, is_where_clause, starts_pipe_alternative};
    use crate::syntax::{HighlightSpan, SyntaxClass};

    /// Attribute anchors are recognized in both item and inner-attribute forms.
    #[test]
    fn attribute_anchor_matches_item_and_inner_attributes() {
        assert!(is_attribute_anchor("#[derive(Debug)]", &[]));
        assert!(is_attribute_anchor("    #[test]", &[]));
        assert!(is_attribute_anchor("#![allow(unused)]", &[]));
        assert!(!is_attribute_anchor("struct Item;", &[]));
    }

    /// `where` heads are recognized only as a standalone keyword.
    #[test]
    fn where_clause_matches_only_the_standalone_keyword() {
        assert!(is_where_clause("where", &[]));
        assert!(is_where_clause("    where", &[]));
        assert!(is_where_clause("    where F: Fn(),", &[]));
        assert!(!is_where_clause("    whereabouts = 1;", &[]));
        assert!(!is_where_clause("    let x = 1;", &[]));
    }

    /// Only a lone leading `|` opens an alternative among the `|` tokens.
    #[test]
    fn pipe_alternative_excludes_boolean_or_and_assignment() {
        assert!(starts_pipe_alternative("    | Outcome::Second(value)", &[]));
        assert!(starts_pipe_alternative("        | SECOND_FLAG", &[]));
        // `||` continues a boolean expression and `|=` assigns into a variable.
        assert!(!starts_pipe_alternative("        || other_condition", &[]));
        assert!(!starts_pipe_alternative("        |= SECOND_FLAG;", &[]));
        assert!(!starts_pipe_alternative("    value | other", &[]));
    }

    /// Attribute-looking text inside a string is not one attribute anchor.
    #[test]
    fn attribute_anchor_ignores_string_contents() {
        let line = "let text = \"#[derive(Debug)]\";";
        let string_start = line.find('"').expect("string opener should exist");
        let spans = vec![HighlightSpan {
            start_col: string_start,
            end_col: line.chars().count(),
            class: SyntaxClass::String,
            modifier: None,
        }];

        assert!(!is_attribute_anchor(line, &spans));
    }
}
