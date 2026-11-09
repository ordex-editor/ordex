//! Julia syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "baremodule", "begin", "break", "catch", "const", "continue", "do", "else", "elseif", "end",
    "for", "function", "if", "import", "in", "let", "macro", "module", "mutable", "quote",
    "return", "struct", "try", "using", "where", "while",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "nothing", "true"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("#"),
    nested_block_comment("#=", "=#"),
];
const STRING_STYLES: &[StringStyle] = &[
    custom_prefixed_delimited_string(&["raw"], "\"", "\"", EscapeMode::None, false),
    triple_double_quoted_string(),
    double_quoted_string(),
];
const CHAR_STYLES: &[CharStyle] = &[char_literal()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&["im"])
            .with_float_exact(&["im"]),
    );

/// Static Julia language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Julia,
    display_name: "Julia",
    exact_filenames: &[],
    extensions: &["jl"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: CHAR_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
