//! Viewport management for scrolling and visible region tracking.
//!
//! The `Viewport` manages which portion of the document is visible on screen
//! and handles scrolling to keep the cursor in view.

use crate::cursor::Cursor;
use crate::soft_wrap::{self, VisualPosition};
use crate::text_buffer::TextBuffer;
#[cfg(test)]
use std::ops::Range;

/// Viewport managing the visible region of the document.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Viewport {
    // `first_visible_line` tracks the logical buffer line at the top of the
    // screen, while `first_visible_row` tracks which wrapped screen row inside
    // that line is visible first when soft wrap is enabled.
    first_visible_line: usize,
    first_visible_row: usize,
    first_visible_column: usize,
    height: usize,
    width: usize,
    scroll_margin: usize,
    horizontal_scroll_margin: usize,
    soft_wrap: bool,
}

/// Effective row offsets for top/center/bottom viewport alignment targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AlignmentOffsets {
    top: usize,
    center: usize,
    bottom: usize,
}

impl Viewport {
    pub(crate) const DEFAULT_SCROLL_MARGIN: usize = 3;
    pub(crate) const DEFAULT_HORIZONTAL_SCROLL_MARGIN: usize = 5;

    /// Create a new viewport with the given height.
    ///
    /// `scroll_margin` defaults to 3 lines, `horizontal_scroll_margin` to 5 columns,
    /// and soft wrapping starts enabled.
    pub(crate) fn new(height: usize) -> Self {
        Self {
            first_visible_line: 0,
            first_visible_row: 0,
            first_visible_column: 0,
            height,
            width: 80,
            scroll_margin: Self::DEFAULT_SCROLL_MARGIN,
            horizontal_scroll_margin: Self::DEFAULT_HORIZONTAL_SCROLL_MARGIN,
            soft_wrap: true,
        }
    }

    /// Set the viewport width.
    pub(crate) fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    /// Return the viewport content width.
    pub(crate) fn width(&self) -> usize {
        self.width
    }

    /// Set the viewport height (content rows only, excluding status rows).
    pub(crate) fn set_height(&mut self, height: usize) {
        self.height = height;
    }

    /// Return the viewport height in content rows.
    pub(crate) fn height(&self) -> usize {
        self.height
    }

    /// Override vertical scroll margin.
    pub(crate) fn set_scroll_margin(&mut self, margin: usize) {
        self.scroll_margin = margin;
    }

    /// Override horizontal scroll margin.
    pub(crate) fn set_horizontal_scroll_margin(&mut self, margin: usize) {
        self.horizontal_scroll_margin = margin;
    }

    /// Enable or disable soft wrapping for viewport visibility calculations.
    pub(crate) fn set_soft_wrap(&mut self, enabled: bool) {
        self.soft_wrap = enabled;
        if enabled {
            // Wrapped mode always starts at the first visible content column. Once
            // a line is split into rows, horizontal scrolling no longer applies.
            self.first_visible_column = 0;
        } else {
            // Unwrapped mode starts each visible line at row 0 because only whole
            // logical lines, not wrapped rows, can be the viewport origin.
            self.first_visible_row = 0;
        }
    }

    /// Return the first visible column (horizontal scroll offset).
    pub(crate) fn first_visible_column(&self) -> usize {
        self.first_visible_column
    }

    /// Return the first visible wrapped-row offset within the top buffer line.
    pub(crate) fn first_visible_row(&self) -> usize {
        self.first_visible_row
    }

    /// Return the range of visible lines `[first, last)`.
    #[cfg(test)]
    pub(crate) fn visible_range(&self) -> Range<usize> {
        self.first_visible_line..self.first_visible_line + self.height
    }

    /// Return the first visible line.
    pub(crate) fn first_visible_line(&self) -> usize {
        self.first_visible_line
    }

    /// Set the first visible line.
    pub(crate) fn set_first_visible_line(&mut self, line: usize) {
        self.first_visible_line = line;
        self.first_visible_row = 0;
    }

    /// Set the top-left visible wrapped-row position.
    fn set_first_visible_position(&mut self, position: VisualPosition) {
        self.first_visible_line = position.line;
        self.first_visible_row = position.row;
    }

    /// Clamp the viewport origin to one wrapped row that exists in `buffer`.
    fn clamp_origin_to_buffer(&mut self, buffer: &TextBuffer) {
        let last_line = buffer.lines_count().saturating_sub(1);
        self.first_visible_line = self.first_visible_line.min(last_line);
        // Wrapped mode may leave the origin inside a stale row after large
        // deletions, so clamp both coordinates before any visibility math.
        if self.soft_wrap {
            let max_row =
                soft_wrap::wrap_row_count(buffer.line_len(self.first_visible_line), self.width)
                    .saturating_sub(1);
            self.first_visible_row = self.first_visible_row.min(max_row);
            self.first_visible_column = 0;
        } else {
            self.first_visible_row = 0;
        }
    }

