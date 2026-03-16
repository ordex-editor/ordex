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
    /// JavaScript source files.
    JavaScript,
    /// TypeScript source files.
    TypeScript,
    /// Python source files.
    Python,
    /// Java source files.
    Java,
    /// C# source files.
    CSharp,
    /// C++ source files.
    Cpp,
    /// Go source files.
    Go,
    /// C source files.
    C,
    /// PHP source files.
    Php,
    /// AsciiDoc documents.
    AsciiDoc,
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
    /// A doubled closer escapes a literal closer inside the string.
    RepeatQuote,
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
    /// A fixed opener and closer that allow explicit prefixes.
    PrefixedDelimited {
        /// Exact prefixes allowed before `open`.
        prefixes: &'static [&'static str],
        /// Opening delimiter that follows the prefix.
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
        /// Exact prefixes allowed before the marker run.
        prefixes: &'static [&'static str],
        /// Repeated marker captured between prefix and quote.
        marker: char,
        /// Quote character used on both ends.
        quote: char,
    },
    /// One C++-style raw string with a captured custom delimiter.
    CppRaw {
        /// Exact prefixes allowed before `R"`.
        prefixes: &'static [&'static str],
        /// Maximum delimiter length accepted from the opener.
        max_delimiter_len: usize,
    },
}

/// Shared string-style metadata for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StringStyle {
    /// Delimiter behavior for this style.
    pub(crate) kind: StringStyleKind,
}

/// Build one delimited string style with explicit escape and multiline settings.
pub(crate) const fn custom_delimited_string(
    open: &'static str,
    close: &'static str,
    escape: EscapeMode,
    multiline: bool,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::Delimited {
            open,
            close,
            escape,
            multiline,
        },
    }
}

/// Build one escaped delimited string style.
pub(crate) const fn escaped_delimited_string(
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    custom_delimited_string(open, close, EscapeMode::Backslash, false)
}

/// Build one plain delimited string style.
pub(crate) const fn plain_delimited_string(open: &'static str, close: &'static str) -> StringStyle {
    custom_delimited_string(open, close, EscapeMode::None, false)
}

/// Build one multiline escaped delimited string style.
pub(crate) const fn multiline_escaped_delimited_string(
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    custom_delimited_string(open, close, EscapeMode::Backslash, true)
}

/// Build one multiline plain delimited string style.
pub(crate) const fn multiline_plain_delimited_string(
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    custom_delimited_string(open, close, EscapeMode::None, true)
}

/// Build one prefixed delimited string style with explicit escape settings.
pub(crate) const fn custom_prefixed_delimited_string(
    prefixes: &'static [&'static str],
    open: &'static str,
    close: &'static str,
    escape: EscapeMode,
    multiline: bool,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::PrefixedDelimited {
            prefixes,
            open,
            close,
            escape,
            multiline,
        },
    }
}

/// Build one escaped prefixed delimited string style.
pub(crate) const fn prefixed_escaped_delimited_string(
    prefixes: &'static [&'static str],
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    custom_prefixed_delimited_string(prefixes, open, close, EscapeMode::Backslash, false)
}

/// Build one multiline escaped prefixed delimited string style.
pub(crate) const fn prefixed_multiline_escaped_delimited_string(
    prefixes: &'static [&'static str],
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    custom_prefixed_delimited_string(prefixes, open, close, EscapeMode::Backslash, true)
}

/// Build one multiline repeated-quote prefixed string style.
pub(crate) const fn prefixed_multiline_repeated_quote_string(
    prefixes: &'static [&'static str],
    open: &'static str,
    close: &'static str,
) -> StringStyle {
    custom_prefixed_delimited_string(prefixes, open, close, EscapeMode::RepeatQuote, true)
}

/// Build one double-quoted string style.
pub(crate) const fn double_quoted_string() -> StringStyle {
    escaped_delimited_string("\"", "\"")
}

/// Build one single-quoted string style.
pub(crate) const fn single_quoted_string() -> StringStyle {
    escaped_delimited_string("'", "'")
}

