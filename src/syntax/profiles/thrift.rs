//! Thrift syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "binary", "bool", "byte", "const", "cpp_include", "double", "false", "i16", "i32", "i64",
    "include", "namespace", "optional", "required", "string", "true", "void",
];
#[rustfmt::skip]
const STORAGE_CLASS_WORDS: &[&str] = &[
    "enum", "exception", "list", "map", "service", "set", "struct", "throws", "typedef", "union",
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(STORAGE_CLASS_WORDS, STORAGE_CLASS_STYLE),
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    line_comment("#"),
    block_comment("/*", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];

/// Static Thrift language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Thrift,
    display_name: "Thrift",
    exact_filenames: &[],
    extensions: &["thrift"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code(),
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
