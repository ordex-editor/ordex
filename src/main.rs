//! Ordex - A minimal TUI text viewer
//!
//! This is the main entry point for the ordex text viewer.
//! It handles CLI argument parsing, file loading, terminal initialization,
//! and the main event loop for command input.

// TODO: test on a slow terminal to see how ordex performs.

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

use std::env;
use std::fs;
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
    // T011: Parse CLI arguments
    let args: Vec<String> = env::args().collect();

    // T012: Display usage if no arguments
    if args.len() < 2 {
        print_usage(&args[0]);
        process::exit(0);
    }

    let file_path = &args[1];

    // T013: Check file existence
    if !std::path::Path::new(file_path).exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found: {}", file_path),
        ));
    }

    // T014: Read file into Vec<String>
    let lines = load_file(file_path)?;

    // T023: Initialize terminal and render content
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    // Get terminal size
    let (width, height) = termion::terminal_size()?;

    // Get visible lines for current viewport (offset 0 for now)
    let visible = viewer::get_visible_lines(&lines, 0, height);

    // Render visible lines
    viewer::render(&mut term, visible, width)?;

    // T026: Initialize command mode
    let mut cmd_mode = command::CommandMode::new();

    // Event loop for command input
    loop {
        use termion::event::Key;

        let key = tui::Terminal::read_key()?;

        match key {
            // T026: Enter command mode on ':'
            Key::Char(':') if !cmd_mode.is_active() => {
                cmd_mode.activate();
                cmd_mode.render(&mut term, height)?;
            }
            // Handle backspace in command mode
            Key::Backspace if cmd_mode.is_active() => {
                cmd_mode.pop_char();
                // Clear and re-render command line to handle shrinking text
                term.write_at(1, height, &" ".repeat(width as usize))?;
                cmd_mode.render(&mut term, height)?;
            }
            // T027: Append character to command buffer (exclude control chars)
            Key::Char(c) if cmd_mode.is_active() && c != '\n' && c != '\r' => {
                cmd_mode.push_char(c);
                cmd_mode.render(&mut term, height)?;
            }
            // T030: Cancel command on Escape
            Key::Esc if cmd_mode.is_active() => {
                cmd_mode.cancel();
                // Clear command line
                term.write_at(1, height, &" ".repeat(width as usize))?;
            }
            // T029: Execute command on Enter (handle both \n and \r)
            Key::Char('\n') | Key::Char('\r') if cmd_mode.is_active() => {
                match cmd_mode.execute()? {
                    command::CommandResult::Quit => break, // Exit loop
                    command::CommandResult::Continue => {
                        // Clear command line
                        term.write_at(1, height, &" ".repeat(width as usize))?;
                    }
                    command::CommandResult::Error(msg) => {
                        // T031: Display error message
                        term.write_at(1, height, &format!("Error: {}", msg))?;
                        // Brief pause to show error (will be improved in polish phase)
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        term.write_at(1, height, &" ".repeat(width as usize))?;
                    }
                }
            }
            _ => {} // Ignore other keys for now
        }
    }

    Ok(())
}

/// Display usage message
///
/// Prints usage information to stderr
fn print_usage(program_name: &str) {
    eprintln!("Usage: {} <file>", program_name);
    eprintln!("A minimal TUI text viewer");
}

/// Load file contents into a vector of lines
///
/// # Arguments
/// * `path` - Path to the file to load
///
/// # Returns
/// Vector of strings, one per line in the file
fn load_file(path: &str) -> io::Result<Vec<String>> {
    let content = fs::read_to_string(path)?;
    Ok(content.lines().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempFile;

    #[test]
    fn test_load_file_success() {
        // Create a temporary test file (auto-deleted on drop)
        let file = TempFile::new().unwrap();
        file.writeln("Line 1").unwrap();
        file.writeln("Line 2").unwrap();
        file.writeln("Line 3").unwrap();

        let path = file.path().to_str().unwrap();
        let lines = load_file(path).unwrap();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "Line 1");
        assert_eq!(lines[1], "Line 2");
        assert_eq!(lines[2], "Line 3");
    }

    #[test]
    fn test_load_file_not_found() {
        let result = load_file("/nonexistent/file/path.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_empty_file() {
        let file = TempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let lines = load_file(path).unwrap();
        assert_eq!(lines.len(), 0);
    }
}
