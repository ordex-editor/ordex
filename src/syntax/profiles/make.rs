//! Make syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "define", "else", "endef", "endif", "export", "if", "ifdef", "ifeq", "ifndef", "ifneq",
    "include", "override", "private", "sinclude", "undefine", "unexport", "vpath",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[
    double_quoted_string(),
    single_quoted_string(),
    custom_delimited_string("`", "`", EscapeMode::Backslash, false),
];
const NUMBER_PATTERN: NumberPattern = UNSIGNED_NUMBER.with_digit_separator(DigitSeparator::None);

/// Static Make language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Make,
    display_name: "Make",
    exact_filenames: &["Makefile", "makefile", "GNUmakefile"],
    extensions: &["mk", "mak"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
