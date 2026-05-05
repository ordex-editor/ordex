//! Java syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class",
    "const", "continue", "default", "do", "double", "else", "enum", "extends", "false",
    "final", "finally", "float", "for", "if", "implements", "import", "instanceof", "int",
    "interface", "long", "native", "new", "null", "package", "private", "protected", "public",
    "record", "return", "sealed", "short", "static", "strictfp", "super", "switch",
    "synchronized", "this", "throw", "throws", "transient", "true", "try", "var", "void",
    "volatile", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    block_comment("/*", "*/"),
    doc_block_comment("/**", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
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
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: C_LIKE_INDENT,
    nested_hooks: &[],
};
