mod tui;
mod viewer;
mod command;

use std::env;
use std::fs;
use std::io;
use std::process;
use termion;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

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
            format!("File not found: {}", file_path)
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

    // TODO: Event loop for command input will be added in Phase 5

    Ok(())
}

/// Display usage message
fn print_usage(program_name: &str) {
    eprintln!("Usage: {} <file>", program_name);
    eprintln!("A minimal TUI text viewer");
}

/// Load file contents into a vector of lines
fn load_file(path: &str) -> io::Result<Vec<String>> {
    let content = fs::read_to_string(path)?;
    Ok(content.lines().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_file_success() {
        // Create a temporary test file
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "Line 1").unwrap();
        writeln!(file, "Line 2").unwrap();
        writeln!(file, "Line 3").unwrap();

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
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let lines = load_file(path).unwrap();
        assert_eq!(lines.len(), 0);
    }
}
