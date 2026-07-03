//! Elixir syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "after", "case", "catch", "cond", "def", "defmodule", "do", "else", "end", "false", "fn",
    "for", "if", "in", "nil", "raise", "receive", "rescue", "true", "try", "unless", "use",
    "when", "with",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const NUMBER_PATTERN: NumberPattern =
    NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore);

/// Static Elixir language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Elixir,
    display_name: "Elixir",
    exact_filenames: &[],
    extensions: &["ex", "exs"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