    /// Ensure the cursor is visible, scrolling if necessary.
    pub(crate) fn ensure_cursor_visible(&mut self, cursor: &Cursor, buffer: &TextBuffer) {
        self.clamp_origin_to_buffer(buffer);
        if self.soft_wrap {
            self.ensure_cursor_visible_wrapped(cursor, buffer);
            return;
        }

        let cursor_line = cursor.line();
        let cursor_col = cursor.column();
        let total_lines = buffer.lines_count();

        // Vertical scrolling remains line-based when wrapping is disabled.
        // Check if we need to scroll up.
        if cursor_line < self.first_visible_line + self.scroll_margin {
            self.first_visible_line = cursor_line.saturating_sub(self.scroll_margin);
        }

        // Check if we need to scroll down.
        let last_visible_line = self.first_visible_line + self.height;
        if cursor_line + self.scroll_margin + 1 > last_visible_line {
            self.first_visible_line = (cursor_line + self.scroll_margin + 1)
                .saturating_sub(self.height)
                .min(total_lines.saturating_sub(self.height));
        }

        // Horizontal scrolling is only active for unwrapped lines. Check if we
        // need to scroll left.
        if cursor_col < self.first_visible_column + self.horizontal_scroll_margin {
            self.first_visible_column = cursor_col.saturating_sub(self.horizontal_scroll_margin);
        }

        // Check if we need to scroll right.
        let last_visible_column = self.first_visible_column + self.width;
        if cursor_col + self.horizontal_scroll_margin + 1 > last_visible_column {
            self.first_visible_column =
                (cursor_col + self.horizontal_scroll_margin + 1).saturating_sub(self.width);
        }
    }

    /// Ensure the cursor is visible when soft wrapping is enabled.
    fn ensure_cursor_visible_wrapped(&mut self, cursor: &Cursor, buffer: &TextBuffer) {
        let width = self.width.max(1);
        let cursor_line_len = buffer.line_len(cursor.line());
        let cursor_visual =
            soft_wrap::visual_cursor(cursor.column(), cursor_line_len, width, true, cursor.line());
        let cursor_position = cursor_visual.position;
        let top_position = VisualPosition::new(self.first_visible_line, self.first_visible_row);

        // Wrapped mode never scrolls horizontally, so every visibility update
        // resets the horizontal origin back to the first content column.
        self.first_visible_column = 0;

        // In wrapped mode the viewport origin is a (line, row) pair. The top
        // margin check asks whether the cursor has drifted above the visible
        // row window that begins at `top_position`.
        let top_margin_limit =
            soft_wrap::advance_visual_position(top_position, buffer, width, self.scroll_margin);
        if cursor_position < top_margin_limit {
            // If the cursor moved above the top margin, shift the viewport so the
            // cursor lands `scroll_margin` rows below the new origin.
            self.set_first_visible_position(soft_wrap::retreat_visual_position(
                cursor_position,
                buffer,
                width,
                self.scroll_margin,
            ));
            return;
        }

        // The bottom margin check mirrors the top one: first find the last
        // visible row, then walk backward by the margin to find the lowest row
        // where the cursor may remain without scrolling.
        let last_visible = soft_wrap::advance_visual_position(
            top_position,
            buffer,
            width,
            self.height.saturating_sub(1),
        );
        let bottom_margin_limit =
            soft_wrap::retreat_visual_position(last_visible, buffer, width, self.scroll_margin);
        if cursor_position > bottom_margin_limit {
            // If the cursor moved below the bottom margin, shift the viewport so
            // there are still `scroll_margin` wrapped rows below the cursor.
            self.set_first_visible_position(soft_wrap::retreat_visual_position(
                cursor_position,
                buffer,
                width,
                self.height.saturating_sub(self.scroll_margin + 1),
            ));
        }
    }

    /// Scroll the viewport up by the specified number of lines.
    pub(crate) fn scroll_up(&mut self, lines: usize) {
        self.first_visible_line = self.first_visible_line.saturating_sub(lines);
        self.first_visible_row = 0;
    }

    /// Scroll the viewport down by the specified number of lines.
    pub(crate) fn scroll_down(&mut self, lines: usize, buffer: &TextBuffer) {
        let max_first_line = buffer.lines_count().saturating_sub(1);
        self.first_visible_line = (self.first_visible_line + lines).min(max_first_line);
        self.first_visible_row = 0;
    }

