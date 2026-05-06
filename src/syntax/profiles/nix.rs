//! Nix syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "assert", "else", "false", "if", "in", "inherit", "let", "null", "or", "rec", "then",
    "true", "with",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    multiline_plain_delimited_string("''", "''"),
    double_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static Nix language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Nix,
    display_name: "Nix",
    exact_filenames: &[],
    extensions: &["nix"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
