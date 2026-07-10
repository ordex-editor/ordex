//! Shared syntax profile tests.

use crate::syntax::engine::{LineLexMode, lex_profile_line};
use crate::syntax::profile::*;
use crate::syntax::profiles::{
    builtin_profiles, corresponding_extensions_for, detect_language_details,
};
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
    assert_eq!(builtin_profiles().len(), 74);
}

/// Verify counterpart-extension rules are defined for C-family and interface-style languages.
#[test]
fn test_corresponding_extension_rules_cover_header_and_interface_profiles() {
    let c = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::C)
        .expect("find C profile");
    let cpp = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Cpp)
        .expect("find C++ profile");
    let ocaml = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Ocaml)
        .expect("find OCaml profile");
    let python = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Python)
        .expect("find Python profile");

    assert_eq!(corresponding_extensions_for(c, "c"), Some(&["h"][..]));
    assert_eq!(
        corresponding_extensions_for(c, "h"),
        Some(&["cc", "cpp", "cxx", "c"][..])
    );
    assert_eq!(
        corresponding_extensions_for(cpp, "cpp"),
        Some(&["h", "hpp", "hh", "hxx"][..])
    );
    assert_eq!(
        corresponding_extensions_for(ocaml, "ml"),
        Some(&["mli"][..])
    );
    assert_eq!(
        corresponding_extensions_for(python, "pyi"),
        Some(&["py"][..])
    );
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
        ("git-rebase-todo", LanguageId::GitRebase),
        ("COMMIT_EDITMSG", LanguageId::GitCommit),
        ("MERGE_MSG", LanguageId::GitCommit),
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

/// Verify block-comment continuation leaders are inferred from delimiters.
#[test]
fn test_block_comment_continue_markers_are_inferred() {
    let rust = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Rust)
        .expect("find rust profile");
    let html = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Html)
        .expect("find html profile");
    let ocaml = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::Ocaml)
        .expect("find ocaml profile");

    let rust_block = rust
        .comment_styles
        .iter()
        .find(|style| style.kind == CommentStyleKind::Block && style.open == "/*")
        .expect("find rust block comment");
    let html_block = html
        .comment_styles
        .iter()
        .find(|style| style.kind == CommentStyleKind::Block && style.open == "<!--")
        .expect("find html block comment");
    let ocaml_block = ocaml
        .comment_styles
        .iter()
        .find(|style| style.kind == CommentStyleKind::Block && style.open == "(*")
        .expect("find ocaml block comment");

    assert_eq!(rust_block.continue_with, Some("*"));
    assert_eq!(html_block.continue_with, Some("--"));
    assert_eq!(ocaml_block.continue_with, Some("*"));
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

/// Verify AsciiDoc fence markers only trigger when the line contains nothing
/// else besides the repeated marker characters, allowing headings and nested
/// list markers to be detected.
#[test]
fn test_asciidoc_fences_ignore_lines_with_trailing_content() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::AsciiDoc)
        .expect("find asciidoc profile");

    let heading = lex_profile_line(profile, "==== Title", LineLexMode::Plain);
    assert!(
        heading
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::Heading)),
        "`==== Title` should be a heading, not a fence"
    );
    assert_eq!(heading.exit_mode, LineLexMode::Plain);

    let nested_list = lex_profile_line(profile, "**** item", LineLexMode::Plain);
    assert!(
        nested_list
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::ListMarker) && span.covers(0)),
        "`**** item` should be a nested list marker, not a fence"
    );
    assert_eq!(nested_list.exit_mode, LineLexMode::Plain);

    let dash_list = lex_profile_line(profile, "---- item", LineLexMode::Plain);
    assert!(
        dash_list
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::ListMarker) && span.covers(0)),
        "`---- item` should be a nested list marker, not a fence"
    );
    assert_eq!(dash_list.exit_mode, LineLexMode::Plain);

    let pure_fence = lex_profile_line(profile, "----", LineLexMode::Plain);
    assert!(
        pure_fence
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::CodeFence)),
        "`----` should still open a delimited block"
    );
    assert!(matches!(
        pure_fence.exit_mode,
        LineLexMode::MarkupFence { .. }
    ));

    let fence_with_trailing_ws = lex_profile_line(profile, "---- ", LineLexMode::Plain);
    assert!(
        fence_with_trailing_ws
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::CodeFence)),
        "`---- ` (trailing whitespace) should still open a delimited block"
    );

    let equals_fence = lex_profile_line(profile, "====", LineLexMode::Plain);
    assert!(
        equals_fence
            .spans
            .iter()
            .any(|span| span.modifier == Some(SyntaxModifier::CodeFence)),
        "`====` should open a delimited block"
    );
    assert!(matches!(
        equals_fence.exit_mode,
        LineLexMode::MarkupFence { .. }
    ));
}

/// Verify AsciiDoc fence close lines ignore trailing content so code-content
/// lines inside a block do not accidentally close it.
#[test]
fn test_asciidoc_fence_close_ignores_lines_with_trailing_content() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::AsciiDoc)
        .expect("find asciidoc profile");

    let fence_open = lex_profile_line(profile, "----", LineLexMode::Plain);
    let entry_mode = fence_open.exit_mode;

    let close_with_text = lex_profile_line(profile, "---- item", entry_mode);
    assert!(
        matches!(close_with_text.exit_mode, LineLexMode::MarkupFence { .. }),
        "`---- item` inside a fence should stay in fence mode, not close it"
    );

    let proper_close = lex_profile_line(profile, "----", close_with_text.exit_mode);
    assert_eq!(
        proper_close.exit_mode,
        LineLexMode::Plain,
        "`----` alone should close the fence"
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

/// Verify git-rebase commands and commit hashes are highlighted on instruction lines.
#[test]
fn test_gitrebase_highlights_command_and_hash() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::GitRebase)
        .expect("find git rebase profile");
    let line = "pick deadbeef add parser tests";
    let parsed = lex_profile_line(profile, line, LineLexMode::Plain);
    let command_col = line.find("pick").expect("find rebase command");
    let hash_col = line.find("deadbeef").expect("find rebase hash");

    assert!(
        parsed
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Keyword && span.covers(command_col))
    );
    assert!(
        parsed
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Number && span.covers(hash_col))
    );
}

/// Verify git-rebase comment lines do not highlight command tokens.
#[test]
fn test_gitrebase_comment_lines_do_not_highlight_commands() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::GitRebase)
        .expect("find git rebase profile");
    let line = "# pick deadbeef is commented";
    let parsed = lex_profile_line(profile, line, LineLexMode::Plain);

    assert!(
        !parsed
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Keyword && span.covers(2))
    );
}

/// Verify git-rebase short non-hex tokens are not highlighted as commit hashes.
#[test]
fn test_gitrebase_rejects_non_hash_second_token() {
    let profile = builtin_profiles()
        .iter()
        .find(|profile| profile.id == LanguageId::GitRebase)
        .expect("find git rebase profile");
    let line = "pick topic_branch add parser tests";
    let parsed = lex_profile_line(profile, line, LineLexMode::Plain);
    let token_col = line.find("topic_branch").expect("find non-hash token");

    assert!(
        !parsed
            .spans
            .iter()
            .any(|span| span.class == SyntaxClass::Number && span.covers(token_col))
    );
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
