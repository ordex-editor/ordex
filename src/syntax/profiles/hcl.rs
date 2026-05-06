//! HCL syntax profile.

use crate::syntax::profile::*;

const KEYWORDS: &[&str] = &["false", "for", "if", "in", "null", "true"];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("#"),
    line_comment("//"),
    block_comment("/*", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static HCL language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Hcl,
    display_name: "HCL",
    exact_filenames: &[],
    extensions: &["hcl", "tf", "tfvars"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]():,.=+-*/%$<>!?",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
};
