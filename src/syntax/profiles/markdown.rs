//! Markdown syntax profile.

use crate::syntax::profile::*;

const MARKUP_RULES: MarkupRules = MarkupRules {
    thematic_break: Some(markup_thematic_break_rule(&['-', '*', '_'], 3)),
    heading_rules: &[markup_heading_rule('#', 1, 6)],
    block_quote_prefixes: &["> ", ">"],
    list_rules: &[
        repeated_marker_list_rule('-', 1),
        repeated_marker_list_rule('*', 1),
        repeated_marker_list_rule('+', 1),
        decimal_dot_list_rule(),
    ],
    fence_markers: &['`', '~'],
    comment_fence_markers: &[],
    min_fence_len: 3,
    inline_delimited_rules: &[
        InlineDelimitedMarkupRule {
            delimiter: "`",
            min_span_len: 3,
            boundary: InlineDelimiterBoundary::None,
            modifier: SyntaxModifier::InlineCode,
        },
        InlineDelimitedMarkupRule {
            delimiter: "**",
            min_span_len: 4,
            boundary: InlineDelimiterBoundary::EmphasisLike,
            modifier: SyntaxModifier::Strong,
        },
        InlineDelimitedMarkupRule {
            delimiter: "__",
            min_span_len: 4,
            boundary: InlineDelimiterBoundary::EmphasisLike,
            modifier: SyntaxModifier::Strong,
        },
        InlineDelimitedMarkupRule {
            delimiter: "*",
            min_span_len: 3,
            boundary: InlineDelimiterBoundary::EmphasisLike,
            modifier: SyntaxModifier::Emphasis,
        },
        InlineDelimitedMarkupRule {
            delimiter: "_",
            min_span_len: 3,
            boundary: InlineDelimiterBoundary::EmphasisLike,
            modifier: SyntaxModifier::Emphasis,
        },
    ],
    inline_bracket_links: &[
        inline_bracket_link_rule("![", ']', '(', ')'),
        inline_bracket_link_rule("[", ']', '(', ')'),
    ],
    inline_prefixed_bracket_spans: &[],
    inline_balanced_pair_spans: &[],
};

/// Static Markdown language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Markdown,
    display_name: "Markdown",
    exact_filenames: &[],
    extensions: &["md", "markdown"],
    comment_styles: &[],
    string_styles: &[],
    identifier: ascii_identifier(),
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: UNSIGNED_NUMBER,
    markup_rules: Some(MARKUP_RULES),
    indentation: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
    corresponding_extensions: None,
};
