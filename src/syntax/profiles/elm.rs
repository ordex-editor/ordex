//! Elm syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "alias", "as", "case", "else", "exposing", "false", "if", "import", "in", "let", "module",
    "of", "port", "then", "true", "type",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("--"),
    nested_block_comment("{-", "-}"),
];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Static Elm language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Elm,
    display_name: "Elm",
    exact_filenames: &[],
    extensions: &["elm"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
