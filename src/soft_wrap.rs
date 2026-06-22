//! Soft-wrap layout helpers.
//!
//! In this module, a **line** means one logical buffer line, while a **row**
//! means one visible on-screen slice of a line after soft wrapping.

use crate::display_columns;
use crate::text_buffer::TextBuffer;

/// One visual row position within the wrapped document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct VisualPosition {
    /// Buffer line index containing the row.
    pub(crate) line: usize,
    /// Zero-based wrapped-row index within `line`.
    pub(crate) row: usize,
}

impl VisualPosition {
    /// Build a visual position from a buffer line and wrapped-row offset.
    pub(crate) fn new(line: usize, row: usize) -> Self {
        Self { line, row }
    }
}

/// Cursor placement within one wrapped row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisualCursor {
    /// Wrapped-row position in the document.
    pub(crate) position: VisualPosition,
    /// Zero-based display column within that wrapped row.
    pub(crate) column: usize,
}

/// Return how many wrapped rows a line occupies at the given width.
///
/// `width` is the number of text columns available for file content after the
/// gutter has been removed.
pub(crate) fn wrap_row_count(line_len: usize, width: usize) -> usize {
    let width = width.max(1);
    line_len.max(1).div_ceil(width)
}

/// Return the first buffer-column index shown by one wrapped row.
///
/// `row` is the wrapped-row index within a logical line, and `width` is the
/// number of content columns available on screen.
pub(crate) fn row_start_column(row: usize, width: usize) -> usize {
    row.saturating_mul(width.max(1))
}

/// Map one buffer cursor position to its wrapped-row location.
///
/// `width` is the number of visible content columns, not the full terminal
/// width including the gutter.
pub(crate) fn visual_cursor(
    buffer: &TextBuffer,
    line: usize,
    column: usize,
    width: usize,
    normal_mode: bool,
    tab_width: usize,
) -> VisualCursor {
    let width = width.max(1);
    let display_column = line_display_column(buffer, line, column, tab_width);
    let position = if normal_mode {
        // Normal mode never places the cursor past end-of-line, so row placement
        // only depends on the mapped display column.
        VisualPosition::new(line, display_column / width)
    } else {
        let line_display_width = line_display_width(buffer, line, tab_width);
        if display_column < line_display_width || line_display_width == 0 {
            VisualPosition::new(line, display_column / width)
        } else {
            // Insert-mode cursors may sit one cell past the last visible glyph.
            // Using `display_column / width` (without subtracting 1) naturally
            // handles both cases:
            //   - When the line does not fill the last row, the cursor lands on
            //     the same row as `(display_column - 1) / width` because integer
            //     division truncates toward zero.
            //   - When the line exactly fills the last row, the cursor lands on
            //     a new visual row past the line's content, avoiding overlap with
            //     the last character.
            VisualPosition::new(line, display_column / width)
        }
    };
    let column = display_column.saturating_sub(row_start_column(position.row, width));
    VisualCursor { position, column }
}

/// Convert a wrapped-row column back to a buffer column.
///
/// `row` is the wrapped-row index inside the logical line, `visual_column` is
/// the desired horizontal position inside that row, and `width` is the number
/// of content columns available on screen.
pub(crate) fn buffer_column_for_visual_column(
    buffer: &TextBuffer,
    line: usize,
    row: usize,
    visual_column: usize,
    width: usize,
    normal_mode: bool,
    tab_width: usize,
) -> usize {
    let width = width.max(1);
    let base_display_column = row_start_column(row, width);
    let line_display_width = line_display_width(buffer, line, tab_width);
    let max_display_column = if normal_mode {
        line_display_width.saturating_sub(1)
    } else {
        line_display_width
    };

    // Wrapped-row motions preserve the desired on-screen display column first,
    // then clamp to the last valid display position in the target line.
    let target_display_column = base_display_column
        .saturating_add(visual_column)
        .min(max_display_column);
    display_column_to_buffer_column(buffer, line, target_display_column, tab_width)
}

