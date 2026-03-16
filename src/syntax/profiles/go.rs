//! Go syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
    "false", "for", "func", "go", "goto", "if", "import", "interface", "map", "package",
    "range", "return", "select", "struct", "switch", "true", "type", "var",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    custom_delimited_string("`", "`", EscapeMode::None, true),
    double_quoted_string(),
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const INTEGER_SUFFIXES: &[&str] = &["i"];
const FLOAT_SUFFIXES: &[&str] = &["i"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_hex_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(INTEGER_SUFFIXES)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static Go language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Go,
    display_name: "Go",
    exact_filenames: &[],
    extensions: &["go"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
