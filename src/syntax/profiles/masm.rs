//! MASM syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    ".code", ".const", ".data", ".model", ".stack", "ASSUME", "END", "ENDP", "PROC", "db", "dd",
    "dq", "dw", "endp", "proc",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment(";")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_hex(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new().with_integer_exact(&["h", "H", "b", "B", "o", "O", "q", "Q"]),
    );

/// Static MASM language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Masm,
    display_name: "MASM",
    exact_filenames: &[],
    extensions: &["masm"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule_ignore_ascii_case(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: NO_MANUAL_INDENT,
    nested_hooks: &[],
};
