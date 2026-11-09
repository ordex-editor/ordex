//! QML syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "break", "case", "catch", "class", "component", "const", "continue", "default", "do",
    "else", "for", "function", "id", "if", "import", "in", "let", "new", "null", "on",
    "property", "readonly", "required", "return", "signal", "switch", "this", "throw", "try",
    "var", "while",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "true"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    custom_delimited_string("`", "`", EscapeMode::Backslash, false),
    double_quoted_string(),
    single_quoted_string(),
];

/// Static QML language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Qml,
    display_name: "QML",
    exact_filenames: &[],
    extensions: &["qml"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
