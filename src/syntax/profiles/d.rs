//! D syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "alias", "auto", "break", "case", "class", "const", "continue", "debug",
    "else", "enum", "false", "foreach", "foreach_reverse", "if", "immutable",
    "import", "in", "interface", "module", "new", "private", "public", "return",
    "shared", "static", "struct", "switch", "template", "this", "true", "void",
    "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[
    preferred_line_comment("//"),
    doc_line_comment("///"),
    block_comment("/*", "*/"),
    doc_block_comment("/**", "*/"),
    nested_block_comment("/+", "+/"),
    nested_doc_block_comment("/++", "+/"),
];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string()];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];

/// Static D language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::D,
    display_name: "D",
    exact_filenames: &[],
    extensions: &["d"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>",
    number_pattern: SIGNED_NUMBER,
    markup_rules: None,
    nested_hooks: &[],
};
