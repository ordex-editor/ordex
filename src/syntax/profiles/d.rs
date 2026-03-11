//! D syntax profile and lexing rules.

use crate::syntax::engine::{HighlightSpan, LineLexMode, LineParseResult};
use crate::syntax::helpers::{is_ident_start, scan_identifier, scan_number, starts_with};
use crate::syntax::profile::{
    CommentFlavor, CommentStyle, CommentStyleKind, LanguageDetection, LanguageId, LanguageProfile,
    NestedLanguageHook, SyntaxClass, SyntaxModifier,
};

const KEYWORDS: &[&str] = &[
    "alias",
    "auto",
    "break",
    "case",
    "class",
    "const",
    "continue",
    "debug",
    "else",
    "enum",
    "false",
    "foreach",
    "foreach_reverse",
    "if",
    "immutable",
    "import",
    "in",
    "interface",
    "module",
    "new",
    "private",
    "public",
    "return",
    "shared",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "true",
    "void",
    "while",
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
        id: "line_doc",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Line,
        open: "///",
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
        nests: false,
        preferred_default: false,
    },
    CommentStyle {
        id: "block_doc",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Block,
        open: "/**",
        close: Some("*/"),
        nests: false,
        preferred_default: false,
    },
    CommentStyle {
        id: "nested",
        flavor: CommentFlavor::Ordinary,
        kind: CommentStyleKind::Block,
        open: "/+",
        close: Some("+/"),
        nests: true,
        preferred_default: false,
    },
    CommentStyle {
        id: "nested_doc",
        flavor: CommentFlavor::Documentation,
        kind: CommentStyleKind::Block,
        open: "/++",
        close: Some("+/"),
        nests: true,
        preferred_default: false,
    },
];

const NESTED_HOOKS: &[NestedLanguageHook] = &[];

/// Static D language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::D,
    display_name: "D",
    detection: LanguageDetection {
        exact_filenames: &[],
        extensions: &["d"],
    },
    comment_styles: COMMENT_STYLES,
    nested_hooks: NESTED_HOOKS,
    lex_line: lex_d_line,
};

/// Lex one D source line from the supplied entry mode.
pub(crate) fn lex_d_line(line: &str, entry_mode: LineLexMode) -> LineParseResult {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut idx = 0;
    let mut exit_mode = entry_mode;

    // D inherits multiline state for both classic block comments and nested
    // `/+ +/` comments, so the carried mode determines which terminator rules apply.
    if let LineLexMode::DBlockComment { nested, depth, doc } = entry_mode {
        let delimiter = if nested { ("/+", "+/") } else { ("/*", "*/") };
        let (end_idx, end_depth) =
            consume_d_block_comment(&chars, 0, delimiter.0, delimiter.1, depth, nested, false);
        spans.push(comment_span(0, end_idx, doc));
        idx = end_idx;
        exit_mode = if end_depth == 0 {
            LineLexMode::Plain
        } else {
            LineLexMode::DBlockComment {
                nested,
                depth: end_depth,
                doc,
            }
        };
    }

    if exit_mode != LineLexMode::Plain {
        return LineParseResult { spans, exit_mode };
    }

    while idx < chars.len() {
        if starts_with(&chars, idx, "///") {
            spans.push(comment_span(idx, chars.len(), true));
            break;
        }
        if starts_with(&chars, idx, "//") {
            spans.push(comment_span(idx, chars.len(), false));
            break;
        }
        if starts_with(&chars, idx, "/++") {
            let (end_idx, end_depth) =
                consume_d_block_comment(&chars, idx, "/+", "+/", 1, true, true);
            spans.push(comment_span(idx, end_idx, true));
            if end_depth > 0 {
                exit_mode = LineLexMode::DBlockComment {
                    nested: true,
                    depth: end_depth,
                    doc: true,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if starts_with(&chars, idx, "/+") {
            let (end_idx, end_depth) =
                consume_d_block_comment(&chars, idx, "/+", "+/", 1, true, true);
            spans.push(comment_span(idx, end_idx, false));
            if end_depth > 0 {
                exit_mode = LineLexMode::DBlockComment {
                    nested: true,
                    depth: end_depth,
                    doc: false,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if starts_with(&chars, idx, "/**") {
            let (end_idx, end_depth) =
                consume_d_block_comment(&chars, idx, "/*", "*/", 1, false, true);
            spans.push(comment_span(idx, end_idx, true));
            if end_depth > 0 {
                exit_mode = LineLexMode::DBlockComment {
                    nested: false,
                    depth: end_depth,
                    doc: true,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if starts_with(&chars, idx, "/*") {
            let (end_idx, end_depth) =
                consume_d_block_comment(&chars, idx, "/*", "*/", 1, false, true);
            spans.push(comment_span(idx, end_idx, false));
            if end_depth > 0 {
                exit_mode = LineLexMode::DBlockComment {
                    nested: false,
                    depth: end_depth,
                    doc: false,
                };
                break;
            }
            idx = end_idx;
            continue;
        }
        if chars[idx] == '"' {
            let end_idx = consume_string(&chars, idx);
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
        if matches!(
            chars[idx],
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
        ) {
            spans.push(punctuation_span(idx, idx + 1));
        }
        idx += 1;
    }

    LineParseResult { spans, exit_mode }
}

/// Consume one D block comment and return the exclusive end column and depth.
fn consume_d_block_comment(
    chars: &[char],
    start: usize,
    open: &str,
    close: &str,
    initial_depth: usize,
    nested: bool,
    initial_open_consumed: bool,
) -> (usize, usize) {
    let mut idx = start;
    let mut depth = initial_depth;

    // Nested `/+ +/` comments increase depth on each nested opener, while
    // classic `/* */` comments only watch for the first closing delimiter.
    while idx < chars.len() {
        if nested && starts_with(chars, idx, open) {
            if !(initial_open_consumed && idx == start) {
                depth += 1;
            }
            idx += open.chars().count();
            continue;
        }
        if starts_with(chars, idx, close) {
            depth = depth.saturating_sub(1);
            idx += close.chars().count();
            if depth == 0 {
                return (idx, 0);
            }
            continue;
        }
        idx += 1;
    }

    (chars.len(), depth)
}

/// Consume a D string literal and return the exclusive end column.
fn consume_string(chars: &[char], start: usize) -> usize {
    let mut idx = start + 1;
    let mut escaped = false;
    while idx < chars.len() {
        let ch = chars[idx];
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return idx + 1;
        }
        idx += 1;
    }
    chars.len()
}

/// Build a D comment span.
fn comment_span(start_col: usize, end_col: usize, doc: bool) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Comment,
        modifier: doc.then_some(SyntaxModifier::DocComment),
    }
}

/// Build a D string span.
fn string_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::String,
        modifier: None,
    }
}

/// Build a D number span.
fn number_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Number,
        modifier: None,
    }
}

/// Build a D keyword span.
fn keyword_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Keyword,
        modifier: None,
    }
}

/// Build a D punctuation span.
fn punctuation_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Punctuation,
        modifier: None,
    }
}
