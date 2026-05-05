//! Haskell syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "case", "class", "data", "default", "deriving", "do", "else", "if", "import", "in",
    "infix", "infixl", "infixr", "instance", "let", "module", "newtype", "of", "then", "type",
    "where",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("--"),
    nested_block_comment("{-", "-}"),
];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_octal_prefix(true)
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Static Haskell language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Haskell,
    display_name: "Haskell",
    exact_filenames: &[],
    extensions: &["hs", "lhs"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: NO_MANUAL_INDENT,
    nested_hooks: &[],
};
