//! PHP syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "as", "break", "case", "catch", "class", "const", "continue", "default",
    "do", "echo", "else", "elseif", "extends", "finally", "fn", "for", "foreach", "function",
    "if", "implements", "include", "include_once", "instanceof", "interface", "match",
    "namespace", "new", "null", "require", "require_once", "return", "switch", "throw",
    "trait", "try", "use", "while",
];
const STORAGE_CLASS_WORDS: &[&str] = &["final", "private", "protected", "public", "static"];
const BUILTIN_VALUE_WORDS: &[&str] = &["false", "true"];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    line_comment("#"),
    block_comment("/*", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    keyword_rule(KEYWORDS),
    exact_words_rule(STORAGE_CLASS_WORDS, STORAGE_CLASS_STYLE),
    exact_words_rule(BUILTIN_VALUE_WORDS, BUILTIN_VALUE_STYLE),
];
pub(crate) const NUMBER_PATTERN: NumberPattern =
    NumberPattern::common_code().with_digit_separator(DigitSeparator::Underscore);

/// Static PHP language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Php,
    display_name: "PHP",
    exact_filenames: &[],
    extensions: &["php", "phtml"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
