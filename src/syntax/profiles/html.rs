//! HTML syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] = &[block_comment("<!--", "-->")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static HTML language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Html,
    display_name: "HTML",
    exact_filenames: &[],
    extensions: &["html", "htm"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[any_identifier_before('=', KEYWORD_STYLE)],
    punctuation_chars: "<>/!?=:.-[]",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
