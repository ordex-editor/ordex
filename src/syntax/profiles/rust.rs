//! Rust syntax profile.

use crate::syntax::profile::*;

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "trait", "true", "type", "unsafe", "use",
    "where", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    doc_line_comment("//!"),
    nested_block_comment("/*", "*/"),
    nested_doc_block_comment("/**", "*/"),
    nested_doc_block_comment("/*!", "*/"),
];
const STRING_STYLES: &[StringStyle] = &[raw_hash_string('r', '#', '"'), double_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const NESTED_HOOKS: &[NestedLanguageHook] = &[];

/// Static Rust language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Rust,
    display_name: "Rust",
    detection: LanguageDetection {
        exact_filenames: &[],
        extensions: &["rs"],
    },
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: SIGNED_NUMBER,
    markdown_rules: None,
    nested_hooks: NESTED_HOOKS,
};