    /// Return the inclusive line band where the cursor may stay without violating the margin.
    pub(crate) fn line_margin_limits(&self) -> (usize, usize) {
        let offsets = self.alignment_offsets();
        (
            self.first_visible_line.saturating_add(offsets.top),
            self.first_visible_line.saturating_add(offsets.bottom),
        )
    }

    /// Return the wrapped-row band where the cursor may stay without violating the margin.
    pub(crate) fn wrapped_margin_limits(
        &self,
        buffer: &TextBuffer,
    ) -> (VisualPosition, VisualPosition) {
        let width = self.width.max(1);
        let top_position = VisualPosition::new(self.first_visible_line, self.first_visible_row);

        // These bounds intentionally mirror `ensure_cursor_visible_wrapped` so
        // viewport-only scrolls and generic visibility updates share one margin model.
        let top_limit =
            soft_wrap::advance_visual_position(top_position, buffer, width, self.scroll_margin);
        let last_visible = soft_wrap::advance_visual_position(
            top_position,
            buffer,
            width,
            self.height.saturating_sub(1),
        );
        let bottom_limit =
            soft_wrap::retreat_visual_position(last_visible, buffer, width, self.scroll_margin);
        (top_limit, bottom_limit)
    }

    /// Page up: move viewport and cursor up by `(height - 1)` lines.
    pub(crate) fn page_up(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.page_up_by(cursor, buffer, 1);
    }

    /// Align the current cursor row with the top of the scroll-margin-safe band.
    pub(crate) fn align_cursor_top(&mut self, cursor: &Cursor, buffer: &TextBuffer) {
        self.align_cursor_with_offset(cursor, buffer, self.alignment_offsets().top);
    }

    /// Align the current cursor row with the center of the scroll-margin-safe band.
    pub(crate) fn align_cursor_center(&mut self, cursor: &Cursor, buffer: &TextBuffer) {
        self.align_cursor_with_offset(cursor, buffer, self.alignment_offsets().center);
    }

    /// Align the current cursor row with the bottom of the scroll-margin-safe band.
    pub(crate) fn align_cursor_bottom(&mut self, cursor: &Cursor, buffer: &TextBuffer) {
        self.align_cursor_with_offset(cursor, buffer, self.alignment_offsets().bottom);
    }

    /// Align the cursor by placing it `offset` rows below the viewport origin.
    fn align_cursor_with_offset(&mut self, cursor: &Cursor, buffer: &TextBuffer, offset: usize) {
        if self.soft_wrap {
            // Wrapped mode aligns against the cursor's rendered row instead of the
            // whole logical line so `zt/zz/zb` stay consistent with soft wrapping.
            let width = self.width.max(1);
            let cursor_position = self.cursor_visual_position(cursor, buffer, width);
            self.first_visible_column = 0;
            self.set_first_visible_position(soft_wrap::retreat_visual_position(
                cursor_position,
                buffer,
                width,
                offset,
            ));
            return;
        }

        self.first_visible_line = cursor.line().saturating_sub(offset);
        self.first_visible_row = 0;
    }

    /// Compute the effective top/center/bottom offsets inside the visible band.
    fn alignment_offsets(&self) -> AlignmentOffsets {
        if self.height == 0 {
            return AlignmentOffsets {
                top: 0,
                center: 0,
                bottom: 0,
            };
        }

        let top = self.scroll_margin.min(self.height.saturating_sub(1));
        let bottom = self
            .height
            .saturating_sub(self.scroll_margin.saturating_add(1));
        if top > bottom {
            // If the configured margin consumes the whole viewport, collapse every
            // alignment target onto the viewport middle instead of inverting top
            // and bottom semantics.
            let middle = self.height / 2;
            return AlignmentOffsets {
                top: middle,
                center: middle,
                bottom: middle,
            };
        }

        let center = (self.height / 2).clamp(top, bottom);
        AlignmentOffsets {
            top,
            center,
            bottom,
        }
    }

    /// Compute the wrapped visual row occupied by the current cursor.
    fn cursor_visual_position(
        &self,
        cursor: &Cursor,
        buffer: &TextBuffer,
        width: usize,
    ) -> VisualPosition {
        let line_len = buffer.line_len(cursor.line());
        soft_wrap::visual_cursor(cursor.column(), line_len, width, true, cursor.line()).position
    }

