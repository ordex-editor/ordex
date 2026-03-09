//! Soft-wrap layout helpers.
//!
//! In this module, a **line** means one logical buffer line, while a **row**
//! means one visible on-screen slice of a line after soft wrapping.

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
    column: usize,
    line_len: usize,
    width: usize,
    normal_mode: bool,
    line: usize,
) -> VisualCursor {
    let width = width.max(1);
    let position = if normal_mode || column < line_len || line_len == 0 {
        VisualPosition::new(line, column / width)
    } else {
        // Insert-mode cursors may sit one cell past the last character. When the
        // cursor is exactly at end-of-line, keep it on the final wrapped row
        // instead of inventing an extra empty row.
        VisualPosition::new(line, column.saturating_sub(1) / width)
    };
    let column = column.saturating_sub(row_start_column(position.row, width));
    VisualCursor { position, column }
}

/// Convert a wrapped-row column back to a buffer column.
///
/// `row` is the wrapped-row index inside the logical line, `visual_column` is
/// the desired horizontal position inside that row, and `width` is the number
/// of content columns available on screen.
pub(crate) fn buffer_column_for_visual_column(
    row: usize,
    visual_column: usize,
    line_len: usize,
    width: usize,
    normal_mode: bool,
) -> usize {
    let width = width.max(1);
    let base_column = row_start_column(row, width);
    let max_column = if normal_mode {
        line_len.saturating_sub(1)
    } else {
        line_len
    };

    // Wrapped-row motions preserve the desired on-screen column first, then
    // clamp to the last valid buffer position if the target row is shorter.
    base_column.saturating_add(visual_column).min(max_column)
}

/// Move forward by at most `count` wrapped rows.
///
/// `width` is the number of content columns available for file text.
pub(crate) fn advance_visual_position(
    start: VisualPosition,
    buffer: &TextBuffer,
    width: usize,
    count: usize,
) -> VisualPosition {
    let width = width.max(1);
    let line_rows = wrap_row_count(buffer.line_len(start.line), width);
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
        let rows = wrap_row_count(buffer.line_len(line), width);
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
    let last_row = wrap_row_count(buffer.line_len(last_line), width).saturating_sub(1);
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
        let rows = wrap_row_count(buffer.line_len(line), width);
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
) -> usize {
    if start == end {
        return 0;
    }

    // Count forward only once; reversing the arguments should produce the same
    // distance, so normalize the order before summing rows.
    if end < start {
        return visual_rows_between(end, start, buffer, width);
    }

    let width = width.max(1);
    if start.line == end.line {
        // Rows inside the same logical line differ only by their wrapped-row
        // offsets, so the distance is a simple subtraction.
        return end.row.abs_diff(start.row);
    }

    // First account for the rows remaining in the starting line after `start`.
    let mut rows = wrap_row_count(buffer.line_len(start.line), width).saturating_sub(start.row + 1);
    for line in (start.line + 1)..end.line {
        // Whole lines between the endpoints contribute all of their wrapped rows.
        rows = rows.saturating_add(wrap_row_count(buffer.line_len(line), width));
    }
    // Finally add the rows from the top of the destination line through `end`.
    rows.saturating_add(end.row + 1)
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
        let cursor = visual_cursor(5, 6, 4, true, 0);
        assert_eq!(cursor.position, VisualPosition::new(0, 1));
        assert_eq!(cursor.column, 1);
    }

    #[test]
    fn test_visual_cursor_keeps_insert_eol_on_last_row() {
        let cursor = visual_cursor(4, 4, 4, false, 0);
        assert_eq!(cursor.position, VisualPosition::new(0, 0));
        assert_eq!(cursor.column, 4);
    }

    #[test]
    fn test_buffer_column_for_visual_column_clamps_normal_mode() {
        assert_eq!(buffer_column_for_visual_column(1, 3, 6, 4, true), 5);
    }

    #[test]
    fn test_advance_visual_position_crosses_line_boundaries() {
        let buffer = test_buffer();
        let position = advance_visual_position(VisualPosition::new(0, 1), &buffer, 4, 2);
        assert_eq!(position, VisualPosition::new(2, 0));
    }

    #[test]
    fn test_retreat_visual_position_crosses_line_boundaries() {
        let buffer = test_buffer();
        let position = retreat_visual_position(VisualPosition::new(1, 0), &buffer, 4, 2);
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
        );
        assert_eq!(distance, 4);
    }
}
