//! C++ syntax profile.

use crate::syntax::profile::*;

#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "alignas", "alignof", "auto", "bool", "break", "case", "catch", "char", "class", "const",
    "constexpr", "continue", "decltype", "default", "delete", "do", "double", "else", "enum",
    "explicit", "export", "false", "final", "float", "for", "friend", "goto", "if", "inline",
    "int", "long", "mutable", "namespace", "new", "noexcept", "nullptr", "operator", "override",
    "private", "protected", "public", "return", "short", "signed", "sizeof", "static",
    "struct", "switch", "template", "this", "throw", "true", "try", "typedef", "typename",
    "union", "unsigned", "using", "virtual", "void", "while",
];
const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//"), block_comment("/*", "*/")];
const STRING_STYLES: &[StringStyle] = &[
    cpp_raw_string(&["", "u8", "u", "U", "L"], 16),
    prefixed_escaped_delimited_string(&["", "u8", "u", "U", "L"], "\"", "\""),
];
const PREPROCESSOR_KEYWORDS: &[&str] = &[
    "define",
    "elif",
    "else",
    "endif",
    "error",
    "if",
    "ifdef",
    "ifndef",
    "include",
    "include_next",
    "line",
    "pragma",
    "undef",
    "warning",
];
const IDENTIFIER_RULES: &[IdentifierRule] = &[
    exact_words_after(PREPROCESSOR_KEYWORDS, '#', true, true, PREPROCESSOR_STYLE),
    keyword_rule(KEYWORDS),
];
const INTEGER_SUFFIX_GROUPS: &[NumberSuffixGroup] = &[
    suffix_group(&["u", "U"]),
    suffix_group(&["ll", "LL", "l", "L"]),
];
const FLOAT_SUFFIXES: &[&str] = &["f", "F", "l", "L"];
const CPP_TO_HEADERS: &[&str] = &["h", "hpp", "hh", "hxx"];
const CPP_HEADERS_TO_SOURCE: &[&str] = &["cc", "cpp", "cxx"];
const CORRESPONDING_RULES: &[CorrespondingExtensionRule] = &[
    corresponding_extension_rule("cc", CPP_TO_HEADERS),
    corresponding_extension_rule("cpp", CPP_TO_HEADERS),
    corresponding_extension_rule("cxx", CPP_TO_HEADERS),
    corresponding_extension_rule("h", CPP_HEADERS_TO_SOURCE),
    corresponding_extension_rule("hpp", CPP_HEADERS_TO_SOURCE),
    corresponding_extension_rule("hh", CPP_HEADERS_TO_SOURCE),
    corresponding_extension_rule("hxx", CPP_HEADERS_TO_SOURCE),
];
pub(crate) const NUMBER_PATTERN: NumberPattern = NumberPattern::common_code()
    .with_digit_separator(DigitSeparator::Apostrophe)
    .supports_octal_prefix(false)
    .supports_legacy_octal(true)
    .supports_hex_exponent(true)
    .with_suffix_pattern(
        NumberSuffixPattern::new()
            .with_integer_groups(INTEGER_SUFFIX_GROUPS)
            .with_float_exact(FLOAT_SUFFIXES),
    );

/// Static C++ language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Cpp,
    display_name: "C++",
    exact_filenames: &[],
    extensions: &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
    comment_styles: COMMENT_STYLES,
    string_styles: STRING_STYLES,
    identifier: Some(ascii_identifier()),
    identifier_rules: IDENTIFIER_RULES,
    punctuation_chars: "{}[]();:,.=+-*/%&|^!?<>#",
    number_pattern: NUMBER_PATTERN,
    markup_rules: None,
    indentation: C_LIKE_INDENT,
    nested_hooks: &[],
};

/// Return ordered corresponding extensions for C++ source/header files.
pub(crate) fn corresponding_extensions(source_extension: &str) -> Option<&'static [&'static str]> {
    lookup_corresponding_extensions(CORRESPONDING_RULES, source_extension)
}
