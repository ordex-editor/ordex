//! Viewport management for scrolling and visible region tracking.
//!
//! The `Viewport` manages which portion of the document is visible on screen
//! and handles scrolling to keep the cursor in view.

use crate::cursor::Cursor;
use crate::display_columns;
use crate::soft_wrap::{self, VisualPosition, line_display_width};
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
    tab_width: usize,
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
            tab_width: 8,
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

    /// Override display tab width used for viewport calculations.
    pub(crate) fn set_tab_width(&mut self, tab_width: usize) {
        self.tab_width = tab_width.max(1);
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
            let max_row = soft_wrap::wrap_row_count(
                line_display_width(buffer, self.first_visible_line, self.tab_width),
                self.width,
            )
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
        let cursor_col = Self::cursor_display_column(cursor, buffer, self.tab_width);
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
        let cursor_visual = soft_wrap::visual_cursor(
            buffer,
            cursor.line(),
            cursor.column(),
            width,
            true,
            self.tab_width,
        );
        let cursor_position = cursor_visual.position;
        let top_position = VisualPosition::new(self.first_visible_line, self.first_visible_row);

        // Wrapped mode never scrolls horizontally, so every visibility update
        // resets the horizontal origin back to the first content column.
        self.first_visible_column = 0;

        // In wrapped mode the viewport origin is a (line, row) pair. The top
        // margin check asks whether the cursor has drifted above the visible
        // row window that begins at `top_position`.
        let top_margin_limit = soft_wrap::advance_visual_position(
            top_position,
            buffer,
            width,
            self.scroll_margin,
            self.tab_width,
        );
        if cursor_position < top_margin_limit {
            // If the cursor moved above the top margin, shift the viewport so the
            // cursor lands `scroll_margin` rows below the new origin.
            self.set_first_visible_position(soft_wrap::retreat_visual_position(
                cursor_position,
                buffer,
                width,
                self.scroll_margin,
                self.tab_width,
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
            self.tab_width,
        );
        // If the viewport already ends on the buffer's final wrapped row, there
        // is no additional content below to satisfy a bottom margin. Enforcing
        // the bottom check here would only pull EOF upward and override a valid
        // user alignment (for example after `zt` near EOF), so we keep origin.
        if last_visible == Self::last_visual_position(buffer, width, self.tab_width) {
            return;
        }
        let bottom_margin_limit = soft_wrap::retreat_visual_position(
            last_visible,
            buffer,
            width,
            self.scroll_margin,
            self.tab_width,
        );
        if cursor_position > bottom_margin_limit {
            // If the cursor moved below the bottom margin, shift the viewport so
            // there are still `scroll_margin` wrapped rows below the cursor.
            self.set_first_visible_position(soft_wrap::retreat_visual_position(
                cursor_position,
                buffer,
                width,
                self.height.saturating_sub(self.scroll_margin + 1),
                self.tab_width,
            ));
        }
    }

    /// Return the final wrapped-row position available in `buffer`.
    fn last_visual_position(buffer: &TextBuffer, width: usize, tab_width: usize) -> VisualPosition {
        let last_line = buffer.lines_count().saturating_sub(1);
        let last_row =
            soft_wrap::wrap_row_count(line_display_width(buffer, last_line, tab_width), width)
                .saturating_sub(1);
        VisualPosition::new(last_line, last_row)
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

    /// Return the inclusive logical-line band currently visible on screen.
    pub(crate) fn line_visible_limits(&self, buffer: &TextBuffer) -> (usize, usize) {
        let last_line = buffer.lines_count().saturating_sub(1);
        let top_line = self.first_visible_line.min(last_line);
        let bottom_line = top_line
            .saturating_add(self.height.saturating_sub(1))
            .min(last_line);
        (top_line, bottom_line)
    }

    /// Return the wrapped-row band currently visible on screen.
    pub(crate) fn wrapped_visible_limits(
        &self,
        buffer: &TextBuffer,
    ) -> (VisualPosition, VisualPosition) {
        let width = self.width.max(1);
        let top_position = VisualPosition::new(self.first_visible_line, self.first_visible_row);

        // Wrapped visibility is expressed in rendered rows, so the lower bound is
        // computed from the top visible row plus `height - 1` rows.
        let bottom_limit = soft_wrap::advance_visual_position(
            top_position,
            buffer,
            width,
            self.height.saturating_sub(1),
            self.tab_width,
        );
        (top_position, bottom_limit)
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
                self.tab_width,
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
        soft_wrap::visual_cursor(
            buffer,
            cursor.line(),
            cursor.column(),
            width,
            true,
            self.tab_width,
        )
        .position
    }

    /// Return the cursor's display column in its current buffer line.
    fn cursor_display_column(cursor: &Cursor, buffer: &TextBuffer, tab_width: usize) -> usize {
        let Some(line_text) = buffer.line_for_display(cursor.line()) else {
            return cursor.column();
        };
        display_columns::buffer_column_to_display_column_chars(
            line_text.chars(),
            cursor.column(),
            tab_width,
        )
    }

    /// Page up by `count` pages using one aggregated cursor adjustment.
    ///
    /// The viewport scrolls up by `(height - 1) * count` rows, then the cursor is placed at
    /// the bottom of the scroll-margin band in the new viewport, keeping it visible with context.
    pub(crate) fn page_up_by(&mut self, cursor: &mut Cursor, buffer: &TextBuffer, count: usize) {
        let page_size = self.height.saturating_sub(1).max(1);
        let scroll_rows = page_size.saturating_mul(count);
        // Use clamped alignment offsets so that an oversized scroll_margin does not invert the
        // top/bottom semantics when the viewport is shorter than 2 * scroll_margin + 1 rows.
        let bottom_row = self.alignment_offsets().bottom;
        if self.soft_wrap {
            let width = self.width.max(1);
            let top = VisualPosition::new(self.first_visible_line, self.first_visible_row);
            // Retreat the viewport by the scroll amount, clamping at the document top.
            let new_top =
                soft_wrap::retreat_visual_position(top, buffer, width, scroll_rows, self.tab_width);
            self.set_first_visible_position(new_top);
            // Place the cursor at the bottom margin of the new viewport.
            let target = soft_wrap::advance_visual_position(
                new_top,
                buffer,
                width,
                bottom_row,
                self.tab_width,
            );
            cursor.move_to_line(buffer, target.line);
        } else {
            // Retreat the viewport origin, clamping at line 0.
            self.first_visible_line = self.first_visible_line.saturating_sub(scroll_rows);
            self.first_visible_row = 0;
            // Place the cursor at the bottom margin of the new viewport.
            let target_line =
                (self.first_visible_line + bottom_row).min(buffer.lines_count().saturating_sub(1));
            cursor.move_to_line(buffer, target_line);
        }
    }

    /// Page down: move viewport and cursor down by `(height - 1)` lines.
    pub(crate) fn page_down(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.page_down_by(cursor, buffer, 1);
    }

    /// Page down by `count` pages using one aggregated cursor adjustment.
    ///
    /// The viewport scrolls down by `(height - 1) * count` rows, then the cursor is placed at
    /// the top of the scroll-margin band in the new viewport, keeping it visible with context.
    pub(crate) fn page_down_by(&mut self, cursor: &mut Cursor, buffer: &TextBuffer, count: usize) {
        let page_size = self.height.saturating_sub(1).max(1);
        let scroll_rows = page_size.saturating_mul(count);
        // Use clamped alignment offsets so that an oversized scroll_margin does not invert the
        // top/bottom semantics when the viewport is shorter than 2 * scroll_margin + 1 rows.
        let top_row = self.alignment_offsets().top;
        if self.soft_wrap {
            let width = self.width.max(1);
            let top = VisualPosition::new(self.first_visible_line, self.first_visible_row);
            let last_vp = Self::last_visual_position(buffer, width, self.tab_width);
            // Advance the viewport by the scroll amount, clamping at the document bottom.
            let new_top =
                soft_wrap::advance_visual_position(top, buffer, width, scroll_rows, self.tab_width)
                    .min(last_vp);
            self.set_first_visible_position(new_top);
            // Place the cursor at the top margin of the new viewport.
            let target =
                soft_wrap::advance_visual_position(new_top, buffer, width, top_row, self.tab_width)
                    .min(last_vp);
            cursor.move_to_line(buffer, target.line);
        } else {
            let max_first_line = buffer.lines_count().saturating_sub(1);
            // Advance the viewport origin, clamping at the last buffer line.
            self.first_visible_line = (self.first_visible_line + scroll_rows).min(max_first_line);
            self.first_visible_row = 0;
            // Place the cursor at the top margin of the new viewport.
            let target_line =
                (self.first_visible_line + top_row).min(buffer.lines_count().saturating_sub(1));
            cursor.move_to_line(buffer, target_line);
        }
    }

    /// Half-page up: move viewport and cursor up by half the viewport height.
    pub(crate) fn half_page_up(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.half_page_up_by(cursor, buffer, 1);
    }

    /// Half-page up by `count` half-pages using one aggregated cursor adjustment.
    ///
    /// The viewport scrolls up by `(height / 2) * count` rows.  The cursor then lands on the
    /// same screen row it occupied before the scroll, clamped to the document top if necessary.
    pub(crate) fn half_page_up_by(
        &mut self,
        cursor: &mut Cursor,
        buffer: &TextBuffer,
        count: usize,
    ) {
        let page_size = (self.height / 2).max(1);
        let scroll_rows = page_size.saturating_mul(count);
        if self.soft_wrap {
            let width = self.width.max(1);
            let top = VisualPosition::new(self.first_visible_line, self.first_visible_row);
            // Measure the cursor's current distance from the viewport top in wrapped rows.
            let cursor_vp = self.cursor_visual_position(cursor, buffer, width);
            let screen_row =
                soft_wrap::visual_rows_between(top, cursor_vp, buffer, width, self.tab_width);
            // Retreat the viewport, clamping at the document top.
            let new_top =
                soft_wrap::retreat_visual_position(top, buffer, width, scroll_rows, self.tab_width);
            self.set_first_visible_position(new_top);
            // Restore the cursor to the same screen row in the new viewport.
            let target = soft_wrap::advance_visual_position(
                new_top,
                buffer,
                width,
                screen_row,
                self.tab_width,
            );
            cursor.move_to_line(buffer, target.line);
        } else {
            // Measure the cursor's current screen row offset from the viewport top.
            let screen_row = cursor.line().saturating_sub(self.first_visible_line);
            // Retreat the viewport, clamping at line 0.
            self.first_visible_line = self.first_visible_line.saturating_sub(scroll_rows);
            self.first_visible_row = 0;
            // Restore the cursor to the same screen row in the new viewport, clamped at line 0.
            let target_line = self.first_visible_line + screen_row;
            cursor.move_to_line(buffer, target_line);
        }
    }

    /// Half-page down: move viewport and cursor down by half the viewport height.
    pub(crate) fn half_page_down(&mut self, cursor: &mut Cursor, buffer: &TextBuffer) {
        self.half_page_down_by(cursor, buffer, 1);
    }

    /// Half-page down by `count` half-pages using one aggregated cursor adjustment.
    ///
    /// The viewport scrolls down by `(height / 2) * count` rows.  The cursor then lands on the
    /// same screen row it occupied before the scroll, clamped to the document bottom if necessary.
    pub(crate) fn half_page_down_by(
        &mut self,
        cursor: &mut Cursor,
        buffer: &TextBuffer,
        count: usize,
    ) {
        let page_size = (self.height / 2).max(1);
        let scroll_rows = page_size.saturating_mul(count);
        if self.soft_wrap {
            let width = self.width.max(1);
            let top = VisualPosition::new(self.first_visible_line, self.first_visible_row);
            let last_vp = Self::last_visual_position(buffer, width, self.tab_width);
            // Measure the cursor's current distance from the viewport top in wrapped rows.
            let cursor_vp = self.cursor_visual_position(cursor, buffer, width);
            let screen_row =
                soft_wrap::visual_rows_between(top, cursor_vp, buffer, width, self.tab_width);
            // Advance the viewport, clamping at the document bottom.
            let new_top =
                soft_wrap::advance_visual_position(top, buffer, width, scroll_rows, self.tab_width)
                    .min(last_vp);
            self.set_first_visible_position(new_top);
            // Restore the cursor to the same screen row in the new viewport, clamped at EOF.
            let target = soft_wrap::advance_visual_position(
                new_top,
                buffer,
                width,
                screen_row,
                self.tab_width,
            )
            .min(last_vp);
            cursor.move_to_line(buffer, target.line);
        } else {
            let max_first_line = buffer.lines_count().saturating_sub(1);
            // Measure the cursor's current screen row offset from the viewport top.
            let screen_row = cursor.line().saturating_sub(self.first_visible_line);
            // Advance the viewport, clamping at the last buffer line.
            self.first_visible_line = (self.first_visible_line + scroll_rows).min(max_first_line);
            self.first_visible_row = 0;
            // Restore the cursor to the same screen row in the new viewport, clamped at EOF.
            let target_line =
                (self.first_visible_line + screen_row).min(buffer.lines_count().saturating_sub(1));
            cursor.move_to_line(buffer, target_line);
        }
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
    /// ctrl-b: cursor lands at the bottom-margin row of the new viewport.
    fn test_page_up_cursor_at_bottom_margin() {
        // height=20, scroll_margin=3, page_size=19
        // viewport starts at line 40, cursor at line 50 (screen row 10).
        // After page_up: viewport moves to line 21, cursor lands at
        // height - 1 - scroll_margin = 16 rows from top → line 21 + 16 = 37.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(50, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(3);
        viewport.first_visible_line = 40;

        viewport.page_up(&mut cursor, &buffer);
        // New viewport top: 40 - 19 = 21
        assert_eq!(viewport.first_visible_line(), 21);
        // Bottom margin row: 21 + (20 - 1 - 3) = 21 + 16 = 37
        assert_eq!(cursor.line(), 37);
    }

    #[test]
    /// ctrl-f: cursor lands at the top-margin row of the new viewport.
    fn test_page_down_cursor_at_top_margin() {
        // height=20, scroll_margin=3, page_size=19
        // viewport starts at line 0, cursor at line 10 (screen row 10).
        // After page_down: viewport moves to line 19, cursor lands at
        // scroll_margin rows from top → line 19 + 3 = 22.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(10, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(3);

        viewport.page_down(&mut cursor, &buffer);
        // New viewport top: 0 + 19 = 19
        assert_eq!(viewport.first_visible_line(), 19);
        // Top margin row: 19 + 3 = 22
        assert_eq!(cursor.line(), 22);
    }

    #[test]
    /// ctrl-b at buffer start: viewport and cursor clamp to line 0.
    fn test_page_up_at_start() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(5, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(3);
        viewport.page_up(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 0);
        // bottom margin: 0 + (20 - 1 - 3) = 16, but buffer only has 99 lines max
        assert_eq!(cursor.line(), 16);
    }

    #[test]
    /// ctrl-f near EOF: cursor clamps to last line.
    fn test_page_down_at_end() {
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(95, 0);

        viewport.set_soft_wrap(false);
        viewport.set_scroll_margin(3);
        // With viewport starting at 0 and page_size=19, viewport scrolls to line 19.
        // cursor → 19 + 3 = 22. Even near EOF the cursor just lands at the margin.
        // Use a viewport start near end to test EOF clamping.
        viewport.first_visible_line = 90;
        viewport.page_down(&mut cursor, &buffer);
        // New viewport: 90 + 19 = 99 (clamped to last buffer line = 99)
        assert_eq!(viewport.first_visible_line(), 99);
        // cursor: 99 + 3 = 102 → clamped to 99
        assert_eq!(cursor.line(), 99);
    }

    #[test]
    /// ctrl-u preserves the cursor's screen row offset from the viewport top.
    fn test_half_page_up_preserves_cursor_screen_row() {
        // height=20, cursor at screen row 5 (line 25, viewport top at 20).
        // scroll_rows = 10. New viewport top = 10. Cursor at 10 + 5 = 15.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(25, 0);

        viewport.set_soft_wrap(false);
        viewport.first_visible_line = 20;

        viewport.half_page_up(&mut cursor, &buffer);
        // New viewport top: 20 - 10 = 10
        assert_eq!(viewport.first_visible_line(), 10);
        // Same screen row (5) preserved: 10 + 5 = 15
        assert_eq!(cursor.line(), 15);
    }

    #[test]
    /// ctrl-d preserves the cursor's screen row offset from the viewport top.
    fn test_half_page_down_preserves_cursor_screen_row() {
        // height=20, cursor at screen row 5 (line 25, viewport top at 20).
        // scroll_rows = 10. New viewport top = 30. Cursor at 30 + 5 = 35.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(25, 0);

        viewport.set_soft_wrap(false);
        viewport.first_visible_line = 20;

        viewport.half_page_down(&mut cursor, &buffer);
        // New viewport top: 20 + 10 = 30
        assert_eq!(viewport.first_visible_line(), 30);
        // Same screen row (5) preserved: 30 + 5 = 35
        assert_eq!(cursor.line(), 35);
    }

    #[test]
    /// ctrl-d always scrolls the viewport even when the cursor is near the top of the screen.
    fn test_half_page_down_always_scrolls_viewport() {
        // Cursor at screen row 0 (top of screen). ctrl-d should still scroll the viewport.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(0, 0);

        viewport.set_soft_wrap(false);
        // Viewport is also at line 0, cursor is at screen row 0.
        viewport.half_page_down(&mut cursor, &buffer);
        // Viewport must have scrolled.
        assert_eq!(viewport.first_visible_line(), 10);
        // Cursor stays at screen row 0: 10 + 0 = 10
        assert_eq!(cursor.line(), 10);
    }

    #[test]
    /// ctrl-u near buffer start: viewport clamps to line 0, cursor at same relative row.
    fn test_half_page_up_near_start_of_file() {
        // Viewport at line 5, cursor at line 8 (screen row 3). scroll_rows = 10.
        // Viewport after scroll: 5 - 10 = 0 (clamped). Cursor: 0 + 3 = 3.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(8, 0);

        viewport.set_soft_wrap(false);
        viewport.first_visible_line = 5;

        viewport.half_page_up(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 0);
        assert_eq!(cursor.line(), 3);
    }

    #[test]
    /// ctrl-d near EOF: viewport scrolls as far as possible, cursor clamps to last line.
    fn test_half_page_down_near_eof() {
        // height=20, viewport at line 85, cursor at line 90 (screen row 5).
        // scroll_rows = 10. New viewport: 95. Cursor: 95 + 5 = 100 → clamped to 99.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(90, 0);

        viewport.set_soft_wrap(false);
        viewport.first_visible_line = 85;

        viewport.half_page_down(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 95);
        assert_eq!(cursor.line(), 99);
    }

    #[test]
    /// ctrl-d with count=2 scrolls by 2 * half_page and preserves screen row.
    fn test_half_page_down_count_preserves_screen_row() {
        // height=20, cursor at screen row 3 (line 23, viewport top 20).
        // count=2, scroll_rows = 20. New viewport: 40. Cursor: 40 + 3 = 43.
        let buffer = create_test_buffer(100);
        let mut viewport = Viewport::new(20);
        let mut cursor = Cursor::new(23, 0);

        viewport.set_soft_wrap(false);
        viewport.first_visible_line = 20;

        viewport.half_page_down_by(&mut cursor, &buffer, 2);
        assert_eq!(viewport.first_visible_line(), 40);
        assert_eq!(cursor.line(), 43);
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
    fn test_horizontal_scroll_uses_tab_expanded_display_columns() {
        let buffer = TextBuffer::from_str("a\tb");
        let mut viewport = Viewport::new(20);
        viewport.set_width(4);
        viewport.set_soft_wrap(false);
        viewport.set_tab_width(8);
        let cursor = Cursor::new(0, 2);

        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert!(viewport.first_visible_column() > 0);
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
    /// Keep wrapped `zt`-style alignment near EOF when no further rows exist below.
    fn test_wrapped_align_top_near_eof_stays_stable_on_visibility_sync() {
        let buffer = create_test_buffer(12);
        let mut viewport = Viewport::new(8);
        let cursor = Cursor::new(11, 0);

        viewport.set_width(40);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);
        viewport.align_cursor_top(&cursor, &buffer);
        let aligned_first_line = viewport.first_visible_line();
        let aligned_first_row = viewport.first_visible_row();

        // When the viewport already reaches EOF, bottom-margin enforcement cannot
        // demand extra rows below the cursor.
        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), aligned_first_line);
        assert_eq!(viewport.first_visible_row(), aligned_first_row);
    }

    #[test]
    /// Keep wrapped `zz`-style alignment near EOF when no further rows exist below.
    fn test_wrapped_align_center_near_eof_stays_stable_on_visibility_sync() {
        let buffer = create_test_buffer(12);
        let mut viewport = Viewport::new(8);
        let cursor = Cursor::new(11, 0);

        viewport.set_width(40);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);
        viewport.align_cursor_center(&cursor, &buffer);
        let aligned_first_line = viewport.first_visible_line();
        let aligned_first_row = viewport.first_visible_row();

        // The center alignment should remain authoritative after a generic sync.
        viewport.ensure_cursor_visible(&cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), aligned_first_line);
        assert_eq!(viewport.first_visible_row(), aligned_first_row);
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

    #[test]
    /// ctrl-u in soft-wrap mode preserves the cursor's visual screen row.
    fn test_half_page_up_wrapped_preserves_visual_row() {
        // Buffer: 20 lines, each shorter than width (no actual wrapping).
        // height=10, width=40. Viewport at line 10, cursor at line 13 (screen row 3).
        // scroll_rows = 5. New viewport top: line 5, row 0.
        // Cursor: 5 + 3 = 8.
        let buffer = create_test_buffer(20);
        let mut viewport = Viewport::new(10);
        let mut cursor = Cursor::new(13, 0);

        viewport.set_width(40);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);
        viewport.first_visible_line = 10;
        viewport.first_visible_row = 0;

        viewport.half_page_up(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 5);
        assert_eq!(cursor.line(), 8);
    }

    #[test]
    /// ctrl-d in soft-wrap mode preserves the cursor's visual screen row.
    fn test_half_page_down_wrapped_preserves_visual_row() {
        // Buffer: 40 lines, each shorter than width (no actual wrapping).
        // height=10, width=40. Viewport at line 10, cursor at line 13 (screen row 3).
        // scroll_rows = 5. New viewport top: line 15.
        // Cursor: 15 + 3 = 18.
        let buffer = create_test_buffer(40);
        let mut viewport = Viewport::new(10);
        let mut cursor = Cursor::new(13, 0);

        viewport.set_width(40);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);
        viewport.first_visible_line = 10;
        viewport.first_visible_row = 0;

        viewport.half_page_down(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 15);
        assert_eq!(cursor.line(), 18);
    }

    #[test]
    /// ctrl-f in soft-wrap mode places cursor at the top-margin row of the new viewport.
    fn test_page_down_wrapped_cursor_at_top_margin() {
        // Buffer: 40 lines, each shorter than width (no actual wrapping).
        // height=10, scroll_margin=1, page_size=9.
        // Viewport at line 0, page_down → viewport top at line 9.
        // Cursor: 9 + 1 = 10.
        let buffer = create_test_buffer(40);
        let mut viewport = Viewport::new(10);
        let mut cursor = Cursor::new(5, 0);

        viewport.set_width(40);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);

        viewport.page_down(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 9);
        assert_eq!(cursor.line(), 10);
    }

    #[test]
    /// ctrl-b in soft-wrap mode places cursor at the bottom-margin row of the new viewport.
    fn test_page_up_wrapped_cursor_at_bottom_margin() {
        // Buffer: 40 lines, each shorter than width (no actual wrapping).
        // height=10, scroll_margin=1, page_size=9, bottom_row = 10 - 1 - 1 = 8.
        // Viewport at line 20, page_up → viewport top at line 11.
        // Cursor: 11 + 8 = 19.
        let buffer = create_test_buffer(40);
        let mut viewport = Viewport::new(10);
        let mut cursor = Cursor::new(25, 0);

        viewport.set_width(40);
        viewport.set_soft_wrap(true);
        viewport.set_scroll_margin(1);
        viewport.first_visible_line = 20;

        viewport.page_up(&mut cursor, &buffer);
        assert_eq!(viewport.first_visible_line(), 11);
        assert_eq!(cursor.line(), 19);
    }
}
