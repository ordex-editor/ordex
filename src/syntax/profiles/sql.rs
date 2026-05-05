//! SQL syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "ALTER", "AND", "AS", "BY", "CREATE", "DELETE", "DROP", "ELSE", "FALSE", "FROM", "GROUP",
    "HAVING", "INSERT", "INTO", "JOIN", "LIMIT", "NOT", "NULL", "ON", "OR", "ORDER", "SELECT",
    "SET", "TABLE", "TRUE", "UPDATE", "VALUES", "WHERE",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("--"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[single_quoted_string(), double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static SQL language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Sql,
    display_name: "SQL",
    exact_filenames: &[],
    extensions: &["sql"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule_ignore_ascii_case(KEYWORDS)],
    punctuation_chars: "()[],:;.=+-*/%<>!",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
