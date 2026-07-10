//! TypeScript syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "any", "as", "async", "await", "boolean", "break", "case", "catch", "class",
    "const", "continue", "declare", "default", "else", "enum", "export", "extends", "false",
    "finally", "for", "from", "function", "if", "implements", "import", "in", "infer",
    "interface", "keyof", "let", "module", "namespace", "never", "new", "null", "number",
    "readonly", "return", "satisfies", "static", "string", "super", "switch", "this", "throw",
    "true", "try", "type", "typeof", "undefined", "unique", "unknown", "var", "void", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    custom_delimited_string("`", "`", EscapeMode::Backslash, true),
    double_quoted_string(),
    single_quoted_string(),
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const INTEGER_SUFFIXES: &[&str] = &["n"];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .with_suffix_pattern(NumberSuffixPattern::new().with_integer_exact(INTEGER_SUFFIXES));

/// Static TypeScript language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::TypeScript,
    display_name: "TypeScript",
    exact_filenames: &[],
    extensions: &["ts", "tsx"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>`",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
