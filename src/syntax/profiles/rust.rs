//! Rust syntax profile and lexing rules.

use crate::syntax::engine::{HighlightSpan, LineLexMode, LineParseResult};
use crate::syntax::helpers::{
    is_ident_start, previous_char, scan_identifier, scan_number, starts_with,
};
use crate::syntax::profile::{
    CommentFlavor, CommentStyle, CommentStyleKind, LanguageDetection, LanguageId, LanguageProfile,
    NestedLanguageHook, SyntaxClass, SyntaxModifier,
};

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "trait", "true", "type", "unsafe", "use",
    "where", "while",
];

const COMMENT_STYLES: &[CommentStyle] = &[
    CommentStyle {
        id: "line",
        flavor: CommentFlavor::Ordinary,
        kind: CommentStyleKind::Line,
        open: "//",
        close: None,
        nests: false,
        preferred_default: true,
    },
    CommentStyle {
        id: "line_doc_outer",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Line,
        open: "///",
        close: None,
        nests: false,
        preferred_default: false,
    },
    CommentStyle {
        id: "line_doc_inner",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Line,
        open: "//!",
        close: None,
        nests: false,
        preferred_default: false,
    },
    CommentStyle {
        id: "block",
        flavor: CommentFlavor::Ordinary,
        kind: CommentStyleKind::Block,
        open: "/*",
        close: Some("*/"),
        nests: true,
        preferred_default: false,
    },
    CommentStyle {
        id: "block_doc_outer",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Block,
        open: "/**",
        close: Some("*/"),
        nests: true,
        preferred_default: false,
    },
    CommentStyle {
        id: "block_doc_inner",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Block,
        open: "/*!",
        close: Some("*/"),
        nests: true,
        preferred_default: false,
    },
];

const NESTED_HOOKS: &[NestedLanguageHook] = &[];

/// Static Rust language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Rust,
    display_name: "Rust",
    detection: LanguageDetection {
        exact_filenames: &[],
        extensions: &["rs"],
    },
    comment_styles: COMMENT_STYLES,
    nested_hooks: NESTED_HOOKS,
    lex_line: lex_rust_line,
};

