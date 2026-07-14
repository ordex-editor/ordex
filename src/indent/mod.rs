//! Language-specific indentation option routing.

pub(crate) mod rust;

use crate::syntax::engine::LineLexMode;
use crate::syntax::profile::{LanguageId, LanguageProfile};
use crate::syntax::{HighlightSpan, SyntaxClass};

/// Rule families used when a C-like anchor line ends with a trailing comma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CLikeTrailingCommaRule {
    /// Keep generic continuation behavior for comma-terminated anchors.
    None,
    /// Apply Rust-specific handling for complete match-arm and member anchors.
    RustMatchArmAndMember,
}

/// Per-language indentation behavior flags selected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndentLanguageOptions {
    /// Rule used for C-like continuation handling on trailing comma anchors.
    pub(crate) c_like_trailing_comma_rule: CLikeTrailingCommaRule,
    /// Whether Rust-style attribute anchors should be treated as terminated.
    pub(crate) c_like_treat_attribute_anchor_as_terminated: bool,
}

impl Default for IndentLanguageOptions {
    /// Build one default option set with no language-specific C-like overrides.
    fn default() -> Self {
        Self {
            c_like_trailing_comma_rule: CLikeTrailingCommaRule::None,
            c_like_treat_attribute_anchor_as_terminated: false,
        }
    }
}

/// Return indentation options for the active language profile.
pub(crate) fn options_for_profile(profile: &LanguageProfile) -> IndentLanguageOptions {
    match profile.id {
        LanguageId::Rust => IndentLanguageOptions {
            c_like_trailing_comma_rule: CLikeTrailingCommaRule::RustMatchArmAndMember,
            c_like_treat_attribute_anchor_as_terminated: true,
        },
        _ => IndentLanguageOptions::default(),
    }
}

/// Return whether one trailing-comma anchor should suppress continuation indent.
///
/// Returns `true` only when the active language options require suppression for
/// the supplied anchor line; returns `false` for default handling.
pub(crate) fn skip_c_like_continuation_indent_after_trailing_comma(
    anchor_line: &str,
    anchor_spans: &[HighlightSpan],
    previous_same_indent_anchors: &[(&str, &[HighlightSpan])],
    enclosing_less_indent_anchor: Option<(&str, &[HighlightSpan])>,
    profile: &LanguageProfile,
) -> bool {
    match options_for_profile(profile).c_like_trailing_comma_rule {
        CLikeTrailingCommaRule::None => false,
        CLikeTrailingCommaRule::RustMatchArmAndMember => {
            rust::skip_c_like_continuation_indent_after_trailing_comma(
                anchor_line,
                anchor_spans,
                previous_same_indent_anchors,
                enclosing_less_indent_anchor,
            )
        }
    }
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

/// Return one profile-adjusted continuation-head indent after a block closer.
///
/// Returns one possibly clamped head indent suitable for the active language
/// profile; returns the input head indent unchanged when no adjustment applies.
pub(crate) fn adjust_c_like_block_closer_head_indent(
    head_indent: usize,
    anchor_indent: usize,
    line: &str,
    spans: &[HighlightSpan],
    profile: &LanguageProfile,
) -> usize {
    if profile.id != LanguageId::Rust {
        return head_indent;
    }
    if rust::is_terminated_block_closer_anchor(line, spans) {
        return head_indent;
    }
    head_indent.max(anchor_indent)
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
