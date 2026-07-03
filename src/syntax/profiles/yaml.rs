//! YAML syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const LITERALS: &[&str] = &[
    "false", "False", "null", "Null", "off", "Off", "on", "On", "true", "True",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_binary(true)
    .supports_octal_prefix(true)
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Static YAML language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Yaml,
    display_name: "YAML",
    exact_filenames: &[],
    extensions: &["yaml", "yml"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule(LITERALS)],
    punctuation_chars: "{}[],:?-&*!|>%",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: COLON_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
