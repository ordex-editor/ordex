//! GraphQL syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "directive", "enum", "extend", "false", "fragment", "implements", "input", "interface",
    "mutation", "null", "on", "query", "scalar", "schema", "subscription", "true", "type",
    "union",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static GraphQL language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::GraphQl,
    display_name: "GraphQL",
    exact_filenames: &[],
    extensions: &["graphql", "gql"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]():=@|!$.,",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
