//! CoffeeScript syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "and", "break", "by", "catch", "class", "continue", "do", "else", "extends", "false", "for",
    "if", "in", "is", "isnt", "loop", "new", "no", "not", "null", "of", "off", "on", "or",
    "return", "super", "then", "throw", "true", "try", "unless", "until", "when", "while", "yes",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#"), block_comment("###", "###")];
const STRING_STYLES: &[StringStyle] = &[
    triple_double_quoted_string(),
    triple_single_quoted_string(),
    double_quoted_string(),
    single_quoted_string(),
];
const NUMBER_PATTERN: NumberPattern =
    NumberPattern::common_code().with_suffix_pattern(NumberSuffixPattern::none());

/// Static CoffeeScript language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::CoffeeScript,
    display_name: "CoffeeScript",
    exact_filenames: &[],
    extensions: &["coffee", "litcoffee"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: COLON_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
