//! Shared syntax profile tests.

use crate::syntax::engine::{LineLexMode, lex_profile_line};
use crate::syntax::profile::*;
use crate::syntax::profiles::{builtin_profiles, detect_language_details};
use std::path::Path;

/// Verify exact filename detection wins over extension fallback.
#[test]
fn test_detect_language_prefers_exact_filename() {
    let profile = detect_language_details(Some(Path::new("Cargo.toml")))
        .map(|(profile, _)| profile)
        .expect("detect Cargo.toml");
    assert_eq!(profile.id, LanguageId::Toml);
}

/// Verify unsupported files fall back to no profile.
#[test]
fn test_detect_language_returns_none_for_unsupported_paths() {
    assert!(detect_language_details(Some(Path::new("notes.txt"))).is_none());
}

/// Verify `.cfg` files use the shared TOML-like profile by extension.
#[test]
fn test_detect_language_matches_cfg_extension() {
    let profile = detect_language_details(Some(Path::new("config.cfg")))
        .map(|(profile, _)| profile)
        .expect("detect config.cfg");
    assert_eq!(profile.id, LanguageId::Toml);
}

/// Verify the built-in profile registry contains the full language set.
#[test]
fn test_builtin_profile_count_matches_supported_languages() {
    assert_eq!(builtin_profiles().len(), 72);
}

/// Verify the expanded language set is detected by representative paths.
#[test]
fn test_detect_language_matches_supported_language_paths() {
    let cases = [
        ("sample.js", LanguageId::JavaScript),
        ("sample.ts", LanguageId::TypeScript),
        ("sample.py", LanguageId::Python),
        ("sample.java", LanguageId::Java),
        ("sample.cs", LanguageId::CSharp),
        ("sample.cpp", LanguageId::Cpp),
        ("sample.go", LanguageId::Go),
        ("sample.c", LanguageId::C),
        ("sample.php", LanguageId::Php),
        ("sample.adoc", LanguageId::AsciiDoc),
        (".bashrc", LanguageId::Bash),
        ("script.sh", LanguageId::Sh),
        (".zshrc", LanguageId::Zsh),
        ("config.fish", LanguageId::Fish),
        ("sample.json", LanguageId::Json),
        ("sample.jsonc", LanguageId::JsonC),
        ("sample.yaml", LanguageId::Yaml),
        ("sample.ini", LanguageId::Ini),
        ("sample.css", LanguageId::Css),
        ("sample.scss", LanguageId::Scss),
        ("sample.less", LanguageId::Less),
        ("sample.xml", LanguageId::Xml),
        ("sample.proto", LanguageId::Proto),
        ("sample.thrift", LanguageId::Thrift),
        ("sample.erl", LanguageId::Erlang),
        ("sample.elm", LanguageId::Elm),
        ("CMakeLists.txt", LanguageId::CMake),
        ("meson.build", LanguageId::Meson),
        ("build.ninja", LanguageId::Ninja),
        ("Dockerfile", LanguageId::Dockerfile),
        ("sample.tf", LanguageId::Hcl),
        ("flake.nix", LanguageId::Nix),
        ("Kconfig", LanguageId::Kconfig),
        ("PKGBUILD", LanguageId::Pkgbuild),
        ("sample.lua", LanguageId::Lua),
        ("Gemfile", LanguageId::Ruby),
        ("sample.swift", LanguageId::Swift),
        ("sample.kt", LanguageId::Kotlin),
        ("sample.scala", LanguageId::Scala),
        ("sample.R", LanguageId::R),
        ("sample.sql", LanguageId::Sql),
        ("sample.zig", LanguageId::Zig),
        ("sample.jl", LanguageId::Julia),
        ("sample.hs", LanguageId::Haskell),
        ("sample.ml", LanguageId::Ocaml),
        ("sample.fs", LanguageId::FSharp),
        ("sample.ex", LanguageId::Elixir),
        ("Jenkinsfile", LanguageId::Groovy),
        ("sample.dart", LanguageId::Dart),
        ("sample.pl", LanguageId::Perl),
        ("sample.awk", LanguageId::Awk),
        ("sample.sol", LanguageId::Solidity),
        ("sample.vala", LanguageId::Vala),
        ("sample.nim", LanguageId::Nim),
        ("sample.cr", LanguageId::Crystal),
        ("sample.coffee", LanguageId::CoffeeScript),
        ("sample.graphql", LanguageId::GraphQl),
        ("sample.cue", LanguageId::Cue),
        ("sample.sass", LanguageId::Sass),
        ("sample.qml", LanguageId::Qml),
        ("Makefile", LanguageId::Make),
        ("sample.html", LanguageId::Html),
        ("sample.xhtml", LanguageId::Xhtml),
        ("sample.s", LanguageId::Gas),
        ("sample.nasm", LanguageId::Nasm),
        ("sample.masm", LanguageId::Masm),
        ("sample.yasm", LanguageId::Yasm),
        ("sample.lisp", LanguageId::Lisp),
    ];
    for (path, expected) in cases {
        let profile = detect_language_details(Some(Path::new(path)))
            .map(|(profile, _)| profile)
            .expect("detect representative extension");
        assert_eq!(profile.id, expected, "unexpected profile for {path}");
    }
}

