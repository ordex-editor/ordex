//! Ruby syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "BEGIN", "END", "alias", "and", "begin", "break", "case", "class", "def", "defined?",
    "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next",
    "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then", "true",
    "undef", "unless", "until", "when", "while", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_binary(true)
    .supports_octal_prefix(true)
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Static Ruby language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Ruby,
    display_name: "Ruby",
    exact_filenames: &["Gemfile", "Rakefile"],
    extensions: &["rb", "rake"],
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
