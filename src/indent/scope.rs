//! Structural indentation derived from enclosing bracket scopes.
//!
//! Every function here is pure: callers resolve the enclosing bracket scopes to
//! the indent of each opener's line, and these rules turn that structure plus
//! the target line's own leading tokens into an indent column count. Deriving
//! the indent from nesting rather than from the previous line's indent keeps
//! each line's result absolute, so one mis-indented line cannot cascade into the
//! lines below it.

use crate::indent::{significant_last_char, structural_token_is_code_column};
use crate::syntax::HighlightSpan;

/// Return the structural base indent for one line.
///
/// `enclosing_indents` holds the indent of the line that opens each bracket
/// still open at the start of the target line, ordered outermost first. A line
/// whose first tokens are closing delimiters returns to the indent of the line
/// that opened the outermost bracket those closers finish. Every other line is
/// indented one step inside its innermost enclosing bracket, or sits at column
/// zero when no bracket encloses it.
///
/// Anchoring on the opener's line rather than on a nesting count collapses
/// several brackets opened on one line into a single indent step, which matches
/// how block bodies are conventionally formatted.
pub(crate) fn base_indent(
    enclosing_indents: &[usize],
    line: &str,
    spans: &[HighlightSpan],
    indent_width: usize,
) -> usize {
    let closers = leading_closer_count(line, spans);
    if closers > 0 {
        // The outermost bracket finished by this line's leading closers decides
        // the alignment, so the line lands back on that opener's own indent.
        let outermost_closed = enclosing_indents.len().saturating_sub(closers);
        return enclosing_indents
            .get(outermost_closed)
            .copied()
            .unwrap_or(0);
    }

    // Interior lines sit one step inside the innermost enclosing bracket.
    enclosing_indents
        .last()
        .map_or(0, |opener_indent| opener_indent + indent_width)
}

/// Return how many closing delimiters open `line`.
///
/// Counts the run of `)`, `]`, and `}` characters that begins the line's code
/// text, ignoring whitespace between them. Returns `0` when the first code
/// character is not a closing delimiter, and when the line begins inside a
/// comment or string where delimiters carry no structural meaning.
pub(crate) fn leading_closer_count(line: &str, spans: &[HighlightSpan]) -> usize {
    let mut count = 0;
    for (byte_offset, character) in line.char_indices() {
        // Whitespace never interrupts a run of closing delimiters.
        if character.is_whitespace() {
            continue;
        }
        let column = line[..byte_offset].chars().count();
        // Delimiters inside comments or strings are text, not structure.
        if !structural_token_is_code_column(spans, column) {
            break;
        }
        if !matches!(character, ')' | ']' | '}') {
            break;
        }
        count += 1;
    }
    count
}

/// Return whether `line` opens with one head aligned to its statement head.
///
/// Returns `true` for lines beginning with a closing delimiter, with a block
/// opening brace, or with the `else` keyword. Those heads belong to the
/// statement that precedes them and therefore keep that statement's own indent
/// instead of receiving continuation indent. Returns `false` for every other
/// line, including lines that begin inside a comment or string.
pub(crate) fn starts_with_aligned_head(line: &str, spans: &[HighlightSpan]) -> bool {
    let Some(code) = leading_code_slice(line, spans) else {
        return false;
    };
    let Some(first) = code.chars().next() else {
        return false;
    };
    if matches!(first, ')' | ']' | '}' | '{') {
        return true;
    }
    starts_with_else_keyword(code)
}

/// Return whether `line` leaves one statement unterminated.
///
/// Returns `true` when the last significant character continues the statement
/// onto the following line. Returns `false` for lines ending in a statement
/// terminator (`;`), a block delimiter (`{` or `}`), a separating comma, and
/// for lines with no significant character at all. A trailing comma separates
/// list elements rather than continuing a statement, so it never contributes
/// continuation indent.
pub(crate) fn line_continues_statement(line: &str, spans: &[HighlightSpan]) -> bool {
    !matches!(
        significant_last_char(line, spans),
        None | Some(';' | '}' | '{' | ',')
    )
}

/// Return whether `line` does nothing but return from enclosing brackets.
///
/// Returns `true` when the line both begins and ends with closing delimiters, so
/// its content is exhausted by closing brackets that were opened earlier. Such a
/// line has already returned to the indent of the statement that owns it, and a
/// method call chained onto it continues at that same indent rather than one
/// step further in. Returns `false` for every line that carries other code,
/// including a closer followed by more of an expression.
pub(crate) fn line_only_closes_brackets(line: &str, spans: &[HighlightSpan]) -> bool {
    leading_closer_count(line, spans) > 0
        && matches!(significant_last_char(line, spans), Some(')' | ']' | '}'))
}

/// Return the line text starting at its first code character.
///
/// Returns `None` when the line holds no non-whitespace character, and when
/// that first character belongs to a comment or string span.
fn leading_code_slice<'a>(line: &'a str, spans: &[HighlightSpan]) -> Option<&'a str> {
    for (byte_offset, character) in line.char_indices() {
        if character.is_whitespace() {
            continue;
        }
        let column = line[..byte_offset].chars().count();
        if !structural_token_is_code_column(spans, column) {
            return None;
        }
        return Some(&line[byte_offset..]);
    }
    None
}