/// Verify D exposes exactly one preferred ordinary comment style.
#[test]
fn test_d_has_one_preferred_ordinary_comment_style() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::D)
        .expect("find D profile");
    let preferred = profile
        .comment_styles
        .iter()
        .filter(|style| style.flavor == CommentFlavor::Ordinary && style.preferred_default)
        .find(|style| style.open == "//");
    assert!(
        preferred.is_some(),
        "D should prefer // for ordinary comments"
    );
    assert_eq!(
        profile
            .comment_styles
            .iter()
            .filter(|style| style.flavor == CommentFlavor::Ordinary && style.preferred_default)
            .count(),
        1
    );
}

/// Verify Rust and D expose documentation comment metadata.
#[test]
fn test_doc_comment_metadata_exists_for_rust_and_d() {
    for language in [LanguageId::Rust, LanguageId::D] {
        let profile = builtin_profiles()
            .iter()
            .find(|profile| profile.id == language)
            .expect("profile exists");
        assert!(
            profile
                .comment_styles
                .iter()
                .any(|style| style.flavor == CommentFlavor::Documentation),
            "language should define documentation comments"
        );
    }
}

/// Verify representative profiles expose the expected manual indentation family.
#[test]
fn test_representative_indentation_styles() {
    let rust = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Rust)
        .expect("find rust profile");
    let python = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Python)
        .expect("find python profile");
    let markdown = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Markdown)
        .expect("find markdown profile");

    assert_eq!(
        rust.indentation().map(|config| config.style),
        Some(IndentationStyle::CLike)
    );
    assert_eq!(
        python.indentation().map(|config| config.style),
        Some(IndentationStyle::PythonLike)
    );
    assert_eq!(
        markdown.indentation().map(|config| config.style),
        Some(IndentationStyle::PreviousLine)
    );
}

/// Verify Markdown highlighting stays conservative for punctuation-heavy prose.
#[test]
fn test_markdown_punctuation_heavy_prose_stays_plain() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Markdown)
        .expect("find markdown profile");
    let parsed = lex_profile_line(
        profile,
        "a_b_c * and [brackets] without target",
        LineLexMode::Plain,
    );
    assert!(
        parsed.spans.is_empty(),
        "ambiguous punctuation-heavy prose should stay plain"
    );
}

/// Verify Markdown inline code remains distinctly marked.
#[test]
fn test_markdown_inline_code_is_marked() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Markdown)
        .expect("find markdown profile");
    let parsed = lex_profile_line(profile, "before `code` after", LineLexMode::Plain);
    assert!(
        parsed
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::InlineCode))
    );
}

/// Verify AsciiDoc highlights common structural and inline markup constructs.
#[test]
fn test_asciidoc_highlights_common_markup_constructs() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::AsciiDoc)
        .expect("find asciidoc profile");

    let heading = lex_profile_line(profile, "= Title", LineLexMode::Plain);
    assert!(
        heading
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::Heading))
    );

    let list = lex_profile_line(profile, "** nested item", LineLexMode::Plain);
    assert!(
        list.spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::ListMarker) && span.covers(0))
    );

    let inline = lex_profile_line(
        profile,
        "See https://example.com[Docs] and +literal+ text.",
        LineLexMode::Plain,
    );
    assert!(
        inline
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::Link))
    );
    assert!(
        inline
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::InlineCode))
    );
}

/// Verify AsciiDoc delimited blocks carry state and style across lines.
#[test]
fn test_asciidoc_delimited_blocks_use_markup_and_comment_styles() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::AsciiDoc)
        .expect("find asciidoc profile");

    let comment_open = lex_profile_line(profile, "////", LineLexMode::Plain);
    assert_eq!(
        comment_open.exit_mode,
        LineLexMode::MarkupFence {
            marker: '/',
            count: 4,
            style: COMMENT_STYLE,
        }
    );
    assert!(
        comment_open
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Comment)
    );

    let comment_body = lex_profile_line(profile, "body", comment_open.exit_mode);
    assert!(
        comment_body
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Comment && span.start_col == 0)
    );

    let fence_open = lex_profile_line(profile, "----", LineLexMode::Plain);
    assert_eq!(
        fence_open.exit_mode,
        LineLexMode::MarkupFence {
            marker: '-',
            count: 4,
            style: SpanStyle::new(SyntaxClass::Markup, Some(SyntaxModifier::CodeFence)),
        }
    );
    assert!(
        fence_open
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::CodeFence))
    );
    let fence_body = lex_profile_line(profile, "code", fence_open.exit_mode);
    assert_eq!(fence_body.exit_mode, fence_open.exit_mode);
    let fence_close = lex_profile_line(profile, "----", fence_body.exit_mode);
    assert_eq!(fence_close.exit_mode, LineLexMode::Plain);
}

