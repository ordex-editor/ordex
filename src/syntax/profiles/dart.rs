//! Dart syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue",
    "default", "do", "else", "enum", "extends", "false", "final", "for", "if", "implements",
    "import", "in", "is", "mixin", "new", "null", "return", "super", "switch", "this", "throw",
    "true", "try", "var", "void", "while", "with", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    prefixed_multiline_escaped_delimited_string(&["r"], "\"\"\"", "\"\"\""),
    prefixed_multiline_escaped_delimited_string(&["r"], "'''", "'''"),
    triple_double_quoted_string(),
    triple_single_quoted_string(),
    prefixed_escaped_delimited_string(&["r"], "\"", "\""),
    prefixed_escaped_delimited_string(&["r"], "'", "'"),
    double_quoted_string(),
    single_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Static Dart language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Dart,
    display_name: "Dart",
    exact_filenames: &[],
    extensions: &["dart"],
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
