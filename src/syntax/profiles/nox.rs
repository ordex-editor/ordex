//! Nox syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "assume", "break", "const", "delete", "else", "ensures", "enum", "extern", "false", "for",
    "fun", "if", "impl", "in", "let", "match", "mut", "new", "null", "requires", "return", "self",
    "struct", "to", "true", "typealias", "use", "where", "while",
];
// `@print`, `@as`, ... name compiler builtins.
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    any_identifier_after('@', KEYWORD_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    nested_block_comment("/*", "*/"),
];
// Strings may span lines; `'` only ever opens a character literal, whose
// escapes include three-digit codes such as `'\065'`.
const STRING_STYLES: &[StringStyle] = &[
    multiline_escaped_delimited_string("\"", "\""),
    escaped_delimited_string("'", "'"),
];
// A type suffix is joined to the literal with `_`, as in `10_u8` or `4.5_f32`.
const INTEGER_SUFFIXES: &[&str] = &[
    "_usize", "_u64", "_u32", "_u16", "_u8", "_i64", "_i32", "_i16", "_i8",
];
const FLOAT_SUFFIXES: &[&str] = &["_f64", "_f32"];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned().with_suffix_pattern(
    NumberSuffixPattern::new()
        .with_integer_exact(INTEGER_SUFFIXES)
        .with_float_exact(FLOAT_SUFFIXES),
);

/// Static Nox language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Nox,
    display_name: "Nox",
    exact_filenames: &[],
    extensions: &["nx"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@~",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
