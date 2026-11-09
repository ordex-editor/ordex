//! Nix syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "null", "true"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
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
    char_styles: &[],
    identifier: ascii_identifier_with_dashes(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
