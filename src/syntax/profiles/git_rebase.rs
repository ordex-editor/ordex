//! Git interactive rebase syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[double_quoted_string(), single_quoted_string()];
const NUMBER_PATTERN: NumberPattern = UNSIGNED_NUMBER.with_digit_separator(DigitSeparator::None);

/// Static Git interactive rebase profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::GitRebase,
    display_name: "Git Rebase",
    exact_filenames: &["git-rebase-todo"],
    extensions: &[],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier_with_dashes()),
    identifier_rules: &[],
    punctuation_chars: "()[]{}.,:;=+-*/%&|^!?<>@~",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
