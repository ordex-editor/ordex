//! Weir-specific indentation helpers.

use crate::indent::significant_code_text;
use crate::syntax::HighlightSpan;

/// Return whether `line` opens one capability row with a trailing `with`.
///
/// A signature such as `fn serve() -> Int with` lists its capabilities on the
/// lines below, one per line, before the body's `{`. Returns `true` when the
/// line's code ends with the standalone `with` keyword; returns `false` for
/// every other line shape, including identifiers that merely end in `with`.
pub(crate) fn opens_capability_row(line: &str, spans: &[HighlightSpan]) -> bool {
    let significant = significant_code_text(line, spans);
    significant.strip_suffix("with").is_some_and(|head| {
        head.chars()
            .next_back()
            .is_none_or(|character| !character.is_alphanumeric() && character != '_')
    })
}

#[cfg(test)]
mod tests {
    use super::opens_capability_row;
    use crate::syntax::{HighlightSpan, SyntaxClass};

    /// A trailing standalone `with` opens a row, wherever the signature starts.
    #[test]
    fn capability_row_opens_on_trailing_with_keyword() {
        assert!(opens_capability_row("fn main() -> Int with", &[]));
        assert!(opens_capability_row(
            "    pub handler Blocking(s: Slots): shared for Net with",
            &[]
        ));
        assert!(opens_capability_row("with", &[]));
        assert!(!opens_capability_row("fn main() with Alloc {", &[]));
        assert!(!opens_capability_row("    let x = forwith", &[]));
        assert!(!opens_capability_row("    Alloc,", &[]));
    }

    /// A `with` inside a trailing comment does not open a row.
    #[test]
    fn capability_row_ignores_comment_text() {
        let line = "fn f() {} // with";
        let comment_start = line.find("//").expect("comment opener should exist");
        let spans = vec![HighlightSpan {
            start_col: comment_start,
            end_col: line.chars().count(),
            class: SyntaxClass::Comment,
            modifier: None,
        }];

        assert!(!opens_capability_row(line, &spans));
    }
}
