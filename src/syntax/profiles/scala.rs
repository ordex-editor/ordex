//! Scala syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "case", "catch", "class", "def", "do", "else", "enum", "extends", "false", "final", "for",
    "given", "if", "implicit", "import", "lazy", "match", "new", "null", "object", "override",
    "package", "private", "protected", "return", "sealed", "super", "then", "this", "throw",
    "trait", "true", "try", "type", "using", "val", "var", "while", "with", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&["l", "L"])
            .with_float_exact(&["f", "F", "d", "D"]),
    );

/// Static Scala language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Scala,
    display_name: "Scala",
    exact_filenames: &[],
    extensions: &["scala", "sc"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
