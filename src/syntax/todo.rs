use crate::syntax::engine::HighlightSpan;
use crate::syntax::helpers::LineCursor;
use crate::syntax::profile::{SpanStyle, TODO_STYLE};

/// Keywords that should be highlighted as TODO markers.
const TODO_KEYWORDS: &[&str] = &["TODO", "FIXME", "XXX", "HACK", "NOTE"];

/// Return true if the character is considered a word boundary for TODO markers.
fn is_word_boundary(c: char) -> bool {
    !c.is_alphanumeric() && c != '_'
}

/// Check if the cursor is at the start of a TODO marker.
pub(crate) fn find_todo_marker(cursor: &LineCursor, text: &str) -> Option<(&'static str, usize)> {
    let is_boundary_before = cursor.col() == 0 || cursor.prev().is_some_and(is_word_boundary);
    if !is_boundary_before {
        return None;
    }
    for &kw in TODO_KEYWORDS {
        if cursor.starts_with(kw) {
            let end_byte = cursor.mark().byte_pos + kw.len();
            let char_after = text[end_byte..].chars().next();
            let is_boundary_after = char_after.is_none_or(is_word_boundary);
            if is_boundary_after {
                return Some((kw, kw.chars().count()));
            }
        }
    }
    None
}

/// Split a text region into multiple spans, highlighting TODO keywords.
pub(crate) fn split_todo_spans(
    start_col: usize,
    text: &str,
    base_style: SpanStyle,
) -> Vec<HighlightSpan> {
    let mut spans = Vec::new();
    let mut cursor = LineCursor::new(text);
    let mut current_text_start_col = start_col;

    while !cursor.is_empty() {
        let is_boundary_before = cursor.col() == 0 || cursor.prev().is_some_and(is_word_boundary);

        let mut matched_keyword = None;
        if is_boundary_before {
            for &kw in TODO_KEYWORDS {
                if cursor.starts_with(kw) {
                    let end_byte = cursor.mark().byte_pos + kw.len();
                    let char_after = text[end_byte..].chars().next();
                    let is_boundary_after = char_after.is_none_or(is_word_boundary);

                    if is_boundary_after {
                        matched_keyword = Some(kw);
                        break;
                    }
                }
            }
        }

        if let Some(kw) = matched_keyword {
            let kw_start_col = start_col + cursor.col();
            if kw_start_col > current_text_start_col {
                spans.push(HighlightSpan::styled(
                    current_text_start_col,
                    kw_start_col,
                    base_style,
                ));
            }

            for _ in 0..kw.chars().count() {
                cursor.advance_char();
            }

            spans.push(HighlightSpan::styled(
                kw_start_col,
                start_col + cursor.col(),
                TODO_STYLE,
            ));
            current_text_start_col = start_col + cursor.col();
        } else {
            cursor.advance_char();
        }
    }

    if start_col + cursor.col() > current_text_start_col {
        spans.push(HighlightSpan::styled(
            current_text_start_col,
            start_col + cursor.col(),
            base_style,
        ));
    }

    spans
}
