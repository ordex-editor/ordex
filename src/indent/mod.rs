//! Language-specific indentation option routing.

pub(crate) mod rust;
pub(crate) mod scope;

use crate::syntax::engine::LineLexMode;
use crate::syntax::profile::{LanguageId, LanguageProfile};
use crate::syntax::{HighlightSpan, SyntaxClass};

/// Per-language indentation behavior flags selected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct IndentLanguageOptions {
    /// Whether Rust-style attribute anchors should be treated as terminated.
    pub(crate) c_like_treat_attribute_anchor_as_terminated: bool,
    /// Whether Rust `where` clauses and `|` or-patterns keep their statement indent.
    pub(crate) c_like_align_statement_heads: bool,
}

/// Return indentation options for the active language profile.
pub(crate) fn options_for_profile(profile: &LanguageProfile) -> IndentLanguageOptions {
    match profile.id {
        LanguageId::Rust => IndentLanguageOptions {
            c_like_treat_attribute_anchor_as_terminated: true,
            c_like_align_statement_heads: true,
        },
        _ => IndentLanguageOptions::default(),
    }
}

/// Return whether one line heads a construct aligned to its own statement.
///
/// Returns `true` only when the active language marks this line as a head that
/// keeps the indent of the statement it belongs to instead of receiving
/// continuation indent; returns `false` otherwise.
pub(crate) fn treat_c_like_line_as_aligned_head(
    line: &str,
    spans: &[HighlightSpan],
    profile: &LanguageProfile,
) -> bool {
    let options = options_for_profile(profile);
    options.c_like_align_statement_heads && rust::is_where_clause(line, spans)
}

/// Return whether one line opens with a `|` alternative for the active language.
///
/// Returns `true` only when the language gives `|` alternatives their own
/// alignment rules; returns `false` otherwise.
pub(crate) fn c_like_line_starts_pipe_alternative(
    line: &str,
    spans: &[HighlightSpan],
    profile: &LanguageProfile,
) -> bool {
    let options = options_for_profile(profile);
    options.c_like_align_statement_heads && rust::starts_pipe_alternative(line, spans)
}

/// Return whether one anchor line must behave as a terminated C-like statement.
///
/// Returns `true` only when the active language profile marks this anchor as a
/// non-continuation terminator; returns `false` otherwise.
pub(crate) fn treat_c_like_anchor_as_terminated(
    line: &str,
    spans: &[HighlightSpan],
    profile: &LanguageProfile,
) -> bool {
    let options = options_for_profile(profile);
    options.c_like_treat_attribute_anchor_as_terminated && rust::is_attribute_anchor(line, spans)
}

/// Return whether reindent should keep one line's leading prefix unchanged.
///
/// Returns `true` when the line should skip prefix rewrite during reindent;
/// returns `false` when normal indentation rewrite should proceed.
pub(crate) fn skip_reindent_prefix_rewrite(
    line: &str,
    spans: &[HighlightSpan],
    entry_mode: LineLexMode,
) -> bool {
    matches!(entry_mode, LineLexMode::String { .. })
        && first_non_whitespace_token_is_string(line, spans)
}

/// Return the last significant character of `line`.
///
/// Scans characters from the end of the line, skipping whitespace and any
/// character covered by non-code (`Comment`/`String`) spans, then returns the
/// nearest remaining character. Returns `None` when no significant character
/// exists.
pub(crate) fn significant_last_char(line: &str, spans: &[HighlightSpan]) -> Option<char> {
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
            structural_token_is_code_column(spans, *col)
        })
        .map(|(_, ch)| ch)
        .next()
}

/// Return whether `column` belongs to code suitable for structural indentation tokens.
///
/// Returns `true` when `column` is not covered by `Comment` or `String` spans,
/// and returns `false` when the column is inside one of those non-structural
/// regions.
pub(crate) fn structural_token_is_code_column(spans: &[HighlightSpan], column: usize) -> bool {
    spans
        .iter()
        .find(|span| span.covers(column))
        .is_none_or(|span| !matches!(span.class, SyntaxClass::Comment | SyntaxClass::String))
}

/// Return whether `line` starts with a string token after indentation.
///
/// Returns `true` when the first non-whitespace character is covered by one
/// `String` syntax span; returns `false` when no token exists or when the
/// first token belongs to another syntax class.
fn first_non_whitespace_token_is_string(line: &str, spans: &[HighlightSpan]) -> bool {
    line.char_indices()
        .map(|(byte_off, ch)| (line[..byte_off].chars().count(), ch))
        .find(|(_, ch)| !ch.is_whitespace())
        .and_then(|(column, _)| spans.iter().find(|span| span.covers(column)))
        .is_some_and(|span| span.class == SyntaxClass::String)
}

#[cfg(test)]
mod tests {
    use super::{significant_last_char, structural_token_is_code_column};
    use crate::syntax::{HighlightSpan, SyntaxClass};

    /// `significant_last_char` ignores punctuation that lives inside string spans.
    #[test]
    fn significant_last_char_skips_string_span_tokens() {
        let line = "const string: &str = r#\"hello,";
        let string_start = line.find("r#\"").expect("string opener should exist");
        let spans = vec![HighlightSpan {
            start_col: string_start,
            end_col: line.chars().count(),
            class: SyntaxClass::String,
            modifier: None,
        }];

        assert_eq!(significant_last_char(line, &spans), Some('='));
    }

    /// Structural-token checks reject string/comment columns and accept code columns.
    #[test]
    fn structural_token_column_classification_skips_non_code_spans() {
        let spans = vec![
            HighlightSpan {
                start_col: 4,
                end_col: 8,
                class: SyntaxClass::String,
                modifier: None,
            },
            HighlightSpan {
                start_col: 10,
                end_col: 14,
                class: SyntaxClass::Comment,
                modifier: None,
            },
        ];

        assert!(structural_token_is_code_column(&spans, 2));
        assert!(!structural_token_is_code_column(&spans, 5));
        assert!(!structural_token_is_code_column(&spans, 12));
    }
}
