//! R syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "break", "else", "FALSE", "for", "function", "if", "in", "NA", "NULL", "TRUE", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_hex(true)
    .supports_decimal_exponent(true)
    .supports_hex_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&["L", "i"])
            .with_float_exact(&["i"]),
    );

/// Static R language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::R,
    display_name: "R",
    exact_filenames: &[],
    extensions: &["R", "r"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
