//! Groovy syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "assert", "break", "case", "catch", "continue", "def", "default", "do", "else",
    "finally", "for", "if", "import", "in", "instanceof", "new", "package", "return", "super",
    "switch", "this", "throw", "trait", "try", "while",
];
const STORAGE_CLASS_WORDS: &[&str] = &["class", "enum", "extends", "implements", "interface"];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "null", "true"];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(STORAGE_CLASS_WORDS, STORAGE_CLASS_STYLE),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
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
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
