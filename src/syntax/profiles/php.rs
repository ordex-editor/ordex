//! PHP syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "as", "break", "case", "catch", "class", "const", "continue", "default",
    "do", "echo", "else", "elseif", "extends", "false", "final", "finally", "fn", "for",
    "foreach", "function", "if", "implements", "include", "include_once", "instanceof",
    "interface", "match", "namespace", "new", "null", "private", "protected", "public",
    "require", "require_once", "return", "static", "switch", "throw", "trait", "true", "try",
    "use", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    line_comment("#"),
    block_comment("/*", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code();

/// Static PHP language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Php,
    display_name: "PHP",
    exact_filenames: &[],
    extensions: &["php", "phtml"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
