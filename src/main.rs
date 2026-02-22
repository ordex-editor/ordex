//! Ordex - A minimal TUI text editor
//!
//! This is the main entry point for the ordex text editor.
//! It handles CLI argument parsing, file loading, terminal initialization,
//! and the main event loop.

// TODO: Write the asciidoctor doc for ordex (possibly using Hugo if asciidoctor alone is not
// enough).
// FIXME: the screen flickers.

mod cursor;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod text_buffer;
mod tui;
mod viewport;

use editor_state::EditorState;
use std::env;
use std::io;
use std::process;

/// Entry point for the application
///
/// Delegates to run() and handles errors by printing to stderr
fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

/// Main application logic
///
/// Loads the file, initializes the terminal, and runs the event loop
fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let file_path = args.get(1);

    // Initialize terminal
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    let (width, height) = normalize_terminal_size(termion::terminal_size()?);

    // Initialize editor state with terminal height
    let mut editor = EditorState::new(height as usize);

    if let Some(path) = file_path {
        if std::path::Path::new(path).exists() {
            editor.load_file(path)?;
        } else {
            // New file with specified name
            editor.file_path = std::path::PathBuf::from(path);
        }
    }

    // Main event loop
    loop {
        // Render current view
        render_editor(&mut term, &mut editor, width, height)?;

        // Clear status message after displaying
        editor.status_message = None;

        // Read and handle input
        let key = tui::Terminal::read_key()?;
        editor.handle_key(key);

        if editor.should_quit {
            break;
        }
    }

    Ok(())
}

/// Normalize terminal size to avoid underflow in rendering math.
///
/// PTY backends may report 0x0 before size is explicitly set. We clamp to a
/// minimally usable size to keep rendering deterministic.
fn normalize_terminal_size((width, height): (u16, u16)) -> (u16, u16) {
    // Height reserves 2 lines for status + message rows.
    (width.max(1), height.max(3))
}

/// Render the editor state to the terminal
fn render_editor(
    term: &mut tui::Terminal,
    editor: &mut EditorState,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let (width, height) = normalize_terminal_size((width, height));
    term.hide_cursor()?;

    // Reserve bottom 2 lines for status bar and command/message line
    let content_height = height.saturating_sub(2) as usize;

    // Update viewport width
    editor.viewport.set_width(width as usize);

    // Render visible lines from the buffer
    let first_line = editor.viewport.first_visible_line();
    let first_col = editor.viewport.first_visible_column();
    for row in 0..content_height {
        let line_idx = first_line + row;
        let y = (row + 1) as u16;

        // Clear line first
        term.write_at(1, y, &" ".repeat(width as usize))?;

        if let Some(line) = editor.buffer.line_for_display(line_idx) {
            // Render display-safe line content (no trailing CR/LF), then apply horizontal scroll.
            let line_str: String = line.chars().skip(first_col).take(width as usize).collect();
            term.write_at(1, y, &line_str)?;
        }
    }

    // Render status bar (second to last line)
    let status_y = height - 1;
    let mode_str = editor.mode_name();
    let pos_str = format!(
        "{}:{} ",
        editor.cursor.line() + 1,
        editor.cursor.column() + 1
    );
    let modified = if editor.buffer.is_modified() {
        "[+] "
    } else {
        ""
    };
    let file_name = editor
        .file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("[No Name]");

    let status_left = format!(" {} | {}{}", mode_str, modified, file_name);
    let status_right = pos_str;
    let padding = width.saturating_sub((status_left.len() + status_right.len()) as u16) as usize;
    let status_line = format!("{}{:padding$}{}", status_left, "", status_right);

    // Invert colors for status bar
    term.write_at(
        1,
        status_y,
        &format!(
            "{}{}{}",
            termion::style::Invert,
            &status_line[..status_line.len().min(width as usize)],
            termion::style::Reset
        ),
    )?;

    // Render command/message line (last line)
    let msg_y = height;
    term.write_at(1, msg_y, &" ".repeat(width as usize))?;

    if let (Some(prompt), Some(input)) = (editor.input_prompt(), editor.input_line()) {
        term.write_at(1, msg_y, &format!("{}{}", prompt, input))?;
    } else if let Some(ref msg) = editor.status_message {
        term.write_at(1, msg_y, msg)?;
    }

    // Position cursor (accounting for scroll offsets)
    let cursor_x = (editor.cursor.column() - editor.viewport.first_visible_column() + 1) as u16;
    let cursor_y = (editor.cursor.line() - editor.viewport.first_visible_line() + 1) as u16;
    term.write_at(cursor_x, cursor_y, "")?;
    term.show_cursor()?;
    term.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_terminal_size_clamps_zero() {
        assert_eq!(normalize_terminal_size((0, 0)), (1, 3));
    }

    #[test]
    fn test_normalize_terminal_size_preserves_valid_dimensions() {
        assert_eq!(normalize_terminal_size((120, 40)), (120, 40));
    }

    #[test]
    fn test_normalize_terminal_size_clamps_small_height() {
        assert_eq!(normalize_terminal_size((80, 1)), (80, 3));
    }
}
