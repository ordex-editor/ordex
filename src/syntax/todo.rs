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
    for &keyword in TODO_KEYWORDS {
        if cursor.starts_with(keyword) {
            let end_byte = cursor.mark().byte_pos + keyword.len();
            let char_after = text[end_byte..].chars().next();
            let is_boundary_after = char_after.is_none_or(is_word_boundary);
            if is_boundary_after {
                return Some((keyword, keyword.chars().count()));
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
        if let Some((_, len)) = find_todo_marker(&cursor, text) {
            let keyword_start_col = start_col + cursor.col();
            if keyword_start_col > current_text_start_col {
                spans.push(HighlightSpan::styled(
                    current_text_start_col,
                    keyword_start_col,
                    base_style,
                ));
            }

            for _ in 0..len {
                cursor.advance_char();
            }

            spans.push(HighlightSpan::styled(
                keyword_start_col,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::profile::{SyntaxClass, SyntaxModifier};

    #[test]
    fn test_find_todo_marker() {
        let text = "TODO: fix this";
        let cursor = LineCursor::new(text);
        assert_eq!(find_todo_marker(&cursor, text), Some(("TODO", 4)));

        let text = "TODOS: no";
        let cursor = LineCursor::new(text);
        assert_eq!(find_todo_marker(&cursor, text), None);

        let text = "  FIXME";
        let mut cursor = LineCursor::new(text);
        cursor.advance_char();
        cursor.advance_char();
        assert_eq!(find_todo_marker(&cursor, text), Some(("FIXME", 5)));

        let text = "my_todo";
        let mut cursor = LineCursor::new(text);
        for _ in 0..3 {
            cursor.advance_char();
        } // point to 't'
        assert_eq!(find_todo_marker(&cursor, text), None);
    }

    #[test]
    fn test_split_todo_spans() {
        let base_style = SpanStyle::new(SyntaxClass::Comment, None);
        let text = "// TODO: fix";
        let spans = split_todo_spans(0, text, base_style);

        assert_eq!(spans.len(), 3);

        // "// "
        assert_eq!(spans[0].start_col, 0);
        assert_eq!(spans[0].end_col, 3);
        assert_eq!(spans[0].class, SyntaxClass::Comment);
        assert_eq!(spans[0].modifier, None);

        // "TODO"
        assert_eq!(spans[1].start_col, 3);
        assert_eq!(spans[1].end_col, 7);
        assert_eq!(spans[1].class, SyntaxClass::Comment);
        assert_eq!(spans[1].modifier, Some(SyntaxModifier::Todo));

        // ": fix"
        assert_eq!(spans[2].start_col, 7);
        assert_eq!(spans[2].end_col, 12);
        assert_eq!(spans[2].class, SyntaxClass::Comment);
        assert_eq!(spans[2].modifier, None);
    }
}
