//! C syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "false", "float", "for", "goto", "if", "inline", "int", "long",
    "register", "restrict", "return", "short", "signed", "sizeof", "static", "struct",
    "switch", "true", "typedef", "union", "unsigned", "void", "volatile", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const PREPROCESSOR_KEYWORDS: &[&str] = &[
    "define",
    "elif",
    "else",
    "endif",
    "error",
    "if",
    "ifdef",
    "ifndef",
    "include",
    "include_next",
    "line",
    "pragma",
    "undef",
    "warning",
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    exact_words_after(PREPROCESSOR_KEYWORDS, '#', true, true, PREPROCESSOR_STYLE),
    keyword_rule(KEYWORDS),
];
const INTEGER_SUFFIX_GROUPS: &[NumberSuffixGroup] = &[
    suffix_group(&["u", "U"]),
    suffix_group(&["ll", "LL", "l", "L"]),
];
const FLOAT_SUFFIXES: &[&str] = &["f", "F", "l", "L"];
const C_TO_H: &[&str] = &["h"];
const H_TO_C: &[&str] = &["cc", "cpp", "cxx", "c"];
const CORRESPONDING_RULES: &[CorrespondingExtensionRule] = &[
    corresponding_extension_rule("c", C_TO_H),
    corresponding_extension_rule("h", H_TO_C),
];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::None)
    .supports_binary(false)
    .supports_octal_prefix(false)
    .supports_legacy_octal(true)
    .supports_hex_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_groups(INTEGER_SUFFIX_GROUPS)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static C language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::C,
    display_name: "C",
    exact_filenames: &[],
    extensions: &["c", "h"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>#",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: Some(CORRESPONDING_RULES),
};
