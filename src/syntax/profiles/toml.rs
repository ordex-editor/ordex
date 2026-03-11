//! TOML syntax profile and lexing rules.

use crate::syntax::engine::{HighlightSpan, LineLexMode, LineParseResult};
use crate::syntax::helpers::{is_ident_start, scan_identifier, scan_number, starts_with};
use crate::syntax::profile::{
    CommentFlavor, CommentStyle, CommentStyleKind, LanguageDetection, LanguageId, LanguageProfile,
    NestedLanguageHook, SyntaxClass,
};

const COMMENT_STYLES: &[CommentStyle] = &[CommentStyle {
    id: "line",
    flavor: CommentFlavor::Ordinary,
    kind: CommentStyleKind::Line,
    open: "#",
    close: None,
    nests: false,
    preferred_default: true,
}];

const NESTED_HOOKS: &[NestedLanguageHook] = &[];

/// Static TOML language profile.
pub(crate) const PROFILE: LanguageProfile = LanguageProfile {
    id: LanguageId::Toml,
    display_name: "TOML",
    detection: LanguageDetection {
        exact_filenames: &["Cargo.toml"],
        extensions: &["toml"],
    },
    comment_styles: COMMENT_STYLES,
    nested_hooks: NESTED_HOOKS,
    lex_line: lex_toml_line,
};

/// Lex one TOML line from the supplied entry mode.
pub(crate) fn lex_toml_line(line: &str, entry_mode: LineLexMode) -> LineParseResult {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut idx = 0;
    let mut exit_mode = entry_mode;

    // TOML multiline strings keep the entire continued portion styled until the
    // triple-quote delimiter closes again.
    match entry_mode {
        LineLexMode::TomlBasicMultiString => {
            let end_idx = consume_triple_string(&chars, 0, "\"\"\"");
            spans.push(string_span(0, end_idx));
            idx = end_idx;
            exit_mode = if starts_with(&chars, end_idx.saturating_sub(3), "\"\"\"") {
                LineLexMode::Plain
            } else {
                LineLexMode::TomlBasicMultiString
            };
        }
        LineLexMode::TomlLiteralMultiString => {
            let end_idx = consume_triple_string(&chars, 0, "'''");
            spans.push(string_span(0, end_idx));
            idx = end_idx;
            exit_mode = if starts_with(&chars, end_idx.saturating_sub(3), "'''") {
                LineLexMode::Plain
            } else {
                LineLexMode::TomlLiteralMultiString
            };
        }
        _ => {}
    }

    if exit_mode != LineLexMode::Plain {
        return LineParseResult { spans, exit_mode };
    }

    while idx < chars.len() {
        if chars[idx] == '#' {
            spans.push(comment_span(idx, chars.len()));
            break;
        }
        if starts_with(&chars, idx, "\"\"\"") {
            let end_idx = consume_triple_string(&chars, idx, "\"\"\"");
            spans.push(string_span(idx, end_idx));
            if !starts_with(&chars, end_idx.saturating_sub(3), "\"\"\"") || end_idx == idx + 3 {
                exit_mode = LineLexMode::TomlBasicMultiString;
                break;
            }
            idx = end_idx;
            continue;
        }
        if starts_with(&chars, idx, "'''") {
            let end_idx = consume_triple_string(&chars, idx, "'''");
            spans.push(string_span(idx, end_idx));
            if !starts_with(&chars, end_idx.saturating_sub(3), "'''") || end_idx == idx + 3 {
                exit_mode = LineLexMode::TomlLiteralMultiString;
                break;
            }
            idx = end_idx;
            continue;
        }
        if chars[idx] == '"' || chars[idx] == '\'' {
            let end_idx = consume_single_line_string(&chars, idx, chars[idx]);
            spans.push(string_span(idx, end_idx));
            idx = end_idx;
            continue;
        }
        if chars[idx].is_ascii_digit()
            || matches!(chars[idx], '+' | '-')
                && chars.get(idx + 1).is_some_and(|next| next.is_ascii_digit())
        {
            let end_idx = scan_number(&chars, idx);
            spans.push(number_span(idx, end_idx));
            idx = end_idx;
            continue;
        }
        if is_ident_start(chars[idx]) {
            let end_idx = scan_identifier(&chars, idx);
            let token: String = chars[idx..end_idx].iter().collect();
            if token == "true" || token == "false" || looks_like_bare_key(&chars, idx, end_idx) {
                spans.push(keyword_span(idx, end_idx));
            }
            idx = end_idx;
            continue;
        }
        if matches!(chars[idx], '[' | ']' | '{' | '}' | '=' | '.' | ',' | ':') {
            spans.push(punctuation_span(idx, idx + 1));
        }
        idx += 1;
    }

    LineParseResult { spans, exit_mode }
}

/// Consume a TOML triple-quoted string and return its exclusive end column.
fn consume_triple_string(chars: &[char], start: usize, delimiter: &str) -> usize {
    let mut idx = start + 3;
    while idx <= chars.len().saturating_sub(3) {
        if starts_with(chars, idx, delimiter) {
            return idx + 3;
        }
        idx += 1;
    }
    chars.len()
}

/// Consume a TOML single-line string and return its exclusive end column.
fn consume_single_line_string(chars: &[char], start: usize, quote: char) -> usize {
    let mut idx = start + 1;
    let mut escaped = false;

    // Basic strings honor backslash escaping while literal strings do not.
    while idx < chars.len() {
        let ch = chars[idx];
        if quote == '"' && !escaped && ch == '\\' {
            escaped = true;
            idx += 1;
            continue;
        }
        if quote == '"' && escaped {
            escaped = false;
            idx += 1;
            continue;
        }
        if ch == quote {
            return idx + 1;
        }
        idx += 1;
    }

    chars.len()
}

/// Return whether the identifier is a bare key immediately followed by `=`.
fn looks_like_bare_key(chars: &[char], start: usize, end: usize) -> bool {
    let mut idx = end;
    while idx < chars.len() && chars[idx].is_whitespace() {
        idx += 1;
    }
    idx < chars.len() && chars[idx] == '=' && start < end
}

/// Build a TOML comment span.
fn comment_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Comment,
        modifier: None,
    }
}

/// Build a TOML string span.
fn string_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::String,
        modifier: None,
    }
}

/// Build a TOML number span.
fn number_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Number,
        modifier: None,
    }
}

/// Build a TOML key or boolean span.
fn keyword_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Keyword,
        modifier: None,
    }
}

/// Build a TOML punctuation span.
fn punctuation_span(start_col: usize, end_col: usize) -> HighlightSpan {
    HighlightSpan {
        line_index: 0,
        start_col,
        end_col,
        class: SyntaxClass::Punctuation,
        modifier: None,
    }
}
