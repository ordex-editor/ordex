//! Markdown syntax profile.

use crate::syntax::profile::*;

const NESTED_HOOKS: &[NestedLanguageHook] = &[];

/// Static Markdown language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Markdown,
    display_name: "Markdown",
    detection: LanguageDetection {
        exact_filenames: &["README.md"],
        extensions: &["md", "markdown"],
    },
    comment_styles: &[],
    string_styles: &[],
    identifier: None,
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: UNSIGNED_NUMBER,
    markdown_rules: Some(COMMON_MARKDOWN_RULES),
    nested_hooks: NESTED_HOOKS,
};
