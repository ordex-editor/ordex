//! Kconfig syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "bool", "choice", "comment", "config", "default", "depends", "endchoice", "endif", "help",
    "hex", "if", "imply", "int", "menu", "menuconfig", "prompt", "range", "select", "source",
    "string", "tristate",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .with_digit_separator(DigitSeparator::None)
    .supports_hex(true);

/// Static Kconfig language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Kconfig,
    display_name: "Kconfig",
    exact_filenames: &["Kconfig", "Kbuild", "Config.in"],
    extensions: &[],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier_with_dashes(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "()[]:,.=<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
