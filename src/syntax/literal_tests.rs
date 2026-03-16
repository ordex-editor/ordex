//! Focused literal-grammar tests for built-in syntax profiles.

use crate::syntax::engine::{HighlightSpan, LineLexMode, lex_profile_line};
use crate::syntax::profile::{LanguageId, LanguageProfile, SyntaxClass};
use crate::syntax::profiles::builtin_profiles;

/// Return one built-in profile by language identifier.
fn profile(language: LanguageId) -> &'static LanguageProfile {
    builtin_profiles()
        .iter()
        .find(|profile| profile.id == language)
        .expect("language profile should exist")
}

/// Return the first span of `class` that covers `column`.
fn covering_span(
    spans: &[HighlightSpan],
    column: usize,
    class: SyntaxClass,
) -> Option<&HighlightSpan> {
    spans
        .iter()
        .find(|span| span.class == class && span.covers(column))
}

/// Assert that the first occurrence of `token` is fully highlighted as `class`.
#[track_caller]
fn assert_token_is_highlighted(language: LanguageId, line: &str, token: &str, class: SyntaxClass) {
    let parsed = lex_profile_line(profile(language), line, LineLexMode::Plain);
    let start = line.find(token).expect("find token");
    let end = start + token.chars().count();
    let span = covering_span(&parsed.spans, start, class).expect("find covering span");
    assert!(
        span.start_col <= start && span.end_col >= end,
        "expected `{token}` in `{line}` to be fully highlighted as {class:?}, got {span:?}"
    );
}

/// Assert that the first occurrence of `fragment` is not highlighted as `class`.
#[track_caller]
fn assert_fragment_is_not_highlighted(
    language: LanguageId,
    line: &str,
    fragment: &str,
    class: SyntaxClass,
) {
    let parsed = lex_profile_line(profile(language), line, LineLexMode::Plain);
    let start = line.find(fragment).expect("find fragment");
    assert!(
        covering_span(&parsed.spans, start, class).is_none(),
        "expected `{fragment}` in `{line}` to stay plain for {class:?}, got {:?}",
        parsed.spans
    );
}

/// Verify JavaScript and TypeScript number grammars stay exact.
#[test]
fn test_javascript_and_typescript_numbers_are_exact() {
    for language in [LanguageId::JavaScript, LanguageId::TypeScript] {
        for literal in ["0b1010", "0o755", "0xff", ".25", "1.", "1e6", "123n"] {
            assert_token_is_highlighted(
                language,
                &format!("const value = {literal};"),
                literal,
                SyntaxClass::Number,
            );
        }
        assert_fragment_is_not_highlighted(language, "const value = -1;", "-", SyntaxClass::Number);
        assert_fragment_is_not_highlighted(
            language,
            "const value = 1__2;",
            "__2",
            SyntaxClass::Number,
        );
        assert_fragment_is_not_highlighted(language, "const value = 0x;", "x", SyntaxClass::Number);
    }
}

