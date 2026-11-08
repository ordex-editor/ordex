//! Weir syntax profile, covering both Core W (`.w`) and the surface language (`.weir`).

use crate::syntax::profile::*;

// Weir keywords are contextual; words the corpus also uses as ordinary names
// (`new`, `op`, `default`, `drop`, `out`, `keep`, `tail`, `held`, ...) stay plain.
#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "affine", "alias", "any", "as", "break", "caller", "cancel", "canon", "captures", "comptime",
    "const", "cont", "continue", "copy", "cross", "ct", "destroy", "destruct", "discard", "drops",
    "dyn", "effect", "else", "enum", "env", "extern", "fallible", "false", "finally", "fix", "fn",
    "for", "forget", "handler", "heap", "home", "if", "impl", "in", "inside", "invariant", "let",
    "linear", "loop", "match", "move", "movable", "mut", "on_abort", "once", "par", "portable",
    "pub", "raise", "reflect", "region", "requires", "resume", "return", "row", "self", "Self",
    "shared", "shield", "strict", "struct", "terminal", "terminates", "test", "trait", "true",
    "uninit", "unpinned", "unsafe", "use", "while", "with", "wrapping",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//")];
const STRING_STYLES: &[StringStyle] = &[escaped_delimited_string("\"", "\"")];
const CHAR_STYLES: &[CharStyle] = &[prefixed_char_literal(&["b"])];
// A float suffix may follow bare digits too: `1f64` is a float literal.
const INTEGER_SUFFIXES: &[&str] = &[
    "usize", "u64", "u32", "u16", "u8", "isize", "i64", "i32", "i16", "i8", "f64", "f32",
];
const FLOAT_SUFFIXES: &[&str] = &["f64", "f32"];
const NUMBER_PATTERN: NumberPattern = NumberPattern::unsigned()
    .supports_decimal_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_exact(INTEGER_SUFFIXES)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static Weir language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Weir,
    display_name: "Weir",
    exact_filenames: &[],
    extensions: &["weir", "w"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    char_styles: CHAR_STYLES,
    identifier: ascii_identifier(),
    identifier_rules: &[keyword_rule(KEYWORDS)],
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>#",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
