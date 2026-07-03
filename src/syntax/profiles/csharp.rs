//! C# syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "base", "bool", "break", "byte", "case", "catch",
    "char", "checked", "class", "const", "continue", "decimal", "default", "delegate", "do",
    "double", "else", "enum", "event", "explicit", "extern", "false", "finally", "fixed",
    "float", "for", "foreach", "if", "implicit", "in", "int", "interface", "internal", "is",
    "lock", "long", "namespace", "new", "null", "object", "operator", "out", "override",
    "private", "protected", "public", "readonly", "record", "ref", "return", "sealed",
    "short", "static", "string", "struct", "switch", "this", "throw", "true", "try", "typeof",
    "unchecked", "unsafe", "using", "var", "virtual", "void", "while",
];
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
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
