//! Git interactive rebase syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("#")];
const STRING_STYLES: &[StringStyle] = &[];
const NUMBER_PATTERN: NumberPattern = no_number_pattern();
#[rustfmt::skip]
const COMMANDS: &[&str] = &[
    "pick", "p", "reword", "r", "edit", "e", "squash", "s", "fixup", "f", "exec", "x",
    "break", "b", "drop", "d", "label", "l", "reset", "t", "merge", "m", "update-ref", "u",
    "noop",
];
const LEX_HOOKS: &[NestedLanguageHook] = &[line_start_command_hook(
    COMMANDS,
    KEYWORD_STYLE,
    Some(hook_next_ascii_hex(7, NUMBER_STYLE)),
    true,
)];

/// Static Git interactive rebase profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::GitRebase,
    display_name: "Git Rebase",
    exact_filenames: &["git-rebase-todo"],
    extensions: &[],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: ascii_identifier_with_dashes(),
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: LEX_HOOKS,
    corresponding_extensions: None,
};
