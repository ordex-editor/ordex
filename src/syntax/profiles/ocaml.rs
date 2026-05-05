//! OCaml syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "begin", "class", "constraint", "do", "done", "downto", "else",
    "end", "exception", "external", "false", "for", "fun", "function", "if", "in", "include",
    "inherit", "initializer", "lazy", "let", "match", "module", "mutable", "new", "object",
    "of", "open", "or", "private", "rec", "sig", "struct", "then", "to", "true", "try", "type",
    "val", "virtual", "when", "while", "with",
];
const COMMENT_STYLES: &[CommentStyle] = &[block_comment("(*", "*)")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];

/// Static OCaml language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Ocaml,
    display_name: "OCaml",
    exact_filenames: &[],
    extensions: &["ml", "mli"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    manual_indent: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
