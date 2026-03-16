//! Perl syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "BEGIN", "END", "elsif", "else", "for", "foreach", "if", "last", "my", "next", "our",
    "package", "redo", "return", "state", "sub", "unless", "until", "use", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];

/// Static Perl language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Perl,
    display_name: "Perl",
    exact_filenames: &[],
    extensions: &["pl", "pm", "t"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NumberPattern::common_code(),
    markup_rules: None,
    nested_hooks: &[],
};
