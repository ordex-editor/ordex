//! Language-specific indentation option routing.

pub(crate) mod rust;

use crate::syntax::HighlightSpan;
use crate::syntax::profile::{LanguageId, LanguageProfile};

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
}

impl Default for IndentLanguageOptions {
    /// Build one default option set with no language-specific C-like overrides.
    fn default() -> Self {
        Self {
            c_like_trailing_comma_rule: CLikeTrailingCommaRule::None,
        }
    }
}

/// Return indentation options for the active language profile.
pub(crate) fn options_for_profile(profile: &LanguageProfile) -> IndentLanguageOptions {
    match profile.id {
        LanguageId::Rust => IndentLanguageOptions {
            c_like_trailing_comma_rule: CLikeTrailingCommaRule::RustMatchArmAndMember,
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
    profile: &LanguageProfile,
) -> bool {
    match options_for_profile(profile).c_like_trailing_comma_rule {
        CLikeTrailingCommaRule::None => false,
        CLikeTrailingCommaRule::RustMatchArmAndMember => {
            rust::skip_c_like_continuation_indent_after_trailing_comma(anchor_line, anchor_spans)
        }
    }
}
