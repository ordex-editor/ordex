//! Python syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def",
    "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "match", "None", "nonlocal", "not", "or", "pass",
    "raise", "return", "True", "try", "while", "with", "yield",
];
const PREFIXES: &[&str] = &[
    "r", "R", "u", "U", "b", "B", "f", "F", "br", "Br", "bR", "BR", "rb", "Rb", "rB", "RB", "fr",
    "Fr", "fR", "FR", "rf", "Rf", "rF", "RF",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[
    prefixed_multiline_escaped_delimited_string(PREFIXES, "\"\"\"", "\"\"\""),
    prefixed_multiline_escaped_delimited_string(PREFIXES, "'''", "'''"),
    triple_double_quoted_string(),
    triple_single_quoted_string(),
    prefixed_escaped_delimited_string(PREFIXES, "\"", "\""),
    prefixed_escaped_delimited_string(PREFIXES, "'", "'"),
    double_quoted_string(),
    single_quoted_string(),
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const INTEGER_SUFFIXES: &[&str] = &["j", "J"];
const FLOAT_SUFFIXES: &[&str] = &["j", "J"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code().with_suffix_pattern(
    NumberSuffixPattern::new()
        .with_integer_exact(INTEGER_SUFFIXES)
        .with_float_exact(FLOAT_SUFFIXES),
);

/// Static Python language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Python,
    display_name: "Python",
    exact_filenames: &[],
    extensions: &["py", "pyi"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
