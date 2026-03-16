//! INI syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment(";"), line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static INI language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Ini,
    display_name: "INI",
    exact_filenames: &[".gitconfig"],
    extensions: &["ini"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[],
    punctuation_chars: "[]=:.",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
