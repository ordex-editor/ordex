//! CMake syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "add_executable", "add_library", "cmake_minimum_required", "elseif", "else", "endforeach",
    "endif", "endfunction", "foreach", "function", "if", "include", "macro", "message",
    "project", "return", "set", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[
    multiline_plain_delimited_string("[[", "]]"),
    double_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static CMake language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::CMake,
    display_name: "CMake",
    exact_filenames: &["CMakeLists.txt"],
    extensions: &["cmake"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule_ignore_ascii_case(KEYWORDS)],
    punctuation_chars: "()[]{}:,.=+-*/%$<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
