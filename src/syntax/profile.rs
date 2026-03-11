//! Shared syntax profile metadata.
//!
//! The concrete lexing rules live in the per-language profile modules, but the
//! semantic categories and profile registry shape are shared by the whole
//! syntax-highlighting subsystem.

use crate::syntax::engine::{LineLexMode, LineParseResult};
use std::path::Path;

/// Built-in language identifiers supported by the phase-1 syntax engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanguageId {
    /// Rust source files.
    Rust,
    /// TOML and config-like files.
    Toml,
    /// Conservative-core Markdown documents.
    Markdown,
    /// D source files.
    D,
}

/// Semantic syntax categories shared across all profiles.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxClass {
    /// Unstyled plain content.
    Plain,
    /// Comments and comment bodies.
    Comment,
    /// Strings and character-like literals.
    String,
    /// Numeric literals.
    Number,
    /// Keywords and keyword-like bare keys.
    Keyword,
    /// Delimiters and operator punctuation.
    Punctuation,
    /// Markdown-style markup constructs.
    Markup,
}

/// Semantic refinements layered on top of a syntax class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxModifier {
    /// Documentation-comment styling.
    DocComment,
    /// Markdown heading styling.
    Heading,
    /// Markdown emphasis styling.
    Emphasis,
    /// Markdown strong emphasis styling.
    Strong,
    /// Markdown inline-code styling.
    InlineCode,
    /// Markdown fenced-code styling.
    CodeFence,
    /// Markdown list-marker styling.
    ListMarker,
    /// Markdown block-quote styling.
    Quote,
    /// Markdown inline-link styling.
    Link,
}

/// High-level comment flavor for highlighting and future comment commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommentFlavor {
    /// Ordinary comments intended for general prose.
    Ordinary,
    /// Documentation comments that should be style-distinct.
    Documentation,
}

/// Structural comment kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommentStyleKind {
    /// Line comment terminated by the current line ending.
    Line,
    /// Block comment that may cross lines.
    Block,
}

/// Filename- and extension-based language detection data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LanguageDetection {
    /// Exact filenames that should match this profile before extension checks.
    pub(crate) exact_filenames: &'static [&'static str],
    /// File extensions recognized by this profile.
    pub(crate) extensions: &'static [&'static str],
}

/// Shared comment-style metadata for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommentStyle {
    /// Stable comment-style identifier.
    pub(crate) id: &'static str,
    /// Ordinary vs documentation flavor.
    pub(crate) flavor: CommentFlavor,
    /// Line vs block behavior.
    pub(crate) kind: CommentStyleKind,
    /// Opening delimiter.
    pub(crate) open: &'static str,
    /// Closing delimiter when the style is block-based.
    pub(crate) close: Option<&'static str>,
    /// Whether nested occurrences increase block depth.
    pub(crate) nests: bool,
    /// Whether this ordinary style is the preferred default.
    pub(crate) preferred_default: bool,
}

/// Reserved nested-language metadata for future phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NestedLanguageHook {
    /// Host syntax modifier that would carry embedded content.
    pub(crate) host_modifier: SyntaxModifier,
    /// Human-readable hint for a future embedded target.
    pub(crate) target_hint: &'static str,
}

/// One built-in language profile and its lexing callback.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct LanguageProfile {
    /// Stable language identifier.
    pub(crate) id: LanguageId,
    /// User-facing language name.
    pub(crate) display_name: &'static str,
    /// Detection rules for this profile.
    pub(crate) detection: LanguageDetection,
    /// Shared comment-style metadata for this language.
    pub(crate) comment_styles: &'static [CommentStyle],
    /// Reserved nested-language hooks.
    pub(crate) nested_hooks: &'static [NestedLanguageHook],
    /// Per-line lexer implementation for this profile.
    pub(crate) lex_line: fn(&str, LineLexMode) -> LineParseResult,
}

impl LanguageProfile {
    /// Return whether this profile matches the supplied path.
    #[allow(dead_code)]
    pub(crate) fn matches_path(&self, path: &Path) -> bool {
        if let Some(file_name) = path.file_name().and_then(|name| name.to_str())
            && self.detection.exact_filenames.contains(&file_name)
        {
            return true;
        }
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| self.detection.extensions.contains(&ext))
    }

    /// Return the preferred default ordinary comment style, if any.
    #[allow(dead_code)]
    pub(crate) fn preferred_comment_style(&self) -> Option<&'static CommentStyle> {
        self.comment_styles
            .iter()
            .find(|style| style.flavor == CommentFlavor::Ordinary && style.preferred_default)
    }
}
