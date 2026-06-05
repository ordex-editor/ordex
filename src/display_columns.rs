//! Helpers for tab-aware display-column calculations.
//!
//! This module centralizes conversion between buffer columns (character indices)
//! and on-screen display columns when tabs expand to the next tab stop.

/// Return the next tab-stop column strictly after `column`.
///
/// `tab_width` must be between 1 and 9999 as enforced by config validation.
pub(crate) fn next_tab_stop(column: usize, tab_width: usize) -> usize {
    let remainder = column % tab_width;
    if remainder == 0 {
        return column + tab_width;
    }
    column + (tab_width - remainder)
}

/// Return the display column reached after rendering one character.
///
/// Tabs advance to the next tab stop and all other characters advance one cell.
pub(crate) fn advance_display_column(column: usize, ch: char, tab_width: usize) -> usize {
    if ch == '\t' {
        return next_tab_stop(column, tab_width);
    }
    column + 1
}

/// Return the full display width of `line` under `tab_width` expansion.
pub(crate) fn line_display_width(line: &str, tab_width: usize) -> usize {
    line_display_width_chars(line.chars(), tab_width)
}

/// Return the full display width of one character iterator.
pub(crate) fn line_display_width_chars(
    chars: impl Iterator<Item = char>,
    tab_width: usize,
) -> usize {
    let mut column = 0;

    // Tabs may consume multiple display cells, so we fold by visual width.
    for ch in chars {
        column = advance_display_column(column, ch, tab_width);
    }
    column
}

/// Return the display column at one buffer-column boundary for character input.
/// `buffer_column` is a character index inside `chars`. Values past end-of-line clamp to the line
/// end.
pub(crate) fn buffer_column_to_display_column_chars(
    chars: impl Iterator<Item = char>,
    buffer_column: usize,
    tab_width: usize,
) -> usize {
    let mut display_column = 0;

    // Stop at the requested buffer boundary or at EOF, whichever comes first.
    for (index, ch) in chars.enumerate() {
        if index >= buffer_column {
            break;
        }
        display_column = advance_display_column(display_column, ch, tab_width);
    }
    display_column
}

/// Return the buffer column containing `display_column`.
///
/// Returns the character index of the character that occupies `display_column`.
/// If `display_column` is past end-of-line, returns the line's character count.
pub(crate) fn display_column_to_buffer_column(
    line: &str,
    display_column: usize,
    tab_width: usize,
) -> usize {
    display_column_to_buffer_column_chars(line.chars(), display_column, tab_width)
}

/// Return the buffer column containing one display column for character input.
pub(crate) fn display_column_to_buffer_column_chars(
    chars: impl Iterator<Item = char>,
    display_column: usize,
    tab_width: usize,
) -> usize {
    let mut current_display = 0;
    let mut buffer_column = 0;

    // Keep the current character index when the target lies inside a tab's
    // expanded cells so every expanded cell maps to the same source column.
    for ch in chars {
        let next_display = advance_display_column(current_display, ch, tab_width);
        if display_column < next_display {
            return buffer_column;
        }
        current_display = next_display;
        buffer_column += 1;
    }

    buffer_column
}

/// Return one visible display window with tabs expanded to spaces.
///
/// `start_display` and `max_display` are display-column counts, not character
/// indices. The returned text has at most `max_display` display cells.
pub(crate) fn expand_display_window(
    line: &str,
    start_display: usize,
    max_display: usize,
    tab_width: usize,
) -> String {
    expand_display_window_chars(line.chars(), start_display, max_display, tab_width)
}

/// Return one visible display window from character input with expanded tabs.
pub(crate) fn expand_display_window_chars(
    chars: impl Iterator<Item = char>,
    start_display: usize,
    max_display: usize,
    tab_width: usize,
) -> String {
    if max_display == 0 {
        return String::new();
    }

    let mut current_display = 0;
    let mut output = String::new();
    let end_display = start_display.saturating_add(max_display);

    // Render each source character only where it overlaps the visible display
    // range, and expand tabs to the exact number of visible space cells.
    for ch in chars {
        let next_display = advance_display_column(current_display, ch, tab_width);
        if next_display <= start_display {
            current_display = next_display;
            continue;
        }
        if current_display >= end_display {
            break;
        }

        let visible_start = current_display.max(start_display);
        let visible_end = next_display.min(end_display);
        let visible_cells = visible_end.saturating_sub(visible_start);

        if ch == '\t' {
            output.push_str(&" ".repeat(visible_cells));
        } else if visible_cells > 0 {
            output.push(ch);
        }

        current_display = next_display;
        if current_display >= end_display {
            break;
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify tab stops advance to the next configured boundary.
    #[test]
    fn next_tab_stop_advances_to_next_boundary() {
        assert_eq!(next_tab_stop(0, 8), 8);
        assert_eq!(next_tab_stop(1, 8), 8);
        assert_eq!(next_tab_stop(8, 8), 16);
        assert_eq!(next_tab_stop(10, 4), 12);
    }

    /// Verify display width treats tabs as multi-cell glyphs.
    #[test]
    fn line_display_width_counts_expanded_tabs() {
        assert_eq!(line_display_width("a\tb", 8), 9);
        assert_eq!(line_display_width("\t\t", 4), 8);
    }

    /// Verify display-column mapping into tab-expanded regions.
    #[test]
    fn display_column_to_buffer_column_maps_inside_tabs() {
        let line = "a\tb";
        assert_eq!(display_column_to_buffer_column(line, 0, 8), 0);
        assert_eq!(display_column_to_buffer_column(line, 1, 8), 1);
        assert_eq!(display_column_to_buffer_column(line, 2, 8), 1);
        assert_eq!(display_column_to_buffer_column(line, 7, 8), 1);
        assert_eq!(display_column_to_buffer_column(line, 8, 8), 2);
    }

    /// Verify display windows expand tabs to spaces and honor clipping.
    #[test]
    fn expand_display_window_expands_tabs_and_clips() {
        assert_eq!(expand_display_window("a\tb", 0, 9, 8), "a       b");
        assert_eq!(expand_display_window("a\tb", 2, 3, 8), "   ");
    }
}
