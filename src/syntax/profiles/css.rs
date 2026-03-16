//! CSS syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "auto", "false", "important", "inherit", "initial", "none", "normal", "null", "revert",
    "true", "unset",
];
const COMMENT_STYLES: &[CommentStyle] = &[block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule_ignore_ascii_case(KEYWORDS)];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_leading_dot(true);

/// Static CSS language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Css,
    display_name: "CSS",
    exact_filenames: &[],
    extensions: &["css"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%#@!<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
