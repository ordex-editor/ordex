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

/// Verify the expanded language set is detected by representative extensions.
#[test]
fn test_detect_language_matches_new_language_extensions() {
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
