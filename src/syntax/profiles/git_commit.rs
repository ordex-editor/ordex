//! Git commit-message syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[];
const NUMBER_PATTERN: NumberPattern = no_number_pattern();
const LEX_HOOKS: &[NestedLanguageHook] = &[non_empty_line_at_hook(
    1,
    SpanStyle::new(SyntaxClass::Keyword, Some(SyntaxModifier::Invalid)),
    true,
)];

/// Static Git commit-message profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::GitCommit,
    display_name: "Git Commit",
    exact_filenames: &[
        "COMMIT_EDITMSG",
        "MERGE_MSG",
        "TAG_EDITMSG",
        "NOTES_EDITMSG",
        "EDIT_DESCRIPTION",
        "SQUASH_MSG",
        "REVERT_HEAD",
        "CHERRY_PICK_HEAD",
    ],
    extensions: &[],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: &[],
    identifier: ascii_identifier_with_dashes(),
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: LEX_HOOKS,
    corresponding_extensions: None,
};
