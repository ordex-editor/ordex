//! Lisp syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "cond", "defmacro", "defparameter", "defun", "defvar", "if", "lambda", "let", "let*",
    "nil", "or", "progn", "quote", "setf", "t", "when", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment(";")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .supports_binary(true)
    .supports_octal_prefix(true)
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Static Lisp language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Lisp,
    display_name: "Lisp",
    exact_filenames: &[],
    extensions: &["lisp", "lsp", "cl", "el"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "()[]'`,",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
