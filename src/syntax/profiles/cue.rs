//! CUE syntax profile.

use crate::syntax::profile::*;

const KEYWORDS: &[&str] = &["false", "for", "if", "in", "let", "null", "true"];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[triple_double_quoted_string(), double_quoted_string()];

/// Static CUE language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Cue,
    display_name: "CUE",
    exact_filenames: &[],
    extensions: &["cue"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NumberPattern::common_code(),
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
