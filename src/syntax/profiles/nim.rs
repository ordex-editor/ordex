//! Nim syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "addr", "and", "as", "asm", "bind", "block", "break", "case", "const", "continue", "defer",
    "discard", "distinct", "div", "do", "elif", "else", "end", "enum", "except", "export",
    "finally", "for", "from", "func", "if", "import", "in", "include", "interface", "is",
    "iterator", "let", "macro", "method", "mixin", "mod", "nil", "not", "object", "of", "or",
    "out", "proc", "ptr", "raise", "ref", "return", "template", "true", "try", "tuple", "type",
    "using", "var", "when", "while", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#"), block_comment("#[", "]#")];
const STRING_STYLES: &[StringStyle] = &[
    triple_double_quoted_string(),
    triple_single_quoted_string(),
    double_quoted_string(),
    single_quoted_string(),
];

/// Static Nim language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Nim,
    display_name: "Nim",
    exact_filenames: &[],
    extensions: &["nim", "nims"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NumberPattern::common_code(),
    markup_rules: None,
    indentation: COLON_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
