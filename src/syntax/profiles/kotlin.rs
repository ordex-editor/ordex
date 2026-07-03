//! Kotlin syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in",
    "interface", "is", "null", "object", "package", "return", "super", "this", "throw", "true",
    "try", "typealias", "val", "var", "when", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_binary(true)
    .supports_hex(true)
    .supports_decimal_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&["u", "U", "L", "UL", "uL"])
            .with_float_exact(&["f", "F"]),
    );

/// Static Kotlin language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Kotlin,
    display_name: "Kotlin",
    exact_filenames: &[],
    extensions: &["kt", "kts"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
