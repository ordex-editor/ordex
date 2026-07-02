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
    assert_token_is_highlighted(
        LanguageId::Rust,
        r#"let value = "unterminated;"#,
        r#""unterminated;"#,
        SyntaxClass::String,
    );
}

/// Verify shell-family string and number grammars stay exact.
#[test]
fn test_shell_family_numbers_and_strings_are_exact() {
    for language in [
        LanguageId::Bash,
        LanguageId::Sh,
        LanguageId::Zsh,
        LanguageId::Pkgbuild,
    ] {
        for literal in [r#""hello""#, "'hello'", "`date`"] {
            assert_token_is_highlighted(
                language,
                &format!("value={literal}"),
                literal,
                SyntaxClass::String,
            );
        }
        assert_token_is_highlighted(language, "value=42", "42", SyntaxClass::Number);
        assert_fragment_is_not_highlighted(
            language,
            r#"value="unterminated"#,
            r#""unterminated"#,
            SyntaxClass::String,
        );
    }

    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Fish,
            &format!("set value {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_token_is_highlighted(
        LanguageId::Fish,
        "set value -42",
        "-42",
        SyntaxClass::Number,
    );
}

/// Verify data-format string and number grammars stay exact.
#[test]
fn test_data_format_numbers_and_strings_are_exact() {
    for language in [LanguageId::Json, LanguageId::JsonC] {
        assert_token_is_highlighted(
            language,
            r#"{"value": "hello"}"#,
            r#""hello""#,
            SyntaxClass::String,
        );
        assert_token_is_highlighted(
            language,
            r#"{"value": -42.5e2}"#,
            "-42.5e2",
            SyntaxClass::Number,
        );
        assert_fragment_is_not_highlighted(language, r#"{"value": .5}"#, ".", SyntaxClass::Number);
    }

    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Yaml,
            &format!("value: {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0b1010", "0o755", "0x1f", "1e6"] {
        assert_token_is_highlighted(
            LanguageId::Yaml,
            &format!("value: {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Yaml, "value: 1_2", "_2", SyntaxClass::Number);

    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Ini,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_token_is_highlighted(LanguageId::Ini, "value = 42", "42", SyntaxClass::Number);
    assert_fragment_is_not_highlighted(LanguageId::Ini, "value = 0x10", "x", SyntaxClass::Number);

    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Xml,
            &format!(r#"<tag value={literal} />"#),
            literal,
            SyntaxClass::String,
        );
    }
    assert_token_is_highlighted(
        LanguageId::Xml,
        "<tag width=42 />",
        "42",
        SyntaxClass::Number,
    );

    for literal in [r#""hello""#, r#""""hello""""#] {
        assert_token_is_highlighted(
            LanguageId::GraphQl,
            &format!("query {{ field(arg: {literal}) }}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["-42", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::GraphQl,
            &format!("query {{ field(arg: {literal}) }}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::GraphQl,
        "query { field(arg: .5) }",
        ".",
        SyntaxClass::Number,
    );
}

/// Verify stylesheet string and number grammars stay exact.
#[test]
fn test_stylesheet_numbers_and_strings_are_exact() {
    for language in [
        LanguageId::Css,
        LanguageId::Scss,
        LanguageId::Less,
        LanguageId::Sass,
    ] {
        for literal in [r#""hello""#, "'hello'"] {
            assert_token_is_highlighted(
                language,
                &format!("content: {literal};"),
                literal,
                SyntaxClass::String,
            );
        }
        for literal in [".5", "-12"] {
            assert_token_is_highlighted(
                language,
                &format!("width: {literal}rem;"),
                literal,
                SyntaxClass::Number,
            );
        }
        assert_fragment_is_not_highlighted(language, "width: 0x10rem;", "x", SyntaxClass::Number);
    }
}

/// Verify build and config profile literals stay exact.
#[test]
fn test_build_and_config_numbers_and_strings_are_exact() {
    for (language, line, string_literal, number_literal) in [
        (
            LanguageId::CMake,
            r#"set(VALUE [[hello]])"#,
            "[[hello]]",
            "42",
        ),
        (
            LanguageId::Meson,
            "value = '''hello'''",
            "'''hello'''",
            "42",
        ),
        (LanguageId::Ninja, r#"value = "hello""#, r#""hello""#, "42"),
        (
            LanguageId::Dockerfile,
            r#"RUN echo "hello" 42"#,
            r#""hello""#,
            "42",
        ),
        (LanguageId::Hcl, r#"value = "hello""#, r#""hello""#, "42.5"),
        (LanguageId::Nix, "value = ''hello'';", "''hello''", "42.5"),
        (
            LanguageId::Kconfig,
            r#"default "hello""#,
            r#""hello""#,
            "0x1f",
        ),
        (
            LanguageId::Cue,
            r#"value: """hello""""#,
            r#""""hello""""#,
            "1.5e6",
        ),
    ] {
        assert_token_is_highlighted(language, line, string_literal, SyntaxClass::String);
        assert_token_is_highlighted(
            language,
            &line.replace(string_literal, number_literal),
            number_literal,
            SyntaxClass::Number,
        );
    }

    assert_fragment_is_not_highlighted(LanguageId::Ninja, "value = 0x10", "x", SyntaxClass::Number);
    assert_fragment_is_not_highlighted(
        LanguageId::Dockerfile,
        r#"RUN echo "oops"#,
        r#""oops"#,
        SyntaxClass::String,
    );

    for literal in [r#""hello""#, "'hello'", "`date`"] {
        assert_token_is_highlighted(
            LanguageId::Make,
            &format!("VALUE = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    assert_token_is_highlighted(LanguageId::Make, "JOBS = 42", "42", SyntaxClass::Number);
}

/// Verify scripting-language string and number grammars stay exact.
#[test]
fn test_scripting_language_numbers_and_strings_are_exact() {
    for literal in ["[[hello]]", r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Lua,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["42", "0x1.fp2"] {
        assert_token_is_highlighted(
            LanguageId::Lua,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Lua, "value = 0b10", "b", SyntaxClass::Number);

    for language in [LanguageId::Ruby, LanguageId::Perl] {
        for literal in [r#""hello""#, "'hello'"] {
            assert_token_is_highlighted(
                language,
                &format!("value = {literal}"),
                literal,
                SyntaxClass::String,
            );
        }
        for literal in ["0b1010", "0o755", "0x1f", "1.5e6"] {
            assert_token_is_highlighted(
                language,
                &format!("value = {literal}"),
                literal,
                SyntaxClass::Number,
            );
        }
        assert_fragment_is_not_highlighted(language, "value = 1__2", "__2", SyntaxClass::Number);
    }

    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::R,
            &format!("value <- {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["42L", "2i", "0x1.fp2"] {
        assert_token_is_highlighted(
            LanguageId::R,
            &format!("value <- {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::R, "value <- 0b10", "b", SyntaxClass::Number);

    assert_token_is_highlighted(
        LanguageId::Elixir,
        r#"value = """hello""""#,
        r#""""hello""""#,
        SyntaxClass::String,
    );
    for literal in [r#""hello""#, "0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Elixir,
            &format!("value = {literal}"),
            literal,
            if literal.starts_with('"') {
                SyntaxClass::String
            } else {
                SyntaxClass::Number
            },
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Elixir,
        "value = 1__2",
        "__2",
        SyntaxClass::Number,
    );

    assert_token_is_highlighted(
        LanguageId::Awk,
        r#"value = "hello""#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in ["-42", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Awk,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Awk, "value = 0x1f", "x", SyntaxClass::Number);

    for literal in [r#"'''hello'''"#, r#""""hello""""#, r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Nim,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Nim,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#""""hello""""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::Crystal,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["42i32", "3.14f64", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::Crystal,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#"'''hello'''"#, r#""""hello""""#, r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::CoffeeScript,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in [".5", "1.", "0b1010", "0o755", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::CoffeeScript,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
}

/// Verify compiled-language string and number grammars stay exact.
#[test]
fn test_compiled_language_numbers_and_strings_are_exact() {
    for literal in [r##"#"hello"#"##, r#""""hello""""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::Swift,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0b1010", "0x1.fp2", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Swift,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#""""hello""""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::Kotlin,
            &format!("val value = {literal}"),
            literal,
            SyntaxClass::String,
        );
        assert_token_is_highlighted(
            LanguageId::Scala,
            &format!("val value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["42u", "42L", "1.5f", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::Kotlin,
            &format!("val value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Kotlin,
        "val value = 0o755",
        "o",
        SyntaxClass::Number,
    );
    for literal in ["42L", "1.5F", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::Scala,
            &format!("val value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#"'''hello'''"#, r#""""hello""""#, r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Groovy,
            &format!("def value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["42G", "1.5D", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::Groovy,
            &format!("def value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#"r"hello""#, r#""""hello""""#, r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Dart,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Dart,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Dart,
        "var value = 0b10;",
        "b",
        SyntaxClass::Number,
    );

    assert_token_is_highlighted(
        LanguageId::Zig,
        r#"const value = c"hello";"#,
        r#"c"hello""#,
        SyntaxClass::String,
    );
    for literal in ["0b1010", "0o755", "0x1.fp2", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Zig,
            &format!("const value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }

    for language in [LanguageId::Solidity, LanguageId::Qml] {
        for literal in [r#""hello""#, "'hello'"] {
            assert_token_is_highlighted(
                language,
                &format!("value = {literal};"),
                literal,
                SyntaxClass::String,
            );
        }
    }
    for literal in ["0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Solidity,
            &format!("value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    for literal in ["0b1010", "0o755", "0x1f", ".5"] {
        assert_token_is_highlighted(
            LanguageId::Qml,
            &format!("value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_token_is_highlighted(
        LanguageId::Qml,
        "value = `hello`;",
        "`hello`",
        SyntaxClass::String,
    );

    for literal in [r#""""hello""""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::Vala,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Vala,
            &format!("var value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }

    for language in [
        LanguageId::Gas,
        LanguageId::Nasm,
        LanguageId::Masm,
        LanguageId::Yasm,
    ] {
        assert_token_is_highlighted(language, r#"db "hello""#, r#""hello""#, SyntaxClass::String);
    }
    for literal in ["42", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::Gas,
            &format!(".long {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Gas, ".long 0b10", "b", SyntaxClass::Number);
    for language in [LanguageId::Nasm, LanguageId::Masm, LanguageId::Yasm] {
        for literal in ["42h", "101b", "0x1f"] {
            assert_token_is_highlighted(
                language,
                &format!("value {literal}"),
                literal,
                SyntaxClass::Number,
            );
        }
    }
}

/// Verify functional-language string and number grammars stay exact.
#[test]
fn test_functional_language_numbers_and_strings_are_exact() {
    assert_token_is_highlighted(
        LanguageId::Erlang,
        r#"Value = "hello"."#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in ["42", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Erlang,
            &format!("Value = {literal}."),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#""""hello""""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::Elm,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Elm,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#"raw"hello""#, r#""""hello""""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::Julia,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0b1010", "0o755", "0x1f", "2im"] {
        assert_token_is_highlighted(
            LanguageId::Julia,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    assert_token_is_highlighted(
        LanguageId::Haskell,
        r#"value = "hello""#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in ["0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Haskell,
            &format!("value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(
        LanguageId::Haskell,
        "value = 0b10",
        "b",
        SyntaxClass::Number,
    );

    assert_token_is_highlighted(
        LanguageId::Ocaml,
        r#"let value = "hello""#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in ["0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Ocaml,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#""""hello""""#, r#"@"hello""#, r#""hello""#] {
        assert_token_is_highlighted(
            LanguageId::FSharp,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["42y", "42UL", "1.5M", "0x1f"] {
        assert_token_is_highlighted(
            LanguageId::FSharp,
            &format!("let value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    assert_token_is_highlighted(
        LanguageId::Lisp,
        r#"(print "hello")"#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in ["-42", "0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Lisp,
            &format!("(setf value {literal})"),
            literal,
            SyntaxClass::Number,
        );
    }
}

/// Verify HTML-family string and number grammars stay exact.
#[test]
fn test_html_family_numbers_and_strings_are_exact() {
    for language in [LanguageId::Html, LanguageId::Xhtml] {
        for literal in [r#""hello""#, "'hello'"] {
            assert_token_is_highlighted(
                language,
                &format!(r#"<tag value={literal} width=42 />"#),
                literal,
                SyntaxClass::String,
            );
        }
        assert_token_is_highlighted(language, r#"<tag width=42 />"#, "42", SyntaxClass::Number);
        assert_fragment_is_not_highlighted(
            language,
            r#"<tag value="unterminated>"#,
            r#""unterminated"#,
            SyntaxClass::String,
        );
    }
}

/// Verify schema and query-language string and number grammars stay exact.
#[test]
fn test_schema_language_numbers_and_strings_are_exact() {
    assert_token_is_highlighted(
        LanguageId::Proto,
        r#"string value = "hello";"#,
        r#""hello""#,
        SyntaxClass::String,
    );
    for literal in ["0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Proto,
            &format!("option value = {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#""hello""#, "'hello'"] {
        assert_token_is_highlighted(
            LanguageId::Thrift,
            &format!("const string value = {literal}"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["0b1010", "0o755", "0x1f", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Thrift,
            &format!("const i64 value = {literal}"),
            literal,
            SyntaxClass::Number,
        );
    }

    for literal in [r#"'hello'"#, r#""name""#] {
        assert_token_is_highlighted(
            LanguageId::Sql,
            &format!("SELECT {literal};"),
            literal,
            SyntaxClass::String,
        );
    }
    for literal in ["-42", "1.5e6"] {
        assert_token_is_highlighted(
            LanguageId::Sql,
            &format!("SELECT {literal};"),
            literal,
            SyntaxClass::Number,
        );
    }
    assert_fragment_is_not_highlighted(LanguageId::Sql, "SELECT 0x1f;", "x", SyntaxClass::Number);
}

/// Verify TODO/FIXME markers are highlighted in comments and plain text.
#[test]
fn test_todo_markers_are_highlighted() {
    use crate::syntax::profile::SyntaxModifier;

    let prof = profile(LanguageId::Rust);
    let line = "// TODO: fix this";
    let parsed = lex_profile_line(prof, line, LineLexMode::Plain);
    let todo_span = parsed
        .spans
        .iter()
        .find(|s| s.modifier == Some(SyntaxModifier::Todo));
    assert!(todo_span.is_some(), "TODO should be highlighted");
    let todo_span = todo_span.unwrap();
    assert_eq!(&line[todo_span.start_col..todo_span.end_col], "TODO");

    let line = "/* FIXME */";
    let parsed = lex_profile_line(prof, line, LineLexMode::Plain);
    let fixme_span = parsed
        .spans
        .iter()
        .find(|s| s.modifier == Some(SyntaxModifier::Todo));
    assert!(fixme_span.is_some(), "FIXME should be highlighted");
    let fixme_span = fixme_span.unwrap();
    assert_eq!(&line[fixme_span.start_col..fixme_span.end_col], "FIXME");

    let line = "let x = \"TODO\";";
    let parsed = lex_profile_line(prof, line, LineLexMode::Plain);
    let todo_span = parsed
        .spans
        .iter()
        .find(|s| s.modifier == Some(SyntaxModifier::Todo));
    assert!(
        todo_span.is_none(),
        "TODO in string should not be highlighted as a marker"
    );

    let line = "// TODOS";
    let parsed = lex_profile_line(prof, line, LineLexMode::Plain);
    let todo_span = parsed
        .spans
        .iter()
        .find(|s| s.modifier == Some(SyntaxModifier::Todo));
    assert!(
        todo_span.is_none(),
        "TODOS should not be highlighted due to word boundaries"
    );

    let asciidoc_profile = profile(LanguageId::AsciiDoc);
    let line = "TODO: write intro";
    let parsed = lex_profile_line(asciidoc_profile, line, LineLexMode::Plain);
    let todo_span = parsed
        .spans
        .iter()
        .find(|s| s.modifier == Some(SyntaxModifier::Todo));
    assert!(
        todo_span.is_some(),
        "TODO in plain markup text should be highlighted"
    );
}
