//! Lua syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in",
    "local", "not", "or", "repeat", "return", "then", "until", "while",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "nil", "true"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] =
    &[preferred_line_comment("--"), block_comment("--[[", "]]")];
const STRING_STYLES: &[StringStyle] = &[
    multiline_plain_delimited_string("[[", "]]"),
    double_quoted_string(),
    single_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_hex(true)
    .supports_decimal_exponent(true)
    .supports_hex_exponent(true);

/// Static Lua language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Lua,
    display_name: "Lua",
    exact_filenames: &[],
    extensions: &["lua"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^~<>#",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
