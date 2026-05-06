//! QML syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "break", "case", "catch", "class", "component", "const", "continue", "default", "do",
    "else", "false", "for", "function", "id", "if", "import", "in", "let", "new", "null",
    "on", "property", "readonly", "required", "return", "signal", "switch", "this", "throw",
    "true", "try", "var", "while",
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
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
};