/// Return whether `code` begins with the standalone `else` keyword.
///
/// Returns `true` only when `else` is followed by the end of the text or by a
/// character that cannot continue an identifier; returns `false` otherwise, so
/// identifiers such as `elsewhere` are not mistaken for the keyword.
fn starts_with_else_keyword(code: &str) -> bool {
    code.strip_prefix("else").is_some_and(|remainder| {
        remainder
            .chars()
            .next()
            .is_none_or(|character| !character.is_alphanumeric() && character != '_')
    })
}

#[cfg(test)]
mod tests {
    use super::{
        base_indent, leading_closer_count, line_continues_statement, line_only_closes_brackets,
        starts_with_aligned_head,
    };
    use crate::syntax::{HighlightSpan, SyntaxClass};

    /// Build one comment span covering `range` for lexical-context tests.
    fn comment_span(start_col: usize, end_col: usize) -> Vec<HighlightSpan> {
        vec![HighlightSpan {
            start_col,
            end_col,
            class: SyntaxClass::Comment,
            modifier: None,
        }]
    }

    /// Interior lines indent one step inside the innermost enclosing bracket.
    #[test]
    fn base_indent_indents_inside_innermost_bracket() {
        assert_eq!(base_indent(&[0], "body;", &[], 4), 4);
        assert_eq!(base_indent(&[0, 4], "body;", &[], 4), 8);
        // No enclosing bracket leaves the line at the left margin.
        assert_eq!(base_indent(&[], "fn f() {", &[], 4), 0);
    }

    /// Brackets opened on one line collapse into a single indent step.
    #[test]
    fn base_indent_collapses_openers_sharing_one_line() {
        // Both `(` openers of `foo(bar(` sit on a line indented by four.
        assert_eq!(base_indent(&[0, 4, 4], "baz,", &[], 4), 8);
    }

    /// A leading closer aligns to the line that opened its bracket.
    #[test]
    fn base_indent_aligns_leading_closer_to_its_opener_line() {
        assert_eq!(base_indent(&[0, 4], "}", &[], 4), 4);
        assert_eq!(base_indent(&[0], "}", &[], 4), 0);
        // `} else {` keeps the alignment of a plain closer.
        assert_eq!(base_indent(&[0, 4], "} else {", &[], 4), 4);
    }

    /// Several leading closers align to the outermost bracket they finish.
    #[test]
    fn base_indent_aligns_multiple_closers_to_outermost_opener() {
        // Stack `{`@0, `(`@4, `{`@4 closed by `});` returns to the `(` opener.
        assert_eq!(base_indent(&[0, 4, 4], "});", &[], 4), 4);
        // More closers than open brackets clamp to the outermost known opener.
        assert_eq!(base_indent(&[8], ")))", &[], 4), 8);
        // With no enclosing bracket at all a closer stays at the left margin.
        assert_eq!(base_indent(&[], ")", &[], 4), 0);
    }

    /// Closing delimiters inside comments do not count as leading closers.
    #[test]
    fn leading_closer_count_ignores_comment_delimiters() {
        assert_eq!(leading_closer_count("});", &[]), 2);
        assert_eq!(leading_closer_count("  }  )", &[]), 2);
        assert_eq!(leading_closer_count("value,", &[]), 0);
        // A line whose first code column is a comment yields no closers.
        assert_eq!(leading_closer_count("// }", &comment_span(0, 4)), 0);
    }

    /// Aligned heads cover closers, block openers, and the `else` keyword.
    #[test]
    fn aligned_head_detection_matches_statement_heads() {
        assert!(starts_with_aligned_head("{", &[]));
        assert!(starts_with_aligned_head("} else {", &[]));
        assert!(starts_with_aligned_head("else {", &[]));
        assert!(starts_with_aligned_head(")", &[]));
        assert!(!starts_with_aligned_head("elsewhere();", &[]));
        assert!(!starts_with_aligned_head("+ value", &[]));
    }

    /// Bracket-only lines are distinguished from closers carrying more code.
    #[test]
    fn bracket_only_lines_exclude_closers_followed_by_code() {
        assert!(line_only_closes_brackets(")", &[]));
        assert!(line_only_closes_brackets("    })", &[]));
        // A closer that continues an expression still carries code.
        assert!(!line_only_closes_brackets(") + value", &[]));
        // Terminated closers end the statement instead of returning to it.
        assert!(!line_only_closes_brackets(");", &[]));
        assert!(!line_only_closes_brackets("value", &[]));
    }

    /// Statement continuation excludes terminators, delimiters, and commas.
    #[test]
    fn statement_continuation_excludes_terminators_and_commas() {
        assert!(line_continues_statement("let x = a", &[]));
        assert!(line_continues_statement("    .method()", &[]));
        assert!(!line_continues_statement("let x = a;", &[]));
        assert!(!line_continues_statement("fn f() {", &[]));
        assert!(!line_continues_statement("}", &[]));
        // A trailing comma separates elements instead of continuing a statement.
        assert!(!line_continues_statement("    value,", &[]));
        // Trailing comment text never changes the significant terminator.
        assert!(!line_continues_statement(
            "let x = a; // note",
            &comment_span(11, 18)
        ));
    }

    /// A comma inside a string is text, not a list separator.
    #[test]
    fn statement_continuation_ignores_string_commas() {
        let line = "const string: &str = r#\"hello,";
        let string_start = line.find("r#\"").expect("string opener should exist");
        let spans = vec![HighlightSpan {
            start_col: string_start,
            end_col: line.chars().count(),
            class: SyntaxClass::String,
            modifier: None,
        }];

        // The significant terminator is `=`, so the statement still continues
        // even though the raw text ends with a comma.
        assert!(line_continues_statement(line, &spans));
    }
}
