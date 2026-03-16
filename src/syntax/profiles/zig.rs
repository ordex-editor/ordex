//! Zig syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "addrspace", "align", "allowzero", "and", "asm", "await", "break", "catch", "const",
    "continue", "defer", "else", "enum", "errdefer", "error", "false", "fn", "for", "if",
    "inline", "linksection", "noalias", "null", "or", "orelse", "packed", "pub", "resume",
    "return", "struct", "suspend", "switch", "test", "threadlocal", "true", "try", "union",
    "unreachable", "usingnamespace", "var", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//")];
const STRING_STYLES: &[StringStyle] = &[
    prefixed_escaped_delimited_string(&["c"], "\"", "\""),
    double_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_hex_exponent(true);

/// Static Zig language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Zig,
    display_name: "Zig",
    exact_filenames: &[],
    extensions: &["zig"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
