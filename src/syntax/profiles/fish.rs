//! Fish syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "begin", "break", "case", "command", "continue", "else", "end", "for", "function",
    "if", "in", "not", "or", "set", "switch", "time", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const NUMBER_PATTERN: NumberPattern = SIGNED_NUMBER.with_digit_separator(DigitSeparator::None);

/// Static Fish language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Fish,
    display_name: "Fish",
    exact_filenames: &["config.fish"],
    extensions: &["fish"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: C_LIKE_INDENT,
    nested_hooks: &[],
};
