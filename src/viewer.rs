//! File content viewer module
//!
//! Handles rendering file content to the terminal with viewport management

use std::io;

/// Calculate visible lines based on terminal height
///
/// # Arguments
/// * `lines` - All file lines
/// * `offset` - Starting line index (0-based)
/// * `terminal_height` - Total terminal height in rows
///
/// # Returns
/// Slice of lines that fit in the viewport (reserves bottom line for commands)
pub fn get_visible_lines(lines: &[String], offset: usize, terminal_height: u16) -> &[String] {
    // Reserve bottom line for command input
    let viewport_height = if terminal_height > 1 {
        (terminal_height - 1) as usize
    } else {
        0
    };

    let start = offset.min(lines.len());
    let end = (start + viewport_height).min(lines.len());

    &lines[start..end]
}

/// Render file content to terminal
///
/// # Arguments
/// * `term` - Terminal instance
/// * `lines` - Lines to display
/// * `terminal_width` - Terminal width for truncation
pub fn render(
    term: &mut crate::tui::Terminal,
    lines: &[String],
    terminal_width: u16
) -> io::Result<()> {
    let width = terminal_width as usize;

    for (idx, line) in lines.iter().enumerate() {
        let row = (idx + 1) as u16;

        // Truncate line if it exceeds terminal width
        let display_line = if line.len() > width {
            &line[..width]
        } else {
            line
        };

        term.write_at(1, row, display_line)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_visible_lines_full_viewport() {
        let lines = vec![
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
            "Line 4".to_string(),
            "Line 5".to_string(),
        ];

        // Terminal height 4 = 3 visible lines (reserve 1 for command)
        let visible = get_visible_lines(&lines, 0, 4);
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0], "Line 1");
        assert_eq!(visible[2], "Line 3");
    }

    #[test]
    fn test_get_visible_lines_with_offset() {
        let lines = vec![
            "Line 1".to_string(),
            "Line 2".to_string(),
            "Line 3".to_string(),
            "Line 4".to_string(),
            "Line 5".to_string(),
        ];

        // Start from line 2 (0-based index 1)
        let visible = get_visible_lines(&lines, 1, 4);
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0], "Line 2");
        assert_eq!(visible[2], "Line 4");
    }

    #[test]
    fn test_get_visible_lines_exceeds_content() {
        let lines = vec![
            "Line 1".to_string(),
            "Line 2".to_string(),
        ];

        // Terminal can show more lines than available
        let visible = get_visible_lines(&lines, 0, 10);
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn test_get_visible_lines_offset_beyond_content() {
        let lines = vec![
            "Line 1".to_string(),
            "Line 2".to_string(),
        ];

        // Offset beyond file length returns empty slice
        let visible = get_visible_lines(&lines, 10, 10);
        assert_eq!(visible.len(), 0);
    }

    #[test]
    fn test_get_visible_lines_minimal_terminal() {
        let lines = vec!["Line 1".to_string()];

        // Terminal height 1 = 0 visible lines (all reserved for command)
        let visible = get_visible_lines(&lines, 0, 1);
        assert_eq!(visible.len(), 0);
    }
}
