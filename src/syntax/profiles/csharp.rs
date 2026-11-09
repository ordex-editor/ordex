//! C# syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "await", "base", "bool", "break", "byte", "case", "catch", "char", "checked",
    "class", "continue", "decimal", "default", "delegate", "do", "double", "else", "enum",
    "explicit", "finally", "float", "for", "foreach", "if", "implicit", "in", "int",
    "interface", "is", "lock", "long", "namespace", "new", "object", "out", "record", "ref",
    "return", "short", "string", "struct", "switch", "this", "throw", "try", "typeof",
    "unchecked", "using", "var", "void", "while",
];
#[rustfmt::skip]
const STORAGE_CLASS_WORDS: &[&str] = &[
    "abstract", "async", "const", "event", "extern", "fixed", "internal", "operator",
    "override", "private", "protected", "public", "readonly", "sealed", "static", "unsafe",
    "virtual",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "null", "true"];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    block_comment("/*", "*/"),
    doc_block_comment("/**", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[
    custom_delimited_string("\"\"\"", "\"\"\"", EscapeMode::None, true),
    prefixed_multiline_repeated_quote_string(&["@", "$@", "@$"], "\"", "\""),
    double_quoted_string(),
];
const CHAR_STYLES: &[CharStyle] = &[char_literal()];
const PREPROCESSOR_KEYWORDS: &[&str] = &[
    "define",
    "elif",
    "else",
    "endif",
    "error",
    "if",
    "line",
    "nullable",
    "pragma",
    "region",
    "endregion",
    "undef",
    "warning",
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    exact_words_after(PREPROCESSOR_KEYWORDS, '#', true, true, PREPROCESSOR_STYLE),
    keyword_rule(KEYWORDS),
    exact_words_rule(STORAGE_CLASS_WORDS, STORAGE_CLASS_STYLE),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const INTEGER_SUFFIX_GROUPS: &[NumberSuffixGroup] =
    &[suffix_group(&["u", "U"]), suffix_group(&["l", "L"])];
const FLOAT_SUFFIXES: &[&str] = &["f", "F", "d", "D", "m", "M"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_octal_prefix(false)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_groups(INTEGER_SUFFIX_GROUPS)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static C# language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::CSharp,
    display_name: "C#",
    exact_filenames: &[],
    extensions: &["cs"],
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