    /// Page up by `count` pages using one aggregated cursor adjustment.
    pub(crate) fn page_up_by(&mut self, cursor: &mut Cursor, buffer: &TextBuffer, count: usize) {
        let page_size = self.height.saturating_sub(1).max(1);
        let lines = page_size.saturating_mul(count);
        cursor.move_up_normal_by(buffer, lines);
        self.ensure_cursor_visible(cursor, buffer);
    }

    /// Page down: move viewport and cursor down by `(height - 1)` lines.
    pub(crate) fn page_down(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.page_down_by(cursor, buffer, 1);
    }

    /// Page down by `count` pages using one aggregated cursor adjustment.
    pub(crate) fn page_down_by(&mut self, cursor: &mut Cursor, buffer: &TextBuffer, count: usize) {
        let page_size = self.height.saturating_sub(1).max(1);
        let lines = page_size.saturating_mul(count);
        cursor.move_down_normal_by(buffer, lines);
        self.ensure_cursor_visible(cursor, buffer);
    }

    /// Half-page up: move viewport and cursor up by half the viewport height.
    pub(crate) fn half_page_up(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.half_page_up_by(cursor, buffer, 1);
    }

    /// Half-page up by `count` half-pages using one aggregated cursor adjustment.
    pub(crate) fn half_page_up_by(
        &mut self,
        cursor: &mut Cursor,
        buffer: &TextBuffer,
        count: usize,
    ) {
        let page_size = (self.height / 2).max(1);
        let lines = page_size.saturating_mul(count);
        cursor.move_up_normal_by(buffer, lines);
        self.ensure_cursor_visible(cursor, buffer);
    }

    /// Half-page down: move viewport and cursor down by half the viewport height.
    pub(crate) fn half_page_down(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.half_page_down_by(cursor, buffer, 1);
    }

    /// Half-page down by `count` half-pages using one aggregated cursor adjustment.
    pub(crate) fn half_page_down_by(
        &mut self,
        cursor: &mut Cursor,
        buffer: &TextBuffer,
        count: usize,
    ) {
        let page_size = (self.height / 2).max(1);
        let lines = page_size.saturating_mul(count);
        cursor.move_down_normal_by(buffer, lines);
        self.ensure_cursor_visible(cursor, buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a numbered test buffer.
    fn create_test_buffer(num_lines: usize) -> TextBuffer {
        let lines: Vec<String> = (1..=num_lines).map(|i| format!("Line {}", i)).collect();
        TextBuffer::from_str(&lines.join("\n"))
    }

    #[test]
    fn test_new_viewport() {
        let viewport = Viewport::new(20);
        assert_eq!(viewport.first_visible_line(), 0);
        assert_eq!(viewport.first_visible_row(), 0);
        assert_eq!(viewport.visible_range(), 0..20);
    }

    #[test]
    fn test_set_height_updates_visible_range() {
        let mut viewport = Viewport::new(20);
        viewport.set_height(10);
        assert_eq!(viewport.visible_range(), 0..10);
    }

    #[test]
    fn test_ensure_cursor_visible_no_scroll_needed() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let cursor = Cursor::new(10, 0);

        viewport.set_soft_wrap(false);
        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_ensure_cursor_visible_scroll_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let cursor = Cursor::new(50, 0);

        viewport.set_soft_wrap(false);
        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert!(viewport.first_visible_line() > 0);
    }

    #[test]
    fn test_ensure_cursor_visible_scroll_up() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        viewport.set_soft_wrap(false);
        viewport.first_visible_line = 50;
        let cursor = Cursor::new(45, 0);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert!(viewport.first_visible_line() < 50);
    }

    #[test]
    fn test_scroll_up() {
        let mut viewport = Viewport::new(20);
        viewport.first_visible_line = 10;

        viewport.scroll_up(5);
        assert_eq!(viewport.first_visible_line(), 5);

        viewport.scroll_up(10);
        assert_eq!(viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_scroll_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);

        viewport.scroll_down(10, &buffer);
        assert_eq!(viewport.first_visible_line(), 10);
    }

    #[test]
    fn test_page_up() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(50, 0);

        viewport.set_soft_wrap(false);
        viewport.ensure_cursor_visible(&cursor, &buffer);
        let initial_line = cursor.line();

