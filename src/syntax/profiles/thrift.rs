//! Thrift syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "binary", "bool", "byte", "const", "cpp_include", "double", "enum", "exception", "false",
    "i16", "i32", "i64", "include", "list", "map", "namespace", "optional", "required",
    "service", "set", "string", "struct", "throws", "true", "typedef", "union", "void",
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
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code(),
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
};