/// Build one triple-double-quoted string style.
pub(crate) const fn triple_double_quoted_string() -> StringStyle {
    multiline_escaped_delimited_string("\"\"\"", "\"\"\"")
}

/// Build one triple-single-quoted string style.
pub(crate) const fn triple_single_quoted_string() -> StringStyle {
    multiline_escaped_delimited_string("'''", "'''")
}

/// Build one raw string with captured repeated markers.
pub(crate) const fn raw_hash_string(
    prefixes: &'static [&'static str],
    marker: char,
    quote: char,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::HashDelimited {
            prefixes,
            marker,
            quote,
        },
    }
}

/// Build one C++-style raw string with optional prefixes.
pub(crate) const fn cpp_raw_string(
    prefixes: &'static [&'static str],
    max_delimiter_len: usize,
) -> StringStyle {
    StringStyle {
        kind: StringStyleKind::CppRaw {
            prefixes,
            max_delimiter_len,
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

/// Digit separators supported by a number grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DigitSeparator {
    /// No separators are accepted.
    None,
    /// Underscore separators are accepted between digits.
    Underscore,
    /// Apostrophe separators are accepted between digits.
    Apostrophe,
}

/// One suffix group that may appear at most once in a numeric literal suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NumberSuffixGroup {
    /// Exact suffix spellings accepted for this group.
    pub(crate) spellings: &'static [&'static str],
}

/// Build one reusable numeric suffix group.
pub(crate) const fn suffix_group(spellings: &'static [&'static str]) -> NumberSuffixGroup {
    NumberSuffixGroup { spellings }
}

/// Configurable suffix rules attached to one numeric grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NumberSuffixPattern {
    /// Exact suffixes accepted after integer cores.
    pub(crate) integer_exact: &'static [&'static str],
    /// Exact suffixes accepted after float cores.
    pub(crate) float_exact: &'static [&'static str],
    /// Optional suffix groups accepted after integer cores.
    pub(crate) integer_groups: &'static [NumberSuffixGroup],
    /// Optional suffix groups accepted after float cores.
    pub(crate) float_groups: &'static [NumberSuffixGroup],
}

impl NumberSuffixPattern {
    /// Build one suffix pattern with no accepted suffixes.
    pub(crate) const fn none() -> Self {
        Self {
            integer_exact: &[],
            float_exact: &[],
            integer_groups: &[],
            float_groups: &[],
        }
    }

    /// Build one suffix pattern starting from no accepted suffixes.
    pub(crate) const fn new() -> Self {
        Self::none()
    }

    /// Set the exact suffixes accepted after integer cores.
    pub(crate) const fn with_integer_exact(mut self, suffixes: &'static [&'static str]) -> Self {
        self.integer_exact = suffixes;
        self
    }

    /// Set the exact suffixes accepted after float cores.
    pub(crate) const fn with_float_exact(mut self, suffixes: &'static [&'static str]) -> Self {
        self.float_exact = suffixes;
        self
    }

    /// Set the optional suffix groups accepted after integer cores.
    pub(crate) const fn with_integer_groups(
        mut self,
        groups: &'static [NumberSuffixGroup],
    ) -> Self {
        self.integer_groups = groups;
        self
    }
}

/// Number scanning knobs for one code-like language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NumberPattern {
    /// Whether `+` or `-` may start a number token.
    pub(crate) allow_leading_sign: bool,
    /// Whether `.5`-style literals may start with a decimal point.
    pub(crate) allow_leading_dot: bool,
    /// Whether `1.` remains one valid number token.
    pub(crate) allow_trailing_dot: bool,
    /// Whether decimal fractions are supported.
    pub(crate) allow_fraction: bool,
    /// Which separator may appear between digits.
    pub(crate) digit_separator: DigitSeparator,
    /// Whether `0b...` binary literals are supported.
    pub(crate) allow_binary: bool,
    /// Whether `0o...` octal literals are supported.
    pub(crate) allow_octal_prefix: bool,
    /// Whether `0x...` hexadecimal literals are supported.
    pub(crate) allow_hex: bool,
    /// Whether legacy leading-zero octal literals are supported.
    pub(crate) allow_legacy_octal: bool,
    /// Whether decimal exponents (`e` / `E`) are supported.
    pub(crate) allow_decimal_exponent: bool,
    /// Whether hexadecimal float exponents (`p` / `P`) are supported.
    pub(crate) allow_hex_exponent: bool,
    /// Which numeric suffixes are valid after the numeric core.
    pub(crate) suffix_pattern: NumberSuffixPattern,
}