        viewport.page_up(&mut cursor, &buffer);
        assert!(cursor.line() < initial_line);
        assert!(cursor.line() + 19 <= initial_line + 1);
    }

    #[test]
    fn test_page_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(10, 0);

        viewport.set_soft_wrap(false);
        let initial_line = cursor.line();
        viewport.page_down(&mut cursor, &buffer);

        assert!(cursor.line() > initial_line);
        assert!(cursor.line() >= initial_line + 19);
    }

    #[test]
    fn test_page_up_at_start() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(5, 0);

        viewport.set_soft_wrap(false);
        viewport.page_up(&mut cursor, &buffer);
        assert_eq!(cursor.line(), 0);
        assert_eq!(viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_page_down_at_end() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(95, 0);

        viewport.set_soft_wrap(false);
        viewport.page_down(&mut cursor, &buffer);
        assert_eq!(cursor.line(), 99);
    }

    #[test]
    fn test_half_page_up() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(50, 0);

        viewport.set_soft_wrap(false);
        viewport.half_page_up(&mut cursor, &buffer);
        assert_eq!(cursor.line(), 40);
    }

    #[test]
    fn test_half_page_down() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(10, 0);

        viewport.set_soft_wrap(false);
        viewport.half_page_down(&mut cursor, &buffer);
        assert_eq!(cursor.line(), 20);
    }

    #[test]
    fn test_horizontal_scroll_right() {
        let buffer = TextBuffer::from_str("A very long line that exceeds the viewport width");
        let mut viewport = Viewport::new(20);
        viewport.set_width(20);
        viewport.set_soft_wrap(false);
        let cursor = Cursor::new(0, 40);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert!(viewport.first_visible_column() > 0);
    }

    #[test]
    fn test_horizontal_scroll_left() {
        let buffer = TextBuffer::from_str("A very long line that exceeds the viewport width");
        let mut viewport = Viewport::new(20);
        viewport.set_width(20);
        viewport.set_soft_wrap(false);
        viewport.first_visible_column = 30;
        let cursor = Cursor::new(0, 10);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert!(viewport.first_visible_column() < 30);
    }

    #[test]
    fn test_no_horizontal_scroll_needed() {
        let buffer = TextBuffer::from_str("Short line");
        let mut viewport = Viewport::new(20);
        viewport.set_width(80);
        viewport.set_soft_wrap(false);
        let cursor = Cursor::new(0, 5);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert_eq!(viewport.first_visible_column(), 0);
    }

    #[test]
    fn test_soft_wrap_visibility_tracks_wrapped_rows() {
        let buffer = TextBuffer::from_str("abcdefghijklmnop\nzz");
        let mut viewport = Viewport::new(4);
        let cursor = Cursor::new(0, 12);

        viewport.set_width(4);
        viewport.set_soft_wrap(true);
        viewport.ensure_cursor_visible(&cursor, &buffer);

        assert_eq!(viewport.first_visible_line(), 0);
        assert_eq!(viewport.first_visible_row(), 3);
        assert_eq!(viewport.first_visible_column(), 0);
    }

    #[test]
    fn test_align_cursor_top_respects_scroll_margin_offset() {
        let buffer = create_test_buffer(20);
        let mut viewport = Viewport::new(8);
        let cursor = Cursor::new(8, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(1);
        viewport.align_cursor_top(&cursor, &buffer);

        assert_eq!(viewport.first_visible_line(), 7);
    }

    #[test]
    fn test_align_cursor_center_stays_in_margin_band() {
        let buffer = create_test_buffer(20);
        let mut viewport = Viewport::new(8);
        let cursor = Cursor::new(8, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(1);
        viewport.align_cursor_center(&cursor, &buffer);

        assert_eq!(viewport.first_visible_line(), 4);
    }

    #[test]
    fn test_align_cursor_bottom_clamps_near_file_start() {
        let buffer = create_test_buffer(20);
        let mut viewport = Viewport::new(6);
        let cursor = Cursor::new(2, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(1);
        viewport.align_cursor_bottom(&cursor, &buffer);

        assert_eq!(viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_align_cursor_center_preserves_horizontal_scroll_unwrapped() {
        let buffer = TextBuffer::from_str("A very long line that exceeds the viewport width");
        let mut viewport = Viewport::new(6);
        let cursor = Cursor::new(0, 20);

        viewport.set_width(10);
        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(1);
        viewport.first_visible_column = 7;
        viewport.align_cursor_center(&cursor, &buffer);

        assert_eq!(viewport.first_visible_column(), 7);
    }

    #[test]
    fn test_align_cursor_center_tracks_wrapped_rows_with_margin_band() {
        let buffer = TextBuffer::from_str("abcdefghijklmnop\nzz");
        let mut viewport = Viewport::new(6);
        let cursor = Cursor::new(0, 12);

        viewport.set_width(4);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);
        viewport.align_cursor_center(&cursor, &buffer);

        assert_eq!(viewport.first_visible_line(), 0);
        assert_eq!(viewport.first_visible_row(), 0);
        assert_eq!(viewport.first_visible_column(), 0);
    }
}
