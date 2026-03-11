//! Shared syntax profile metadata.
//!
//! The generic lexer consumes these profile definitions so language modules can
//! stay small and focused on data.

use std::path::Path;

/// Built-in language identifiers supported by the syntax engine.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxClass {
    /// Comments and comment bodies.
    Comment,
    /// Strings and string-like literals.
    String,
    /// Numeric literals.
    Number,
    /// Keywords and keyword-like identifiers.
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
    /// Markdown strong-emphasis styling.
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

/// One semantic style that can be turned into a highlight span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpanStyle {
    /// Semantic class for the span.
    pub(crate) class: SyntaxClass,
    /// Optional modifier layered on top of the class.
    pub(crate) modifier: Option<SyntaxModifier>,
}

impl SpanStyle {
    /// Build one semantic span style.
    pub(crate) const fn new(class: SyntaxClass, modifier: Option<SyntaxModifier>) -> Self {
        Self { class, modifier }
    }
}

/// Shared ordinary-comment styling.
pub(crate) const COMMENT_STYLE: SpanStyle = SpanStyle::new(SyntaxClass::Comment, None);
/// Shared documentation-comment styling.
pub(crate) const DOC_COMMENT_STYLE: SpanStyle =
    SpanStyle::new(SyntaxClass::Comment, Some(SyntaxModifier::DocComment));
/// Shared string styling.
pub(crate) const STRING_STYLE: SpanStyle = SpanStyle::new(SyntaxClass::String, None);
/// Shared number styling.
pub(crate) const NUMBER_STYLE: SpanStyle = SpanStyle::new(SyntaxClass::Number, None);
/// Shared keyword styling.
pub(crate) const KEYWORD_STYLE: SpanStyle = SpanStyle::new(SyntaxClass::Keyword, None);
/// Shared punctuation styling.
pub(crate) const PUNCTUATION_STYLE: SpanStyle = SpanStyle::new(SyntaxClass::Punctuation, None);

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

/// Shared comment-style metadata for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommentStyle {
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

impl CommentStyle {
    /// Return the semantic style used when this comment is highlighted.
    pub(crate) const fn span_style(self) -> SpanStyle {
        match self.flavor {
            CommentFlavor::Ordinary => COMMENT_STYLE,
            CommentFlavor::Documentation => DOC_COMMENT_STYLE,
        }
    }
}

/// Build one ordinary line-comment style.
pub(crate) const fn line_comment(open: &'static str) -> CommentStyle {
    CommentStyle {
        flavor: CommentFlavor::Ordinary,
        kind: CommentStyleKind::Line,
        open,
        close: None,
        nests: false,
        preferred_default: false,
    }
}

/// Build one preferred ordinary line-comment style.
pub(crate) const fn preferred_line_comment(open: &'static str) -> CommentStyle {
    CommentStyle {
        preferred_default: true,
        ..line_comment(open)
    }
}

/// Build one documentation line-comment style.
pub(crate) const fn doc_line_comment(open: &'static str) -> CommentStyle {
    CommentStyle {
        flavor: CommentFlavor::Documentation,
        preferred_default: false,
        ..line_comment(open)
    }
}

/// Build one ordinary block-comment style.
pub(crate) const fn block_comment(open: &'static str, close: &'static str) -> CommentStyle {
    CommentStyle {
        flavor: CommentFlavor::Ordinary,
        kind: CommentStyleKind::Block,
        open,
        close: Some(close),
        nests: false,
        preferred_default: false,
    }
}

/// Build one nested ordinary block-comment style.
pub(crate) const fn nested_block_comment(open: &'static str, close: &'static str) -> CommentStyle {
    CommentStyle {
        nests: true,
        ..block_comment(open, close)
    }
}

/// Build one documentation block-comment style.
pub(crate) const fn doc_block_comment(open: &'static str, close: &'static str) -> CommentStyle {
    CommentStyle {
        flavor: CommentFlavor::Documentation,
        ..block_comment(open, close)
    }
}

/// Build one nested documentation block-comment style.
pub(crate) const fn nested_doc_block_comment(
    open: &'static str,
    close: &'static str,
) -> CommentStyle {
    CommentStyle {
        nests: true,
        ..doc_block_comment(open, close)
    }
}

/// How a delimited string handles escape sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapeMode {
    /// The delimiter has no escape mechanism.
    None,
    /// Backslash escapes suppress the next character.
    Backslash,
}