impl NumberPattern {
    /// Build one conservative signed pattern used by TOML-like profiles.
    pub(crate) const fn signed() -> Self {
        Self {
            allow_leading_sign: true,
            ..Self::unsigned()
        }
    }

    /// Build one conservative unsigned pattern used by markup-like profiles.
    pub(crate) const fn unsigned() -> Self {
        Self {
            allow_leading_sign: false,
            allow_leading_dot: false,
            allow_trailing_dot: false,
            allow_fraction: true,
            digit_separator: DigitSeparator::Underscore,
            allow_binary: false,
            allow_octal_prefix: false,
            allow_hex: false,
            allow_legacy_octal: false,
            allow_decimal_exponent: false,
            allow_hex_exponent: false,
            suffix_pattern: NumberSuffixPattern::none(),
        }
    }

    /// Build the common numeric grammar shared by many programming languages.
    pub(crate) const fn common_code() -> Self {
        Self::unsigned()
            .supports_leading_dot(true)
            .supports_trailing_dot(true)
            .supports_binary(true)
            .supports_octal_prefix(true)
            .supports_hex(true)
            .supports_decimal_exponent(true)
    }

    /// Set whether `.5`-style literals may start with a dot.
    pub(crate) const fn supports_leading_dot(mut self, supported: bool) -> Self {
        self.allow_leading_dot = supported;
        self
    }

    /// Set whether `1.` remains a valid single token.
    pub(crate) const fn supports_trailing_dot(mut self, supported: bool) -> Self {
        self.allow_trailing_dot = supported;
        self
    }

    /// Set the digit separator accepted between digit runs.
    pub(crate) const fn with_digit_separator(mut self, separator: DigitSeparator) -> Self {
        self.digit_separator = separator;
        self
    }

    /// Set whether `0b...` literals are supported.
    pub(crate) const fn supports_binary(mut self, supported: bool) -> Self {
        self.allow_binary = supported;
        self
    }

    /// Set whether `0o...` literals are supported.
    pub(crate) const fn supports_octal_prefix(mut self, supported: bool) -> Self {
        self.allow_octal_prefix = supported;
        self
    }

    /// Set whether `0x...` literals are supported.
    pub(crate) const fn supports_hex(mut self, supported: bool) -> Self {
        self.allow_hex = supported;
        self
    }

    /// Set whether legacy leading-zero octal literals are supported.
    pub(crate) const fn supports_legacy_octal(mut self, supported: bool) -> Self {
        self.allow_legacy_octal = supported;
        self
    }

    /// Set whether decimal exponents (`e` / `E`) are supported.
    pub(crate) const fn supports_decimal_exponent(mut self, supported: bool) -> Self {
        self.allow_decimal_exponent = supported;
        self
    }

    /// Set whether hexadecimal float exponents (`p` / `P`) are supported.
    pub(crate) const fn supports_hex_exponent(mut self, supported: bool) -> Self {
        self.allow_hex_exponent = supported;
        self
    }

    /// Set the numeric suffix pattern recognized after the core literal.
    pub(crate) const fn with_suffix_pattern(mut self, suffix_pattern: NumberSuffixPattern) -> Self {
        self.suffix_pattern = suffix_pattern;
        self
    }
}

/// Common signed decimal scanning used by TOML-like profiles.
pub(crate) const SIGNED_NUMBER: NumberPattern = NumberPattern::signed()
    .supports_binary(true)
    .supports_octal_prefix(true)
    .supports_hex(true)
    .supports_decimal_exponent(true);

/// Common unsigned decimal scanning used by markup-like profiles.
pub(crate) const UNSIGNED_NUMBER: NumberPattern = NumberPattern::unsigned();

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
