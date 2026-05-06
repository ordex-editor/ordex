//! Rust syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "trait",
    "true", "type", "unsafe", "use", "where", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    doc_line_comment("//!"),
    nested_block_comment("/*", "*/"),
    nested_doc_block_comment("/**", "*/"),
    nested_doc_block_comment("/*!", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[
    raw_hash_string(&["r", "br"], '#', '"'),
    prefixed_escaped_delimited_string(&["b"], "\"", "\""),
    double_quoted_string(),
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const INTEGER_SUFFIXES: &[&str] = &[
    "usize", "u128", "u64", "u32", "u16", "u8", "isize", "i128", "i64", "i32", "i16", "i8",
];
const FLOAT_SUFFIXES: &[&str] = &["f64", "f32"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .supports_leading_dot(false)
    .supports_trailing_dot(false)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(INTEGER_SUFFIXES)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static Rust language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Rust,
    display_name: "Rust",
    exact_filenames: &[],
    extensions: &["rs"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
};