/// String-style configuration supported by the generic lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringStyleKind {
    /// A fixed opener and closer, optionally spanning multiple lines.
    Delimited {
        /// Opening delimiter.
        open: &'static str,
        /// Closing delimiter.
        close: &'static str,
        /// Escape handling inside the string.
        escape: EscapeMode,
        /// Whether an unclosed delimiter carries to the next line.
        multiline: bool,
    },
    /// A raw string that captures a repeated marker count from its opener.
    HashDelimited {
        /// Prefix that introduces the raw string.
        prefix: char,
        /// Repeated marker captured between `prefix` and `quote`.
        marker: char,
        /// Quote character used on both ends.
        quote: char,
    },
}

/// Shared string-style metadata for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StringStyle {
    /// Delimiter behavior for this style.
    pub(crate) kind: StringStyleKind,
}

/// Build one escaped delimited string style.
pub(crate) const fn escaped_delimited_string(
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::Delimited {
            open,
            close,
            escape: EscapeMode::Backslash,
            multiline: false,
        },
    }
}

/// Build one plain delimited string style.
pub(crate) const fn plain_delimited_string(open: &'static str, close: &'static str) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::Delimited {
            open,
            close,
            escape: EscapeMode::None,
            multiline: false,
        },
    }
}

/// Build one multiline escaped delimited string style.
pub(crate) const fn multiline_escaped_delimited_string(
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::Delimited {
            open,
            close,
            escape: EscapeMode::Backslash,
            multiline: true,
        },
    }
}

/// Build one multiline plain delimited string style.
pub(crate) const fn multiline_plain_delimited_string(
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::Delimited {
            open,
            close,
            escape: EscapeMode::None,
            multiline: true,
        },
    }
}

/// Build one double-quoted string style.
pub(crate) const fn double_quoted_string() -> StringStyle {
    escaped_delimited_string("\"", "\"")
}

/// Build one single-quoted string style.
pub(crate) const fn single_quoted_string() -> StringStyle {
    plain_delimited_string("'", "'")
}

/// Build one triple-double-quoted string style.
pub(crate) const fn triple_double_quoted_string() -> StringStyle {
    multiline_escaped_delimited_string("\"\"\"", "\"\"\"")
}

/// Build one triple-single-quoted string style.
pub(crate) const fn triple_single_quoted_string() -> StringStyle {
    multiline_plain_delimited_string("'''", "'''")
}

/// Build one raw string with captured repeated markers.
pub(crate) const fn raw_hash_string(prefix: char, marker: char, quote: char) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::HashDelimited {
            prefix,
            marker,
            quote,
        },
    }
}

/// Common identifier character sets supported by the generic lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentifierCharSet {
    /// `[a-zA-Z_]`
    LetterOrUnderscore,
    /// `[a-zA-Z0-9_]`
    AlnumOrUnderscore,
    /// `[a-zA-Z0-9_-]`
    AlnumUnderscoreOrDash,
}

/// Identifier parsing for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdentifierPattern {
    /// Character set allowed for the first character.
    pub(crate) start: IdentifierCharSet,
    /// Character set allowed for following characters.
    pub(crate) continue_chars: IdentifierCharSet,
}

/// Build the common `[a-zA-Z_][a-zA-Z0-9_]*` identifier pattern.
pub(crate) const fn ascii_identifier() -> IdentifierPattern {
    IdentifierPattern {
        start: IdentifierCharSet::LetterOrUnderscore,
        continue_chars: IdentifierCharSet::AlnumOrUnderscore,
    }
}

/// Build a TOML-style bare-key identifier pattern that allows dashes after the first character.
pub(crate) const fn ascii_identifier_with_dashes() -> IdentifierPattern {
    IdentifierPattern {
        start: IdentifierCharSet::LetterOrUnderscore,
        continue_chars: IdentifierCharSet::AlnumUnderscoreOrDash,
    }
}

/// Number scanning knobs for common code-like languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NumberPattern {
    /// Whether `+` or `-` may start a number token.
    pub(crate) allow_leading_sign: bool,
}

/// Common signed ASCII number scanning.
pub(crate) const SIGNED_NUMBER: NumberPattern = NumberPattern {
    allow_leading_sign: true,
};
/// Common unsigned ASCII number scanning.
pub(crate) const UNSIGNED_NUMBER: NumberPattern = NumberPattern {
    allow_leading_sign: false,
};

