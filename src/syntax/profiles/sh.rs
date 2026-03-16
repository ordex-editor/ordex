//! POSIX sh syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "if", "in", "then",
    "until", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[
    double_quoted_string(),
    single_quoted_string(),
    custom_delimited_string("`", "`", EscapeMode::Backslash, false),
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[keyword_rule(KEYWORDS)];
const NUMBER_PATTERN: NumberPattern = UNSIGNED_NUMBER.with_digit_separator(DigitSeparator::None);

/// Static POSIX sh language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Sh,
    display_name: "POSIX sh",
    exact_filenames: &[".profile", "profile"],
    extensions: &["sh"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>$",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    nested_hooks: &[],
};
