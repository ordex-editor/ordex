//! Meson syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "elif", "else", "endforeach", "endif", "false", "foreach",
    "if", "not", "or", "project", "return", "true",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[
    multiline_plain_delimited_string("'''", "'''"),
    single_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static Meson language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Meson,
    display_name: "Meson",
    exact_filenames: &["meson.build", "meson_options.txt"],
    extensions: &[],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "()[]{}:,.=+-*/%<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
