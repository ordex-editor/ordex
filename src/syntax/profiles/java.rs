//! Java syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "assert", "boolean", "break", "byte", "case", "catch", "char", "const", "continue",
    "do", "double", "else", "finally", "float", "for", "if", "import", "instanceof", "int",
    "long", "native", "new", "package", "return", "short", "super", "switch", "this",
    "throw", "try", "var", "void", "while",
];
#[rustfmt::skip]
const STORAGE_CLASS_WORDS: &[&str] = &[
    "abstract", "class", "default", "enum", "extends", "final", "implements", "interface",
    "private", "protected", "public", "record", "sealed", "static", "strictfp",
    "synchronized", "throws", "transient", "volatile",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "null", "true"];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    block_comment("/*", "*/"),
    doc_block_comment("/**", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const CHAR_STYLES: &[CharStyle] = &[char_literal()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(STORAGE_CLASS_WORDS, STORAGE_CLASS_STYLE),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const INTEGER_SUFFIXES: &[&str] = &["l", "L"];
const FLOAT_SUFFIXES: &[&str] = &["f", "F", "d", "D"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_octal_prefix(false)
    .supports_legacy_octal(true)
    .supports_hex_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(INTEGER_SUFFIXES)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static Java language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Java,
    display_name: "Java",
    exact_filenames: &[],
    extensions: &["java"],
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
