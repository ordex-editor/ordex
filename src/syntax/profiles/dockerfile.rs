//! Dockerfile syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "ADD", "ARG", "CMD", "COPY", "ENTRYPOINT", "ENV", "EXPOSE", "FROM", "HEALTHCHECK", "LABEL",
    "ONBUILD", "RUN", "SHELL", "STOPSIGNAL", "USER", "VOLUME", "WORKDIR",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = NumberPattern::signed()
    .with_digit_separator(DigitSeparator::None)
    .supports_decimal_exponent(true);

/// Static Dockerfile language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Dockerfile,
    display_name: "Dockerfile",
    exact_filenames: &["Dockerfile", "Containerfile"],
    extensions: &["dockerfile"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[keyword_rule_ignore_ascii_case(KEYWORDS)],
    punctuation_chars: "[]{}():,.=+-*/%$<>@!",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    manual_indent: NO_MANUAL_INDENT,
    nested_hooks: &[],
};
