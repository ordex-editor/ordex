//! Groovy syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "assert", "break", "case", "catch", "class", "continue", "def", "default", "do",
    "else", "enum", "extends", "false", "finally", "for", "if", "implements", "import", "in",
    "instanceof", "interface", "new", "null", "package", "return", "super", "switch", "this",
    "throw", "trait", "true", "try", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    triple_double_quoted_string(),
    triple_single_quoted_string(),
    double_quoted_string(),
    single_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&["g", "G", "i", "I", "l", "L"])
            .with_float_exact(&["g", "G", "d", "D", "f", "F"]),
    );

/// Static Groovy language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Groovy,
    display_name: "Groovy",
    exact_filenames: &["Jenkinsfile"],
    extensions: &["groovy", "gradle"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
