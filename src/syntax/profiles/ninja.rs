//! Ninja syntax profile.

use crate::syntax::profile::*;

const KEYWORDS: &[&str] = &["build", "default", "include", "pool", "rule", "subninja"];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static Ninja language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Ninja,
    display_name: "Ninja",
    exact_filenames: &["build.ninja", "rules.ninja"],
    extensions: &["ninja"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: ":=|$()[]{}",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: NO_MANUAL_INDENT,
    nested_hooks: &[],
};
