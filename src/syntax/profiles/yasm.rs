//! YASM syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "BITS", "CPU", "DEFAULT", "GLOBAL", "SECTION", "db", "dd", "dq", "dw", "extern", "global",
    "resb", "resd", "resq", "resw", "section",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment(";")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_hex(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new().with_integer_exact(&["h", "H", "b", "B", "o", "O", "q", "Q"]),
    );

/// Static YASM language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Yasm,
    display_name: "YASM",
    exact_filenames: &[],
    extensions: &["yasm"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule_ignore_ascii_case(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