/// Extra context used when classifying identifier tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentifierContext {
    /// The rule applies anywhere the token appears.
    Anywhere,
    /// The token must be followed by the given character.
    BeforeChar {
        /// Character required after the token.
        ch: char,
        /// Whether ASCII whitespace may appear before `ch`.
        allow_whitespace: bool,
    },
}

/// How one identifier rule decides whether a token matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentifierMatch {
    /// Match any identifier token.
    Any,
    /// Match one of the supplied words exactly.
    ExactWords(&'static [&'static str]),
}

/// Identifier classification used by the generic lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdentifierRule {
    /// Which identifiers the rule should match.
    pub(crate) match_kind: IdentifierMatch,
    /// Additional context the token must satisfy.
    pub(crate) context: IdentifierContext,
    /// Style emitted when the rule matches.
    pub(crate) style: SpanStyle,
}

/// Build one keyword-style identifier rule from a word list.
pub(crate) const fn keyword_rule(words: &'static [&'static str]) -> IdentifierRule {
    IdentifierRule {
        match_kind: IdentifierMatch::ExactWords(words),
        context: IdentifierContext::Anywhere,
        style: KEYWORD_STYLE,
    }
}

/// Build one exact-word identifier rule with a custom style.
pub(crate) const fn exact_words_rule(
    words: &'static [&'static str],
    style: SpanStyle,
) -> IdentifierRule {
    IdentifierRule {
        match_kind: IdentifierMatch::ExactWords(words),
        context: IdentifierContext::Anywhere,
        style,
    }
}

/// Build one context-sensitive identifier rule that fires before `ch`.
pub(crate) const fn any_identifier_before(ch: char, style: SpanStyle) -> IdentifierRule {
    IdentifierRule {
        match_kind: IdentifierMatch::Any,
        context: IdentifierContext::BeforeChar {
            ch,
            allow_whitespace: true,
        },
        style,
    }
}

/// Markup rules consumed by the generic markup lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MarkupRules {
    /// Fence markers that may open code fences.
    pub(crate) fence_markers: &'static [char],
    /// Single-character unordered list markers.
    pub(crate) unordered_list_markers: &'static [char],
}

/// Shared markup behavior used by the built-in Markdown profile.
pub(crate) const COMMON_MARKUP_RULES: MarkupRules = MarkupRules {
    fence_markers: &['`', '~'],
    unordered_list_markers: &['-', '*', '+'],
};

/// Reserved nested-language metadata for future expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NestedLanguageHook {
    /// Host syntax modifier that would carry embedded content.
    pub(crate) host_modifier: SyntaxModifier,
    /// Human-readable hint for a future embedded target.
    pub(crate) target_hint: &'static str,
}

/// One built-in language profile consumed by the generic lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LanguageProfile {
    /// Stable language identifier.
    pub(crate) id: LanguageId,
    /// User-facing language name.
    pub(crate) display_name: &'static str,
    /// Exact filenames that should match before extension checks.
    pub(crate) exact_filenames: &'static [&'static str],
    /// File extensions recognized by this profile.
    pub(crate) extensions: &'static [&'static str],
    /// Shared comment-style metadata for this language.
    pub(crate) comment_styles: &'static [CommentStyle],
    /// Shared string-style metadata for this language.
    pub(crate) string_styles: &'static [StringStyle],
    /// Identifier parsing, when the language has identifiers.
    pub(crate) identifier: Option<IdentifierPattern>,
    /// Identifier classification rules.
    pub(crate) identifier_rules: &'static [IdentifierRule],
    /// One-character punctuation set highlighted by the generic lexer.
    pub(crate) punctuation_chars: &'static str,
    /// Number scanning for this language.
    pub(crate) number_pattern: NumberPattern,
    /// Markup-specific rules, when this is a markup-like profile.
    pub(crate) markup_rules: Option<MarkupRules>,
    /// Reserved nested-language hooks.
    pub(crate) nested_hooks: &'static [NestedLanguageHook],
}

impl LanguageProfile {
    /// Return whether this profile matches the supplied path.
    pub(crate) fn matches_path(&self, path: &Path) -> bool {
        // Exact filename matches win before extensions so special files like
        // `Cargo.toml` can override any broader extension behavior.
        if let Some(file_name) = path.file_name().and_then(|name| name.to_str())
            && self.exact_filenames.contains(&file_name)
        {
            return true;
        }
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| self.extensions.contains(&ext))
    }
}
