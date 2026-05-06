//! Vala syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "as", "base", "break", "case", "catch", "class", "const", "continue", "default",
    "delegate", "do", "else", "enum", "false", "for", "foreach", "if", "in", "interface", "is",
    "lock", "namespace", "new", "null", "out", "owned", "private", "protected", "public",
    "ref", "return", "signal", "static", "struct", "switch", "this", "throw", "true", "try",
    "using", "var", "virtual", "void", "while", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];

/// Static Vala language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Vala,
    display_name: "Vala",
    exact_filenames: &[],
    extensions: &["vala", "vapi"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
};
