//! Solidity syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "address", "bool", "break", "bytes", "case", "catch", "constant", "constructor",
    "continue", "contract", "default", "delete", "do", "else", "emit", "enum", "event", "for",
    "function", "if", "import", "interface", "library", "mapping", "modifier", "new",
    "payable", "pragma", "private", "public", "pure", "return", "returns", "revert", "storage",
    "struct", "switch", "throw", "try", "type", "using", "view", "while",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "this", "true"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];

/// Static Solidity language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Solidity,
    display_name: "Solidity",
    exact_filenames: &[],
    extensions: &["sol"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
