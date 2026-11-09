//! R syntax profile.

use crate::syntax::profile::*;

const KEYWORDS: &[&str] = &["break", "else", "for", "function", "if", "in", "while"];
const BUILTIN_VALUE_WORDS: &[&str] = &["FALSE", "NA", "NULL", "TRUE"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
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
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
