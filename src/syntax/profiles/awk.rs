//! AWK syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "BEGIN", "BEGINFILE", "END", "ENDFILE", "break", "continue", "delete", "do", "else", "exit",
    "for", "function", "if", "in", "next", "nextfile", "print", "printf", "return", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static AWK language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Awk,
    display_name: "AWK",
    exact_filenames: &[],
    extensions: &["awk"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
