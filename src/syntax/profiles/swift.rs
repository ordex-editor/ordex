//! Swift syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "actor", "as", "async", "await", "break", "case", "catch", "class", "continue", "default",
    "defer", "do", "else", "enum", "extension", "false", "for", "func", "guard", "if", "import",
    "in", "init", "inout", "let", "nil", "protocol", "repeat", "return", "self", "struct",
    "switch", "throw", "throws", "true", "try", "var", "where", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    raw_hash_string(&[""], '#', '"'),
    triple_double_quoted_string(),
    double_quoted_string(),
];
const CHAR_STYLES: &[CharStyle] = &[char_literal()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_hex_exponent(true);

/// Static Swift language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Swift,
    display_name: "Swift",
    exact_filenames: &[],
    extensions: &["swift"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: CHAR_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@#",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
