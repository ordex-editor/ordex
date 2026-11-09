//! Zig syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "addrspace", "align", "allowzero", "and", "asm", "await", "break", "catch", "const",
    "continue", "defer", "else", "enum", "errdefer", "error", "fn", "for", "if", "inline",
    "linksection", "noalias", "or", "orelse", "packed", "pub", "resume", "return", "struct",
    "suspend", "switch", "test", "threadlocal", "try", "union", "usingnamespace", "var",
    "while",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "null", "true", "unreachable"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//")];
const STRING_STYLES: &[StringStyle] = &[
    prefixed_escaped_delimited_string(&["c"], "\"", "\""),
    double_quoted_string(),
];
const CHAR_STYLES: &[CharStyle] = &[char_literal()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_hex_exponent(true);

/// Static Zig language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Zig,
    display_name: "Zig",
    exact_filenames: &[],
    extensions: &["zig"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: CHAR_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