/// Verify Python number grammars stay exact.
#[test]
fn test_python_numbers_are_exact() {
    for literal in ["0b1010", "0o755", "0xff", ".25", "1e6", "2j"] {
        assert_token_is_highlighted(
            LanguageId::Python,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Python,
        "value = 1__2",
        "__2",
        SyntaxClass::Number,
    );
    assert_fragment_is_not_highlighted(LanguageId::Python, "value = 0x", "x", SyntaxClass::Number);
}

/// Verify Java number grammars stay exact.
#[test]
fn test_java_numbers_are_exact() {
    for literal in ["0123", "0b1010L", "0x1.fp2", "1.0f", ".5d"] {
        assert_token_is_highlighted(
            LanguageId::Java,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Java,
        "var value = 123n;",
        "n",
        SyntaxClass::Number,
    );
}

/// Verify C# number grammars stay exact.
#[test]
fn test_csharp_numbers_are_exact() {
    for literal in ["0b1010u", "0xFF_FF", "1.5m", ".5f"] {
        assert_token_is_highlighted(
            LanguageId::CSharp,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::CSharp,
        "var value = 1ll;",
        "l;",
        SyntaxClass::Number,
    );
}

/// Verify C and C++ number grammars stay exact.
#[test]
fn test_c_and_cpp_numbers_are_exact() {
    for literal in ["0x1.fp2f", "1'000"] {
        assert_token_is_highlighted(
            LanguageId::Cpp,
            &format!("auto value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Cpp,
        "auto value = 0x1.fp;",
        "p",
        SyntaxClass::Number,
    );

    for literal in ["077", "0x1.fp2", ".5", "1.0L"] {
        assert_token_is_highlighted(
            LanguageId::C,
            &format!("double value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::C,
        "double value = 0b10;",
        "b",
        SyntaxClass::Number,
    );
}

/// Verify D, Go, PHP, Rust, and TOML number grammars stay exact.
#[test]
fn test_d_go_php_rust_and_toml_numbers_are_exact() {
    for literal in ["1UL", "0x1p2L", "2Fi", "3i"] {
        assert_token_is_highlighted(
            LanguageId::D,
            &format!("value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::D, "value = 1l;", "l", SyntaxClass::Number);

    for literal in ["0o755", "0x1.fp2", "12.5i"] {
        assert_token_is_highlighted(
            LanguageId::Go,
            &format!("value := {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Go, "value := 1e+", "e", SyntaxClass::Number);

    for literal in ["0b1010", "0o755", "1_000.5", ".5"] {
        assert_token_is_highlighted(
            LanguageId::Php,
            &format!("$value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Php, "$value = 1n;", "n", SyntaxClass::Number);

    for literal in ["42usize", "3.14f64", "0xff", "1e6f32"] {
        assert_token_is_highlighted(
            LanguageId::Rust,
            &format!("let value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Rust,
        "let value = .5;",
        ".",
        SyntaxClass::Number,
    );

    for literal in ["-42", "0o755", "1e6"] {
        assert_token_is_highlighted(
            LanguageId::Toml,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Toml,
        "value = 1__2",
        "__2",
        SyntaxClass::Number,
    );
}

/// Verify JavaScript and TypeScript string grammars stay exact.
#[test]
fn test_javascript_and_typescript_strings_are_exact() {
    for language in [LanguageId::JavaScript, LanguageId::TypeScript] {
        for literal in [r#""hello""#, "'hello'", "`hello ${name}`"] {
            assert_token_is_highlighted(
                language,
                &format!("const value = {literal};"),
                literal,
                SyntaxClass::String,
            );
        }
        assert_fragment_is_not_highlighted(
            language,
            r#"const value = "unterminated;"#,
            r#""unterminated"#,
            SyntaxClass::String,
        );
    }
}

/// Verify Python string grammars stay exact.
#[test]
fn test_python_strings_are_exact() {
    for literal in [r#""hello""#, "'hello'", r#"rf"hello""#, "b'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Python,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_token_is_highlighted(
        LanguageId::Python,
        r#"value = """hello""""#,
        r#""""hello""""#,
        SyntaxClass::String,
    );
    assert_fragment_is_not_highlighted(
        LanguageId::Python,
        r#"value = "unterminated"#,
        r#""unterminated"#,
        SyntaxClass::String,
    );
}

/// Verify Java and C# string grammars stay exact.
#[test]
fn test_java_and_csharp_strings_are_exact() {
    for literal in [r#""hello""#, r#""""hello""""#] {
        assert_token_is_highlighted(
            LanguageId::Java,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in [r#""hello""#, r#"@"a""b""#, r#""""hello""""#] {
        assert_token_is_highlighted(
            LanguageId::CSharp,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::CSharp,
        r#"var value = "unterminated;"#,
        r#""unterminated"#,
        SyntaxClass::String,
    );
}

/// Verify C, C++, Go, PHP, and Rust string grammars stay exact.
#[test]
fn test_c_family_go_php_and_rust_strings_are_exact() {
    for literal in [r#""hello""#, r#"u8"hello""#, r#"R"tag(raw)tag""#] {
        assert_token_is_highlighted(
            LanguageId::Cpp,
            &format!("auto value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_token_is_highlighted(
        LanguageId::C,
        r#"const char *value = "hello";"#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in [r#""hello""#, "`hello`"] {
        assert_token_is_highlighted(
            LanguageId::Go,
            &format!("value := {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Php,
            &format!("$value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in [r#""hello""#, r#"b"hello""#, r##"r#"hello"#"##] {
        assert_token_is_highlighted(
            LanguageId::Rust,
            &format!("let value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Rust,
        r#"let value = "unterminated;"#,
        r#""unterminated"#,
        SyntaxClass::String,
    );
}