/// Lex one Rust source line from the supplied entry mode.
pub(crate) fn lex_rust_line(line: &str, entry_mode: LineLexMode) -> LineParseResult {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut idx = 0;
    let mut exit_mode = entry_mode;

    // Continue any multiline construct inherited from the previous line before
    // scanning ordinary tokens on this line.
    match entry_mode {
        LineLexMode::RustBlockComment { depth, doc } => {
            let (end_idx, end_depth) = consume_rust_block_comment(&chars, 0, depth, false);
            spans.push(comment_span(0, end_idx, doc));
            idx = end_idx;
            exit_mode = if end_depth == 0 {
                LineLexMode::Plain
            } else {
                LineLexMode::RustBlockComment {
                    depth: end_depth,
                    doc,
                }
            };
        }
        LineLexMode::RustRawString { hashes } => {
            let (end_idx, closed) = consume_rust_raw_string(&chars, 0, hashes);
            spans.push(string_span(0, end_idx));
            idx = end_idx;
            exit_mode = if closed {
                LineLexMode::Plain
            } else {
                LineLexMode::RustRawString { hashes }
            };
        }
        _ => {}
    }

    if exit_mode != LineLexMode::Plain {
        return LineParseResult { spans, exit_mode };
    }

    while idx < chars.len() {
        if starts_with(&chars, idx, "///") || starts_with(&chars, idx, "//!") {
            spans.push(comment_span(idx, chars.len(), true));
            break;
        }
        if starts_with(&chars, idx, "//") {
            spans.push(comment_span(idx, chars.len(), false));
            break;
        }
        if starts_with(&chars, idx, "/**") || starts_with(&chars, idx, "/*!") {
            let (end_idx, end_depth) = consume_rust_block_comment(&chars, idx, 1, true);
            spans.push(comment_span(idx, end_idx, true));
            if end_depth > 0 {
                exit_mode = LineLexMode::RustBlockComment {
                    depth: end_depth,
                    doc: true,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if starts_with(&chars, idx, "/*") {
            let (end_idx, end_depth) = consume_rust_block_comment(&chars, idx, 1, true);
            spans.push(comment_span(idx, end_idx, false));
            if end_depth > 0 {
                exit_mode = LineLexMode::RustBlockComment {
                    depth: end_depth,
                    doc: false,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if let Some(hashes) = raw_string_hashes(&chars, idx) {
            let (end_idx, closed) = consume_rust_raw_string(&chars, idx, hashes);
            spans.push(string_span(idx, end_idx));
            if !closed {
                exit_mode = LineLexMode::RustRawString { hashes };
                break;
            }
            idx = end_idx;
            continue;
        }
        if chars[idx] == '"' {
            let end_idx = consume_quoted_string(&chars, idx, '"');
            spans.push(string_span(idx, end_idx));
            idx = end_idx;
            continue;
        }
        if chars[idx].is_ascii_digit()
            || (chars[idx] == '-' && chars.get(idx + 1).is_some_and(|next| next.is_ascii_digit()))
        {
            let end_idx = scan_number(&chars, idx);
            spans.push(number_span(idx, end_idx));
            idx = end_idx;
            continue;
        }
        if is_ident_start(chars[idx]) {
            let end_idx = scan_identifier(&chars, idx);
            let token: String = chars[idx..end_idx].iter().collect();
            if KEYWORDS.contains(&token.as_str()) {
                spans.push(keyword_span(idx, end_idx));
            }
            idx = end_idx;
            continue;
        }
        if is_rust_punctuation(chars[idx], previous_char(&chars, idx)) {
            spans.push(punctuation_span(idx, idx + 1));
        }
        idx += 1;
    }

    LineParseResult { spans, exit_mode }
}

/// Consume a Rust nested block comment and return the ending column and depth.
fn consume_rust_block_comment(
    chars: &[char],
    start: usize,
    initial_depth: usize,
    initial_open_consumed: bool,
) -> (usize, usize) {
    let mut idx = start;
    let mut depth = initial_depth;

    // Rust block comments nest, so each `/*` increments depth and each `*/`
    // decrements it until the carried state returns to plain text.
    while idx < chars.len() {
        if starts_with(chars, idx, "/*") {
            if !(initial_open_consumed && idx == start) {
                depth += 1;
            }
            idx += 2;
            continue;
        }
        if starts_with(chars, idx, "*/") {
            depth = depth.saturating_sub(1);
            idx += 2;
            if depth == 0 {
                return (idx, 0);
            }
            continue;
        }
        idx += 1;
    }

    (chars.len(), depth)
}

/// Return the raw-string hash count when a Rust raw string starts at `idx`.
fn raw_string_hashes(chars: &[char], idx: usize) -> Option<usize> {
    if chars.get(idx) != Some(&'r') {
        return None;
    }
    let mut hashes = 0;
    while chars.get(idx + 1 + hashes) == Some(&'#') {
        hashes += 1;
    }
    (chars.get(idx + 1 + hashes) == Some(&'"')).then_some(hashes)
}

/// Consume a Rust raw string and return its end column plus closed/open status.
fn consume_rust_raw_string(chars: &[char], start: usize, hashes: usize) -> (usize, bool) {
    let mut idx = start + 2 + hashes;
    while idx < chars.len() {
        if chars[idx] == '"' && raw_string_closed(chars, idx + 1, hashes) {
            return (idx + 1 + hashes, true);
        }
        idx += 1;
    }
    (chars.len(), false)
}

/// Return whether a raw string is closed by `"` followed by `hashes` markers.
fn raw_string_closed(chars: &[char], start: usize, hashes: usize) -> bool {
    (0..hashes).all(|offset| chars.get(start + offset) == Some(&'#'))
}

/// Consume a quoted single-line string and return its exclusive end column.
fn consume_quoted_string(chars: &[char], start: usize, quote: char) -> usize {
    let mut idx = start + 1;
    let mut escaped = false;
    while idx < chars.len() {
        let ch = chars[idx];
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return idx + 1;
        }
        idx += 1;
    }
    chars.len()
}

/// Return whether `ch` should be styled as Rust punctuation.
fn is_rust_punctuation(ch: char, prev: Option<char>) -> bool {
    matches!(
        ch,
        '{' | '}'
            | '['
            | ']'
            | '('
            | ')'
            | ';'
            | ':'
            | ','
            | '.'
            | '='
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '&'
            | '|'
            | '^'
            | '!'
            | '?'
            | '<'
            | '>'
    ) && !prev.is_some_and(|prev| prev.is_ascii_digit() && ch == '.')
}

/// Build a comment span for Rust output.
fn comment_span(start_col: usize, end_col: usize, doc: bool) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Comment,
        modifier: doc.then_some(SyntaxModifier::DocComment),
    }
}

/// Build a string span for Rust output.
fn string_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::String,
        modifier: None,
    }
}

/// Build a number span for Rust output.
fn number_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Number,
        modifier: None,
    }
}

/// Build a keyword span for Rust output.
fn keyword_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Keyword,
        modifier: None,
    }
}

/// Build a punctuation span for Rust output.
fn punctuation_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Punctuation,
        modifier: None,
    }
}
