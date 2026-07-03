//! GNU assembler syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    ".ascii", ".byte", ".global", ".globl", ".int", ".long", ".macro", ".quad", ".section",
    ".short", ".string", ".text",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_hex(true)
    .supports_decimal_exponent(true)
    .supports_legacy_octal(true);

/// Static GAS language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Gas,
    display_name: "GAS",
    exact_filenames: &[],
    extensions: &["s", "S"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