/// Move forward by at most `count` wrapped rows.
///
/// `width` is the number of content columns available for file text.
pub(crate) fn advance_visual_position(
    start: VisualPosition,
    buffer: &TextBuffer,
    width: usize,
    count: usize,
    tab_width: usize,
) -> VisualPosition {
    let width = width.max(1);
    let line_rows = wrap_row_count(line_display_width(buffer, start.line, tab_width), width);
    let rows_left_in_line = line_rows.saturating_sub(start.row + 1);
    // The fast path stays within the same logical line, so we can jump by row
    // arithmetic without inspecting any other lines.
    if count <= rows_left_in_line {
        return VisualPosition::new(start.line, start.row + count);
    }

    // Crossing the line boundary consumes the remaining rows in the current
    // line plus the first row transition into the next logical line.
    let mut remaining = count.saturating_sub(rows_left_in_line + 1);
    let mut line = start.line + 1;
    while line < buffer.lines_count() {
        let rows = wrap_row_count(line_display_width(buffer, line, tab_width), width);
        // Once the remaining distance fits inside one line, the wrapped-row
        // offset is exactly the remaining count from that line's first row.
        if remaining < rows {
            return VisualPosition::new(line, remaining);
        }
        // Otherwise skip the whole line at once instead of stepping row-by-row.
        remaining = remaining.saturating_sub(rows);
        line += 1;
    }

    // Clamp to the last visible wrapped row when the requested motion would
    // move past the end of the document.
    let last_line = buffer.lines_count().saturating_sub(1);
    let last_row =
        wrap_row_count(line_display_width(buffer, last_line, tab_width), width).saturating_sub(1);
    VisualPosition::new(last_line, last_row)
}

/// Move backward by at most `count` wrapped rows.
///
/// `width` is the number of content columns available for file text.
pub(crate) fn retreat_visual_position(
    start: VisualPosition,
    buffer: &TextBuffer,
    width: usize,
    count: usize,
    tab_width: usize,
) -> VisualPosition {
    let width = width.max(1);
    // The fast path stays inside the current logical line, so a subtraction is
    // enough when we do not need to cross a line boundary.
    if count <= start.row {
        return VisualPosition::new(start.line, start.row - count);
    }

    // Moving into the previous logical line consumes the current row plus the
    // boundary transition back to the prior line's last wrapped row.
    let mut remaining = count.saturating_sub(start.row + 1);
    let mut line = start.line;
    while line > 0 {
        line -= 1;
        let rows = wrap_row_count(line_display_width(buffer, line, tab_width), width);
        // If the remaining distance lands inside this line, count backward from
        // its final wrapped row to find the target row offset.
        if remaining < rows {
            return VisualPosition::new(line, rows.saturating_sub(remaining + 1));
        }
        // Otherwise skip the whole line in one subtraction and keep scanning.
        remaining = remaining.saturating_sub(rows);
    }

    // Clamp to the top-left visual position when the motion would move before
    // the start of the document.
    VisualPosition::new(0, 0)
}

/// Count how many wrapped rows lie between two visual positions.
///
/// The result is the number of row transitions needed to move from `start` to
/// `end`, so adjacent rows have a distance of 1. `width` is the number of
/// visible content columns available for text after subtracting the gutter.
pub(crate) fn visual_rows_between(
    start: VisualPosition,
    end: VisualPosition,
    buffer: &TextBuffer,
    width: usize,
    tab_width: usize,
) -> usize {
    if start == end {
        return 0;
    }

    // Count forward only once; reversing the arguments should produce the same
    // distance, so normalize the order before summing rows.
    if end < start {
        return visual_rows_between(end, start, buffer, width, tab_width);
    }

    let width = width.max(1);
    if start.line == end.line {
        // Rows inside the same logical line differ only by their wrapped-row
        // offsets, so the distance is a simple subtraction.
        return end.row.abs_diff(start.row);
    }

    // First account for the rows remaining in the starting line after `start`.
    let mut rows = wrap_row_count(line_display_width(buffer, start.line, tab_width), width)
        .saturating_sub(start.row + 1);
    for line in (start.line + 1)..end.line {
        // Whole lines between the endpoints contribute all of their wrapped rows.
        rows = rows.saturating_add(wrap_row_count(
            line_display_width(buffer, line, tab_width),
            width,
        ));
    }
    // Finally add the rows from the top of the destination line through `end`.
    rows.saturating_add(end.row + 1)
}

/// Return the display width of one buffer line under the active tab width.
pub(crate) fn line_display_width(buffer: &TextBuffer, line: usize, tab_width: usize) -> usize {
    let Some(line_text) = buffer.line_for_display(line) else {
        return 0;
    };
    display_columns::line_display_width_chars(line_text.chars(), tab_width)
}

/// Return one buffer column converted to a display column.
fn line_display_column(buffer: &TextBuffer, line: usize, column: usize, tab_width: usize) -> usize {
    let Some(line_text) = buffer.line_for_display(line) else {
        return column;
    };
    display_columns::buffer_column_to_display_column_chars(line_text.chars(), column, tab_width)
}

