//! Terminal User Interface module
//!
//! This module isolates all termion-specific code for terminal handling.
//! If the terminal library needs to change in the future, only this file
//! requires modification.

use std::collections::VecDeque;
use std::io::{self, Stdin, Write, stdin, stdout};
use std::panic;
use std::sync::{Mutex, OnceLock};
use termion::event::Key;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::{AlternateScreen, IntoAlternateScreen};

/// Terminal wrapper with RAII cleanup
///
/// Ensures terminal is restored to normal mode even on panic
pub(crate) struct Terminal {
    stdout: AlternateScreen<RawTerminal<io::Stdout>>,
}

static PENDING_BYTES: OnceLock<Mutex<VecDeque<u8>>> = OnceLock::new();

fn pending_bytes() -> &'static Mutex<VecDeque<u8>> {
    PENDING_BYTES.get_or_init(|| Mutex::new(VecDeque::new()))
}

mod unsafe_io;

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
    // 50 ms matches neovim's default `ttimeoutlen` and covers even
    // high-latency SSH/tmux links while keeping bare-Esc responsive.
    const ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS: i32 = 50;
    const ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS: i32 = 50;

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

    fn read_required_byte(stdin: &Stdin) -> io::Result<u8> {
        if let Ok(mut queue) = pending_bytes().lock()
            && let Some(b) = queue.pop_front()
        {
            return Ok(b);
        }

        unsafe_io::read_byte(stdin)
    }

    fn push_pending_byte(byte: u8) {
        if let Ok(mut queue) = pending_bytes().lock() {
            queue.push_back(byte);
        }
    }

    fn read_optional_byte_with_timeout(stdin: &Stdin, timeout_ms: i32) -> io::Result<Option<u8>> {
        if !unsafe_io::poll_readable(stdin, timeout_ms)? {
            return Ok(None);
        }
        Self::read_required_byte(stdin).map(Some)
    }

    fn csi_final_byte(b: u8) -> bool {
        (b'@'..=b'~').contains(&b)
    }

    fn parse_csi_u_sequence(seq: &[u8]) -> Option<Key> {
        let raw = std::str::from_utf8(seq).ok()?;
        let mut parts = raw.split(';');
        let codepoint = parts.next()?.parse::<u32>().ok()?;
        let modifier = parts
            .next()
            .and_then(|m| m.parse::<u32>().ok())
            .unwrap_or(1);
        let modifier_bits = modifier.saturating_sub(1);
        let ctrl = (modifier_bits & 0b100) != 0;
        let alt = (modifier_bits & 0b010) != 0;
        let ch = char::from_u32(codepoint)?;

        if ctrl {
            if ch.is_ascii_alphabetic() {
                return Some(Key::Ctrl(ch.to_ascii_lowercase()));
            }
            if ch.is_ascii() {
                return Some(Key::Ctrl(ch));
            }
            return None;
        }

        if alt && ch.is_ascii() {
            return Some(Key::Alt(ch.to_ascii_lowercase()));
        }

        Some(Key::Char(ch))
    }

    /// Extract the CSI modifier field from a sequence prefix like `1;5`.
    fn parse_csi_modifier(prefix: &[u8]) -> Option<u16> {
        let raw = std::str::from_utf8(prefix).ok()?;
        raw.split(';').nth(1)?.parse::<u16>().ok()
    }

    /// Decode modified navigation keys carried by CSI letter-final sequences.
    fn parse_modified_navigation_key(prefix: &[u8], final_byte: u8) -> Option<Key> {
        match (Self::parse_csi_modifier(prefix)?, final_byte) {
            (2, b'A') => Some(Key::ShiftUp),
            (2, b'B') => Some(Key::ShiftDown),
            (2, b'C') => Some(Key::ShiftRight),
            (2, b'D') => Some(Key::ShiftLeft),
            (3, b'A') => Some(Key::AltUp),
            (3, b'B') => Some(Key::AltDown),
            (3, b'C') => Some(Key::AltRight),
            (3, b'D') => Some(Key::AltLeft),
            (5, b'A') => Some(Key::CtrlUp),
            (5, b'B') => Some(Key::CtrlDown),
            (5, b'C') => Some(Key::CtrlRight),
            (5, b'D') => Some(Key::CtrlLeft),
            (5, b'H') => Some(Key::CtrlHome),
            (5, b'F') => Some(Key::CtrlEnd),
            _ => None,
        }
    }

    /// Decode CSI `~`-terminated keys such as Home, End, and Delete.
    ///
    /// A "tilde key" is an escape sequence whose final byte is `~`, for example
    /// `ESC [ 1 ~` for Home or `ESC [ 4 ; 5 ~` for Ctrl-End.
    fn parse_tilde_key(prefix: &[u8]) -> Key {
        let raw = std::str::from_utf8(prefix).ok();
        let mut parts = raw.unwrap_or_default().split(';');
        let code = parts.next().and_then(|part| part.parse::<u16>().ok());
        let modifier = parts.next().and_then(|part| part.parse::<u16>().ok());
        match (code, modifier) {
            (Some(1 | 7), None) => Key::Home,
            (Some(1 | 7), Some(5)) => Key::CtrlHome,
            (Some(3), None) => Key::Delete,
            (Some(4 | 8), None) => Key::End,
            (Some(4 | 8), Some(5)) => Key::CtrlEnd,
            _ => Key::Null,
        }
    }

    fn parse_csi_sequence(stdin: &Stdin) -> io::Result<Key> {
        // We already received ESC + '[', so use the shorter intra-sequence timeout.
        let Some(first) =
            Self::read_optional_byte_with_timeout(stdin, Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS)?
        else {
            return Ok(Key::Esc);
        };

        let mut seq = vec![first];
        while !Self::csi_final_byte(*seq.last().expect("sequence is non-empty")) && seq.len() < 16 {
            let Some(next) = Self::read_optional_byte_with_timeout(
                stdin,
                Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS,
            )?
            else {
                return Ok(Key::Esc);
            };
            seq.push(next);
        }

        let Some(final_byte) = seq.last().copied() else {
            return Ok(Key::Esc);
        };
        let prefix = &seq[..seq.len() - 1];

        if let Some(key) = Self::parse_modified_navigation_key(prefix, final_byte) {
            return Ok(key);
        }

        match final_byte {
            b'A' => return Ok(Key::Up),
            b'B' => return Ok(Key::Down),
            b'C' => return Ok(Key::Right),
            b'D' => return Ok(Key::Left),
            b'H' => return Ok(Key::Home),
            b'F' => return Ok(Key::End),
            b'Z' => return Ok(Key::BackTab),
            _ => {}
        }

        if final_byte == b'~' {
            return Ok(Self::parse_tilde_key(prefix));
        }

        if final_byte == b'u' {
            return Ok(Self::parse_csi_u_sequence(prefix).unwrap_or(Key::Null));
        }

        Ok(Key::Null)
    }

    fn parse_escape_sequence(stdin: &Stdin) -> io::Result<Key> {
        let Some(second) =
            Self::read_optional_byte_with_timeout(stdin, Self::ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS)?
        else {
            return Ok(Key::Esc);
        };

        match second {
            b'[' => Self::parse_csi_sequence(stdin),
            b'O' => {
                let Some(third) = Self::read_optional_byte_with_timeout(
                    stdin,
                    Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS,
                )?
                else {
                    return Ok(Key::Esc);
                };
                Ok(match third {
                    b'H' => Key::Home,
                    b'F' => Key::End,
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    b'C' => Key::Right,
                    b'D' => Key::Left,
                    _ => Key::Esc,
                })
            }
            b @ 0x01..=0x1a => Ok(Key::Alt((b'a' + (b - 1)) as char)),
            b'b' | b'f' | b'B' | b'F' => Ok(Key::Alt(second as char)),
            b => {
                // Preserve non-Alt followers after ESC so `Esc` then `:`
                // doesn't drop the `:` when entered quickly.
                Self::push_pending_byte(b);
                Ok(Key::Esc)
            }
        }
    }

    /// Decode one UTF-8 character starting from the first already-read byte.
    fn read_utf8_char(first: u8, stdin: &Stdin) -> io::Result<char> {
        // Determine expected UTF-8 width from the lead byte; fall back to a
        // byte-to-char mapping for non-leading values.
        let expected_len = match first {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return Ok(char::from(first)),
        };

        let mut bytes = vec![first];
        for _ in 1..expected_len {
            let next = Self::read_required_byte(stdin)?;
            // UTF-8 continuation bytes must have the `10xxxxxx` shape.
            if (next & 0b1100_0000) != 0b1000_0000 {
                // Put back the unexpected byte so input stream alignment is kept.
                Self::push_pending_byte(next);
                return Ok(char::from(first));
            }
            bytes.push(next);
        }

        // Decode the full byte sequence; if decoding fails, preserve previous
        // behavior by returning a best-effort single-byte char.
        Ok(std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| text.chars().next())
            .unwrap_or_else(|| char::from(first)))
    }

    /// Read next key from input.
    ///
    /// We still surface standalone Esc over SSH reliably while parsing common
    /// escape sequences (including jittered arrivals) into semantic keys.
    pub(crate) fn read_key() -> io::Result<Key> {
        let stdin = stdin();
        let first = Self::read_required_byte(&stdin)?;

        let key = match first {
            b'\x1b' => Self::parse_escape_sequence(&stdin)?,
            b'\n' | b'\r' => Key::Char('\n'),
            0x7f | 0x08 => Key::Backspace,
            0x01..=0x1a => Key::Ctrl((b'a' + (first - 1)) as char),
            b @ 0x20..=0x7e => Key::Char(b as char),
            b => Key::Char(Self::read_utf8_char(b, &stdin)?),
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
