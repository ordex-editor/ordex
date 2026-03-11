//! Shared syntax profile tests.

use crate::syntax::engine::LineLexMode;
use crate::syntax::profile::{CommentFlavor, LanguageId, SyntaxModifier};
use crate::syntax::profiles::{builtin_profiles, detect_language};
use std::path::Path;

/// Verify exact filename detection wins over extension fallback.
#[test]
fn test_detect_language_prefers_exact_filename() {
    let profile = detect_language(Some(Path::new("Cargo.toml"))).expect("detect Cargo.toml");
    assert_eq!(profile.id, LanguageId::Toml);
}

/// Verify unsupported files fall back to no profile.
#[test]
fn test_detect_language_returns_none_for_unsupported_paths() {
    assert!(detect_language(Some(Path::new("notes.txt"))).is_none());
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
        .count();
    assert_eq!(preferred, 1);
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
    let parsed = (profile.lex_line)("a_b_c * and [brackets] without target", LineLexMode::Plain);
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
    let parsed = (profile.lex_line)("before `code` after", LineLexMode::Plain);
    assert!(
        parsed
            .spans
            .iter()
            .any(|span| { span.modifier == Some(SyntaxModifier::InlineCode) })
    );
}