/// Return the buffer column covering one display column on `line`.
fn display_column_to_buffer_column(
    buffer: &TextBuffer,
    line: usize,
    display_column: usize,
    tab_width: usize,
) -> usize {
    let Some(line_text) = buffer.line_for_display(line) else {
        return display_column;
    };
    display_columns::display_column_to_buffer_column_chars(
        line_text.chars(),
        display_column,
        tab_width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small buffer used by wrapped-row helper tests.
    fn test_buffer() -> TextBuffer {
        TextBuffer::from_str("abcdef\nghij\n\nklmnop")
    }

    #[test]
    fn test_wrap_row_count_keeps_empty_lines_visible() {
        assert_eq!(wrap_row_count(0, 4), 1);
    }

    #[test]
    fn test_row_start_column_uses_content_width() {
        assert_eq!(row_start_column(2, 4), 8);
    }

    #[test]
    fn test_visual_cursor_maps_normal_mode_columns() {
        let buffer = test_buffer();
        let cursor = visual_cursor(&buffer, 0, 5, 4, true, 8);
        assert_eq!(cursor.position, VisualPosition::new(0, 1));
        assert_eq!(cursor.column, 1);
    }

    #[test]
    fn test_visual_cursor_insert_eol_exact_wrap_single_row() {
        // Line "abcd" (display_width=4) exactly fills one content row
        // (width=4).  The insert-mode cursor past EOL sits on a new
        // visual row instead of overlapping the last character.
        let buffer = TextBuffer::from_str("abcd");
        let cursor = visual_cursor(&buffer, 0, 4, 4, false, 8);
        assert_eq!(cursor.position, VisualPosition::new(0, 1));
        assert_eq!(cursor.column, 0);
    }

    #[test]
    fn test_visual_cursor_insert_eol_exact_wrap_multi_row() {
        // Line "abcdefgh" (display_width=8) wraps across two rows
        // (width=4).  The insert-mode cursor past EOL sits on a new
        // third visual row, not on the last character of row 1.
        let buffer = TextBuffer::from_str("abcdefgh");
        let cursor = visual_cursor(&buffer, 0, 8, 4, false, 8);
        assert_eq!(cursor.position, VisualPosition::new(0, 2));
        assert_eq!(cursor.column, 0);
    }

    #[test]
    fn test_visual_cursor_insert_eol_non_exact_wrap() {
        // Line "abcde" (display_width=5) wraps across two rows
        // (width=4).  The insert-mode cursor past EOL sits on the
        // last existing row, column 1 (past the last character).
        let buffer = TextBuffer::from_str("abcde");
        let cursor = visual_cursor(&buffer, 0, 5, 4, false, 8);
        assert_eq!(cursor.position, VisualPosition::new(0, 1));
        assert_eq!(cursor.column, 1);
    }

    #[test]
    fn test_visual_cursor_normal_mode_unchanged() {
        // Normal-mode cursor at end of line clamps to last character
        // regardless of wrap geometry.
        let buffer = TextBuffer::from_str("abcd");
        let cursor = visual_cursor(&buffer, 0, 3, 4, true, 8);
        assert_eq!(cursor.position, VisualPosition::new(0, 0));
        assert_eq!(cursor.column, 3);
    }

    #[test]
    fn test_buffer_column_for_visual_column_clamps_normal_mode() {
        let buffer = test_buffer();
        assert_eq!(
            buffer_column_for_visual_column(&buffer, 0, 1, 3, 4, true, 8),
            5
        );
    }

    #[test]
    fn test_advance_visual_position_crosses_line_boundaries() {
        let buffer = test_buffer();
        let position = advance_visual_position(VisualPosition::new(0, 1), &buffer, 4, 2, 8);
        assert_eq!(position, VisualPosition::new(2, 0));
    }

    #[test]
    fn test_retreat_visual_position_crosses_line_boundaries() {
        let buffer = test_buffer();
        let position = retreat_visual_position(VisualPosition::new(1, 0), &buffer, 4, 2, 8);
        assert_eq!(position, VisualPosition::new(0, 0));
    }

    #[test]
    fn test_visual_rows_between_counts_wrapped_rows() {
        let buffer = test_buffer();
        let distance = visual_rows_between(
            VisualPosition::new(0, 1),
            VisualPosition::new(3, 1),
            &buffer,
            4,
            8,
        );
        assert_eq!(distance, 4);
    }

    #[test]
    fn test_visual_cursor_uses_tab_expanded_columns() {
        let buffer = TextBuffer::from_str("a\tb");
        let cursor = visual_cursor(&buffer, 0, 2, 4, true, 8);
        assert_eq!(cursor.position, VisualPosition::new(0, 2));
        assert_eq!(cursor.column, 0);
    }
}
