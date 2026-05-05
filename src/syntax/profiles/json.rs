//! JSON syntax profile.

use crate::syntax::profile::*;

const LITERALS: &[&str] = &["false", "null", "true"];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static JSON language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Json,
    display_name: "JSON",
    exact_filenames: &[],
    extensions: &["json"],
    comment_styles: &[],
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(LITERALS)],
    punctuation_chars: "{}[],:",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: C_LIKE_INDENT,
    nested_hooks: &[],
};
