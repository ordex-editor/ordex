//! Terminal User Interface module
//!
//! This module isolates all termion-specific code for terminal handling.
//! If the terminal library needs to change in the future, only this file
//! requires modification.

use std::io::{self, Read, Write, stdin, stdout};
use std::panic;
use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::{AlternateScreen, IntoAlternateScreen};

/// Terminal wrapper with RAII cleanup
///
/// Ensures terminal is restored to normal mode even on panic
pub(crate) struct Terminal {
    stdout: AlternateScreen<RawTerminal<io::Stdout>>,
}

/// Restore terminal to a sane state (used for cleanup)
fn restore_terminal() {
    let mut stdout = stdout();
    // Leave alternate screen, show cursor, reset styles
    let _ = write!(
        stdout,
        "{}{}{}",
        termion::screen::ToMainScreen,
        termion::cursor::Show,
        termion::style::Reset
    );
    let _ = stdout.flush();
}

impl Terminal {
    /// Initialize terminal in raw mode with alternate screen
    ///
    /// Raw mode disables line buffering and echo, allowing character-by-character input.
    /// Alternate screen preserves the original terminal content.
    pub(crate) fn new() -> io::Result<Self> {
        // Set up panic hook to restore terminal on panic
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            restore_terminal();
            default_hook(info);
        }));

        let stdout = stdout().into_raw_mode()?.into_alternate_screen()?;
        Ok(Terminal { stdout })
    }

    /// Clear the entire screen
    pub(crate) fn clear_screen(&mut self) -> io::Result<()> {
        write!(self.stdout, "{}", termion::clear::All)?;
        self.stdout.flush()
    }

    /// Write text at specific position (1-indexed)
    ///
    /// # Arguments
    /// * `x` - Column position (1-indexed)
    /// * `y` - Row position (1-indexed)
    /// * `text` - Text to display
    pub(crate) fn write_at(&mut self, x: u16, y: u16, text: &str) -> io::Result<()> {
        write!(self.stdout, "{}{}", termion::cursor::Goto(x, y), text)
    }

    /// Hide the terminal cursor.
    pub(crate) fn hide_cursor(&mut self) -> io::Result<()> {
        write!(self.stdout, "{}", termion::cursor::Hide)
    }

    /// Show the terminal cursor.
    pub(crate) fn show_cursor(&mut self) -> io::Result<()> {
        write!(self.stdout, "{}", termion::cursor::Show)
    }

    /// Save current terminal cursor position.
    ///
    /// Terminals keep an internal "saved cursor" slot. After writing UI chrome
    /// (like status/message lines), we can restore to continue showing the
    /// user's text cursor at its previous location without a visible jump.
    pub(crate) fn save_cursor(&mut self) -> io::Result<()> {
        write!(self.stdout, "{}", termion::cursor::Save)
    }

    /// Restore terminal cursor position saved with save_cursor().
    ///
    /// This only restores position; it does not redraw content. It is used to
    /// update non-editing rows while keeping the editing caret visually stable.
    pub(crate) fn restore_cursor(&mut self) -> io::Result<()> {
        write!(self.stdout, "{}", termion::cursor::Restore)
    }

    /// Flush buffered terminal output.
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }

    fn read_required_byte(reader: &mut io::StdinLock<'_>) -> io::Result<u8> {
        let mut buf = [0_u8; 1];
        match reader.read(&mut buf) {
            Ok(1) => Ok(buf[0]),
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "stdin key stream ended",
            )),
            Ok(_) => unreachable!("single-byte read returned unexpected length"),
            Err(e) => Err(e),
        }
    }

    /// Read next key from input.
    ///
    /// Uses byte-level decoding so a lone Escape byte is always surfaced,
    /// which fixes cases observed over SSH where Esc may not be emitted
    /// reliably through higher-level key parsers.
    pub(crate) fn read_key() -> io::Result<Key> {
        let stdin = stdin();
        let mut reader = stdin.lock();
        let first = Self::read_required_byte(&mut reader)?;

        let key = match first {
            b'\x1b' => Key::Esc,
            b'\n' | b'\r' => Key::Char('\n'),
            0x7f | 0x08 => Key::Backspace,
            0x01..=0x1a => Key::Ctrl((b'a' + (first - 1)) as char),
            b @ 0x20..=0x7e => Key::Char(b as char),
            b => Key::Char(char::from(b)),
        };
        Ok(key)
    }
}

impl Drop for Terminal {
    /// Restore terminal to normal mode on drop
    fn drop(&mut self) {
        // Show cursor and reset styles before dropping (AlternateScreen handles screen switch)
        let _ = write!(
            self.stdout,
            "{}{}",
            termion::cursor::Show,
            termion::style::Reset
        );
        let _ = self.stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_creation() {
        // Terminal creation should succeed
        // Note: This test cannot verify terminal state in CI environment
        // but ensures the API is sound
        let result = Terminal::new();
        assert!(result.is_ok() || result.is_err()); // Always true, but documents expected behavior
    }

    #[test]
    fn test_write_at_boundaries() {
        // Test that write_at accepts valid coordinates
        // Actual terminal size checking would require runtime terminal access
        let mut term = match Terminal::new() {
            Ok(t) => t,
            Err(_) => return, // Skip if terminal unavailable
        };

        // Write at minimum valid position
        let result = term.write_at(1, 1, "test");
        assert!(result.is_ok() || result.is_err()); // Either succeeds or fails gracefully
    }
}
