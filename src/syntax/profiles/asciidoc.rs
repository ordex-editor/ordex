//! AsciiDoc syntax profile.

use crate::syntax::profile::*;

const COMMENT_STYLES: &[CommentStyle] = &[preferred_line_comment("//")];
const MARKUP_RULES: MarkupRules = MarkupRules {
    thematic_break: None,
    heading_rules: &[markup_heading_rule('=', 1, 6)],
    block_quote_prefixes: &[],
    list_rules: &[
        repeated_marker_list_rule('-', 1),
        repeated_marker_list_rule('*', 1),
        repeated_marker_list_rule('.', 1),
    ],
    fence_markers: &['-', '.', '*', '_', '+', '=', '/'],
    comment_fence_markers: &['/'],
    min_fence_len: 4,
    inline_delimited_rules: &[InlineDelimitedMarkupRule {
        delimiter: "+",
        min_span_len: 3,
        boundary: InlineDelimiterBoundary::EmphasisLike,
        modifier: SyntaxModifier::InlineCode,
    }],
    inline_bracket_links: &[],
    inline_prefixed_bracket_spans: &[InlinePrefixedBracketSpanRule {
        prefixes: &["link:", "xref:", "http://", "https://", "mailto:"],
        bracket_open: '[',
        bracket_close: ']',
    }],
    inline_balanced_pair_spans: &[inline_balanced_pair_rule("<<", ">>")],
};

/// Static AsciiDoc language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::AsciiDoc,
    display_name: "AsciiDoc",
    exact_filenames: &[],
    extensions: &["adoc", "asciidoc", "asc"],
    comment_styles: COMMENT_STYLES,
    string_styles: &[],
    identifier: None,
    identifier_rules: &[],
    punctuation_chars: "",
    number_pattern: UNSIGNED_NUMBER,
    markup_rules: Some(MARKUP_RULES),
    manual_indent: KEEP_PREVIOUS_LINE_INDENT,
    nested_hooks: &[],
};
