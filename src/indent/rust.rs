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
    use super::is_attribute_anchor;
    use crate::syntax::{HighlightSpan, SyntaxClass};

    /// Attribute anchors are recognized in both item and inner-attribute forms.
    #[test]
    fn attribute_anchor_matches_item_and_inner_attributes() {
        assert!(is_attribute_anchor("#[derive(Debug)]", &[]));
        assert!(is_attribute_anchor("    #[test]", &[]));
        assert!(is_attribute_anchor("#![allow(unused)]", &[]));
        assert!(!is_attribute_anchor("struct Item;", &[]));
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
