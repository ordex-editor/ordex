//! TOML syntax profile.

use crate::syntax::profile::*;

const BOOLEANS: &[&str] = &["true", "false"];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[
    triple_double_quoted_string(),
    multiline_plain_delimited_string("'''", "'''"),
    double_quoted_string(),
    plain_delimited_string("'", "'"),
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    exact_words_rule(BOOLEANS, KEYWORD_STYLE),
    any_identifier_before('=', KEYWORD_STYLE),
];

/// Static TOML language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Toml,
    display_name: "TOML",
    exact_filenames: &["Cargo.toml"],
    extensions: &["toml", "cfg"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "[]{}=.,:",
    number_pattern: SIGNED_NUMBER.with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
