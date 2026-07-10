//! Erlang syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "after", "andalso", "band", "begin", "bnot", "bor", "bsl", "bsr", "bxor", "case", "catch",
    "cond", "div", "end", "false", "fun", "if", "let", "not", "of", "or", "orelse", "receive",
    "rem", "true", "try", "when", "xor",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("%")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const ERL_TO_HRL: &[&str] = &["hrl"];
const HRL_TO_ERL: &[&str] = &["erl"];
const CORRESPONDING_RULES: &[CorrespondingExtensionRule] = &[
    corresponding_extension_rule("erl", ERL_TO_HRL),
    corresponding_extension_rule("hrl", HRL_TO_ERL),
];

/// Static Erlang language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Erlang,
    display_name: "Erlang",
    exact_filenames: &[],
    extensions: &["erl", "hrl"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@#",
    number_pattern: NumberPattern::common_code(),
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: Some(CORRESPONDING_RULES),
};
