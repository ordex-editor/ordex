//! AsciiDoc syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] =
    &[preferred_line_comment("//"), block_comment("////", "////")];

/// Static AsciiDoc language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::AsciiDoc,
    display_name: "AsciiDoc",
    exact_filenames: &[],
    extensions: &["adoc", "asciidoc", "asc"],
    comment_styles: COMMENT_STYLES,
    string_styles: &[],
    identifier: None,
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: UNSIGNED_NUMBER,
    markup_rules: None,
    nested_hooks: &[],
};
