//! F# syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "abstract", "and", "as", "assert", "base", "begin", "class", "default", "do", "done",
    "downcast", "downto", "elif", "else", "end", "exception", "extern", "false", "finally",
    "for", "fun", "function", "if", "in", "inherit", "inline", "interface", "internal", "let",
    "match", "member", "module", "mutable", "namespace", "new", "null", "of", "open", "or",
    "override", "private", "public", "rec", "return", "static", "struct", "then", "to", "true",
    "try", "type", "upcast", "use", "val", "void", "when", "while", "with", "yield",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("(*", "*)")];
const STRING_STYLES: &[StringStyle] = &[
    triple_double_quoted_string(),
    prefixed_escaped_delimited_string(&["@"], "\"", "\""),
    double_quoted_string(),
];
const FS_TO_FSI: &[&str] = &["fsi"];
const FSI_TO_FS: &[&str] = &["fs"];
const CORRESPONDING_RULES: &[CorrespondingExtensionRule] = &[
    corresponding_extension_rule("fs", FS_TO_FSI),
    corresponding_extension_rule("fsi", FSI_TO_FS),
];
const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Underscore)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(&["y", "uy", "s", "us", "l", "L", "u", "UL", "I"])
            .with_float_exact(&["f", "F", "m", "M"]),
    );

/// Static F# language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::FSharp,
    display_name: "F#",
    exact_filenames: &[],
    extensions: &["fs", "fsi", "fsx"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>@",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};

/// Return ordered corresponding extensions for F# source/interface files.
pub(crate) fn corresponding_extensions(source_extension: &str) -> Option<&'static [&'static str]> {
    lookup_corresponding_extensions(CORRESPONDING_RULES, source_extension)
}