/// Verify AsciiDoc comment fences only trigger on dedicated delimiter lines.
#[test]
fn test_asciidoc_comment_fences_are_more_precise_than_inline_block_comments() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::AsciiDoc)
        .expect("find asciidoc profile");
    let parsed = lex_profile_line(profile, "before //// after", LineLexMode::Plain);
    assert_eq!(parsed.exit_mode, LineLexMode::Plain);
    assert!(
        !parsed
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Comment),
        "inline `////` should stay plain outside dedicated AsciiDoc fence lines"
    );
}

/// Verify SQL keywords are matched case-insensitively.
#[test]
fn test_sql_keywords_match_ignore_ascii_case() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Sql)
        .expect("find SQL profile");
    let parsed = lex_profile_line(profile, "select Value FrOm table_name", LineLexMode::Plain);
    for token in ["select", "FrOm"] {
        let start = "select Value FrOm table_name"
            .find(token)
            .expect("find SQL keyword");
        assert!(
            parsed
                .spans
                .iter()
                .any(|span| span.class == SyntaxClass::Keyword && span.covers(start)),
            "expected `{token}` to be highlighted as a SQL keyword"
        );
    }
}

/// Verify the lexer marks the requested tokens as keywords in a line.
#[track_caller]
fn assert_keyword_tokens(profile: &LanguageProfile, line: &str, tokens: &[&str]) {
    let parsed = lex_profile_line(profile, line, LineLexMode::Plain);
    for token in tokens {
        // Match the exact token occurrence so mixed-case regressions stay visible.
        let start = line.find(token).expect("find keyword token");
        assert!(
            parsed
                .spans
                .iter()
                .any(|span| span.class == SyntaxClass::Keyword && span.covers(start)),
            "expected `{token}` to be highlighted as a keyword"
        );
    }
}

/// Verify preprocessor directives highlight their directive words distinctly.
#[test]
fn test_preprocessor_directives_have_distinct_modifier() {
    let cases = [
        (LanguageId::C, "#  include <stdio.h>", "include"),
        (LanguageId::Cpp, "   #define VALUE 42", "define"),
        (LanguageId::CSharp, "#pragma warning disable 0168", "pragma"),
    ];
    for (language, line, token) in cases {
        let profile = builtin_profiles()
            .iter()
            .find(|profile| profile.id == language)
            .expect("find preprocessor-enabled profile");
        let parsed = lex_profile_line(profile, line, LineLexMode::Plain);
        let start = line.find(token).expect("find directive token");
        assert!(
            parsed.spans.iter().any(|span| {
                span.class == SyntaxClass::Keyword
                    && span.modifier == Some(SyntaxModifier::Preprocessor)
                    && span.covers(start)
            }),
            "expected `{token}` to be highlighted as a preprocessor directive"
        );
    }
}

/// Verify other case-insensitive languages highlight mixed-case keywords.
#[test]
fn test_additional_keywords_match_ignore_ascii_case() {
    // These profiles have language-defined ASCII case-insensitive keywords.
    let cases = [
        (
            LanguageId::Css,
            "div { color: AuTo; display: NoNe; }",
            &["AuTo", "NoNe"][..],
        ),
        (
            LanguageId::Scss,
            ".box { color: AuTo; display: NoNe; }",
            &["AuTo", "NoNe"][..],
        ),
        (
            LanguageId::Less,
            ".box { color: AuTo; display: NoNe; }",
            &["AuTo", "NoNe"][..],
        ),
        (
            LanguageId::Sass,
            "color: AuTo\n display: NoNe",
            &["AuTo", "NoNe"][..],
        ),
        (LanguageId::CMake, "ElseIf(VAR)", &["ElseIf"][..]),
        (LanguageId::Dockerfile, "from alpine:latest", &["from"][..]),
        (LanguageId::Masm, "eNdP main", &["eNdP"][..]),
        (LanguageId::Nasm, "SeCtIoN .text", &["SeCtIoN"][..]),
        (LanguageId::Yasm, "GlObAl _start", &["GlObAl"][..]),
    ];
    for (language, line, tokens) in cases {
        let profile = builtin_profiles()
            .iter()
            .find(|profile| profile.id == language)
            .expect("find case-insensitive profile");
        assert_keyword_tokens(profile, line, tokens);
    }
}
