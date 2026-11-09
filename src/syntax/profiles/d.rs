//! D syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "alias", "break", "case", "class", "continue", "debug", "else", "enum",
    "foreach", "foreach_reverse", "if", "import", "in", "interface", "module", "new",
    "return", "struct", "switch", "template", "this", "void", "while",
];
#[rustfmt::skip]
const STORAGE_CLASS_WORDS: &[&str] = &[
    "auto", "const", "immutable", "private", "public", "shared", "static",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "true"];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    block_comment("/*", "*/"),
    doc_block_comment("/**", "*/"),
    nested_block_comment("/+", "+/"),
    nested_doc_block_comment("/++", "+/"),
];
const STRING_STYLES: &[StringStyle] = &[
    double_quoted_string(),
    custom_delimited_string("`", "`", EscapeMode::None, true),
];
const CHAR_STYLES: &[CharStyle] = &[char_literal(), prefixed_char_literal(&["w", "d", "c"])];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(STORAGE_CLASS_WORDS, STORAGE_CLASS_STYLE),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const INTEGER_SUFFIXES: &[&str] = &[
    "Lu", "LU", "uL", "UL", "fi", "Fi", "Li", "L", "u", "U", "f", "F", "i",
];
const FLOAT_SUFFIXES: &[&str] = &["fi", "Fi", "Li", "f", "F", "L", "i"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_hex_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(INTEGER_SUFFIXES)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static D language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::D,
    display_name: "D",
    exact_filenames: &[],
    extensions: &["d"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: CHAR_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
