# Module Interface Contract: Language Profile Registry

**Modules**: `src/syntax/profile.rs`, `src/syntax/registry.rs`  
**Purpose**: Define built-in language metadata, detection rules, and reusable syntax categories  
**Date**: 2026-03-11

## Public API

```rust
pub(crate) enum LanguageId {
    Rust,
    Toml,
    Markdown,
    D,
}

pub(crate) enum SyntaxClass {
    Plain,
    Comment,
    String,
    Number,
    Keyword,
    Punctuation,
    Markup,
}

pub(crate) enum SyntaxModifier {
    Heading,
    Emphasis,
    Strong,
    InlineCode,
    CodeFence,
    ListMarker,
    Quote,
    Link,
}

pub(crate) struct LanguageDetection {
    pub(crate) exact_filenames: &'static [&'static str],
    pub(crate) extensions: &'static [&'static str],
}

pub(crate) struct CommentStyle {
    pub(crate) id: &'static str,
    pub(crate) kind: CommentStyleKind,
    pub(crate) open: &'static str,
    pub(crate) close: Option<&'static str>,
    pub(crate) nests: bool,
    pub(crate) preferred_default: bool,
}

pub(crate) struct LanguageProfile {
    pub(crate) id: LanguageId,
    pub(crate) detection: LanguageDetection,
    pub(crate) comment_styles: &'static [CommentStyle],
}

impl LanguageProfile {
    pub(crate) fn matches_path(&self, path: &Path) -> bool;
}

pub(crate) fn builtin_profiles() -> &'static [LanguageProfile];
pub(crate) fn detect_language(path: Option<&Path>) -> Option<&'static LanguageProfile>;
```

## Responsibilities

- Own the built-in Rust, config/TOML, Markdown, and D profile definitions
- Implement filename/extension-based detection only
- Expose reusable comment metadata for highlighting and future comment commands
- Keep syntax categories semantic rather than hard-coding terminal colors into profiles

## Required Behaviors

- Exact filename detection must take precedence over extension detection
- D must expose all supported comment styles, including nested block comments
- A language with multiple comment styles must mark exactly one preferred default
- Markdown may advertise conservative markup modifiers without promising full CommonMark support

## Testing Requirements

- filename/extension detection precedence
- unsupported file fallback to `None`
- D preferred comment-style uniqueness
- Markdown profile stays conservative and does not claim unsupported constructs
