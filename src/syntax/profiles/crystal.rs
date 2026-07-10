//! Crystal syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "alias", "as", "asm", "begin", "break", "case", "class", "def", "do", "else",
    "elsif", "end", "enum", "extend", "false", "for", "fun", "if", "in", "include", "lib",
    "macro", "module", "next", "nil", "of", "out", "private", "protected", "require", "rescue",
    "return", "self", "struct", "then", "true", "typeof", "union", "unless", "until", "when",
    "while", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];
const CHAR_STYLES: &[CharStyle] = &[char_literal()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&[
                "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
                "usize",
            ])
            .with_float_exact(&["f32", "f64"]),
    );

/// Static Crystal language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Crystal,
    display_name: "Crystal",
    exact_filenames: &[],
    extensions: &["cr"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: CHAR_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
