//! Ordex - A minimal TUI text editor
//!
//! This is the main entry point for the ordex text editor.
//! It handles CLI argument parsing, file loading, terminal initialization,
//! and the main event loop.

mod command;
mod cursor;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod text_buffer;
mod tui;
mod viewer;
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

    if args.len() < 2 {
        print_usage(&args[0]);
        process::exit(0);
    }

    let file_path = &args[1];

    if !std::path::Path::new(file_path).exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found: {}", file_path),
        ));
    }

    // Initialize terminal
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    let (width, height) = termion::terminal_size()?;

    // Initialize editor state with terminal height
    let mut editor = EditorState::new(height as usize);
    editor.load_file(file_path)?;

    // Main event loop
    loop {
        // Render current view
        render_editor(&mut term, &mut editor, width, height)?;

        // Read and handle input
        let key = tui::Terminal::read_key()?;
        editor.handle_key(key);

        if editor.should_quit {
            break;
        }

        // Clear status message after displaying once
        editor.status_message = None;
    }

    Ok(())
}

/// Render the editor state to the terminal
fn render_editor(
    term: &mut tui::Terminal,
    editor: &mut EditorState,
    width: u16,
    height: u16,
) -> io::Result<()> {
    // Reserve bottom 2 lines for status bar and command/message line
    let content_height = height.saturating_sub(2) as usize;

    // Render visible lines from the buffer
    let first_line = editor.viewport.first_visible_line();
    for row in 0..content_height {
        let line_idx = first_line + row;
        let y = (row + 1) as u16;

        // Clear line first
        term.write_at(1, y, &" ".repeat(width as usize))?;

        if let Some(line) = editor.buffer.line(line_idx) {
            let line_str: String = line.chars().take(width as usize).collect();
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
    let padding = width as usize - status_left.len() - status_right.len();
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

    // Position cursor
    let cursor_x = (editor.cursor.column() + 1) as u16;
    let cursor_y = (editor.cursor.line() - editor.viewport.first_visible_line() + 1) as u16;
    term.write_at(cursor_x, cursor_y, "")?;

    Ok(())
}

/// Display usage message
fn print_usage(program_name: &str) {
    eprintln!("Usage: {} <file>", program_name);
    eprintln!("A minimal TUI text editor");
}
