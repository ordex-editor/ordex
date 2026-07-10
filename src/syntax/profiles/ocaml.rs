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
const ML_TO_MLI: &[&str] = &["mli"];
const MLI_TO_ML: &[&str] = &["ml"];
const CORRESPONDING_RULES: &[CorrespondingExtensionRule] = &[
    corresponding_extension_rule("ml", ML_TO_MLI),
    corresponding_extension_rule("mli", MLI_TO_ML),
];

/// Static OCaml language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Ocaml,
    display_name: "OCaml",
    exact_filenames: &[],
    extensions: &["ml", "mli"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore),
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: Some(CORRESPONDING_RULES),
};
