//! Markdown syntax profile.

use crate::syntax::profile::*;

/// Static Markdown language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Markdown,
    display_name: "Markdown",
    exact_filenames: &["README.md"],
    extensions: &["md", "markdown"],
    comment_styles: &[],
    string_styles: &[],
    identifier: None,
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: UNSIGNED_NUMBER,
    markup_rules: Some(COMMON_MARKUP_RULES),
    nested_hooks: &[],
};
