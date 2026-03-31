//! Terminal input parsing and key decoding.

use super::Terminal;
use super::unsafe_io;
use std::collections::VecDeque;
use std::io::{self, Stdin, stdin};
use std::sync::{Mutex, OnceLock};
use termion::event::Key;

static PENDING_BYTES: OnceLock<Mutex<VecDeque<u8>>> = OnceLock::new();

/// Return the shared queue for bytes that were read ahead during parsing.
fn pending_bytes() -> &'static Mutex<VecDeque<u8>> {
    PENDING_BYTES.get_or_init(|| Mutex::new(VecDeque::new()))
}

impl Terminal {
    // 50 ms matches neovim's default `ttimeoutlen` and covers even
    // high-latency SSH/tmux links while keeping bare-Esc responsive.
    const ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS: i32 = 50;
    const ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS: i32 = 50;

    /// Read one byte from stdin or from the pending byte queue.
    fn read_required_byte(stdin: &Stdin) -> io::Result<u8> {
        if let Ok(mut queue) = pending_bytes().lock()
            && let Some(byte) = queue.pop_front()
        {
            return Ok(byte);
        }

        unsafe_io::read_byte(stdin)
    }

    /// Push one lookahead byte back into the pending queue.
    fn push_pending_byte(byte: u8) {
        if let Ok(mut queue) = pending_bytes().lock() {
            queue.push_back(byte);
        }
    }

    /// Read an optional byte after waiting up to the requested timeout.
    fn read_optional_byte_with_timeout(stdin: &Stdin, timeout_ms: i32) -> io::Result<Option<u8>> {
        if !unsafe_io::poll_readable(stdin, timeout_ms)? {
            return Ok(None);
        }
        Self::read_required_byte(stdin).map(Some)
    }

    /// Return whether the byte can terminate a CSI escape sequence.
    fn csi_final_byte(byte: u8) -> bool {
        (b'@'..=b'~').contains(&byte)
    }

    /// Decode a CSI `u` key sequence into the closest termion key variant.
    fn parse_csi_u_sequence(seq: &[u8]) -> Option<Key> {
        // CSI `u` sequences carry a Unicode codepoint and an optional modifier field.
        let raw = std::str::from_utf8(seq).ok()?;
        let mut parts = raw.split(';');
        let codepoint = parts.next()?.parse::<u32>().ok()?;
        let modifier = parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1);
        let modifier_bits = modifier.saturating_sub(1);
        let ctrl = (modifier_bits & 0b100) != 0;
        let alt = (modifier_bits & 0b010) != 0;
        let ch = char::from_u32(codepoint)?;

        // Match the keybinding layer's ASCII-oriented modifiers before falling back
        // to a plain character for terminals that use CSI `u` for ordinary input.
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
        // Xterm-style CSI modifiers use `2` for Shift, `3` for Alt, and `5` for Ctrl.
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
        // Tilde sequences use the first field for the key code and the second for modifiers.
        let raw = std::str::from_utf8(prefix).ok();
        let mut parts = raw.unwrap_or_default().split(';');
        let code = parts.next().and_then(|part| part.parse::<u16>().ok());
        let modifier = parts.next().and_then(|part| part.parse::<u16>().ok());
        match (code, modifier) {
            (Some(1 | 7), None) => Key::Home,
            (Some(1 | 7), Some(5)) => Key::CtrlHome,
            (Some(3), None) => Key::Delete,
            (Some(5), None | Some(5)) => Key::PageUp,
            (Some(6), None | Some(5)) => Key::PageDown,
            (Some(4 | 8), None) => Key::End,
            (Some(4 | 8), Some(5)) => Key::CtrlEnd,
            _ => Key::Null,
        }
    }

    /// Parse a CSI escape sequence that starts with `ESC [`.
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

    /// Parse an escape sequence that starts with `ESC`.
    fn parse_escape_sequence(stdin: &Stdin) -> io::Result<Key> {
        let Some(second) =
            Self::read_optional_byte_with_timeout(stdin, Self::ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS)?
        else {
            return Ok(Key::Esc);
        };

        match second {
            b'[' => Self::parse_csi_sequence(stdin),
            b'O' => {
                // SS3 sequences carry Home/End and arrow keys in some terminal modes.
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
            byte => {
                // Preserve non-Alt followers after ESC so `Esc` then `:` keeps the `:`.
                Self::push_pending_byte(byte);
                Ok(Key::Esc)
            }
        }
    }

    /// Decode one UTF-8 character starting from the first already-read byte.
    fn read_utf8_char(first: u8, stdin: &Stdin) -> io::Result<char> {
        // Determine expected UTF-8 width from the lead byte; non-leading values
        // fall back to a direct byte-to-char mapping.
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
                // Put back the unexpected byte so input stream alignment is preserved.
                Self::push_pending_byte(next);
                return Ok(char::from(first));
            }
            bytes.push(next);
        }

        // Decode the full sequence and fall back to a direct byte mapping when
        // the collected bytes do not form valid UTF-8.
        Ok(std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| text.chars().next())
            .unwrap_or_else(|| char::from(first)))
    }

    /// Read the next key from terminal input.
    ///
    /// Standalone `Esc` stays responsive while common escape sequences decode
    /// into semantic navigation and editing keys, including jittered arrivals.
    pub(crate) fn read_key() -> io::Result<Key> {
        let stdin = stdin();
        let first = Self::read_required_byte(&stdin)?;

        // Interpret ASCII control bytes directly before deferring multibyte input
        // to the UTF-8 decoder.
        let key = match first {
            b'\x1b' => Self::parse_escape_sequence(&stdin)?,
            b'\n' | b'\r' => Key::Char('\n'),
            0x7f | 0x08 => Key::Backspace,
            0x01..=0x1a => Key::Ctrl((b'a' + (first - 1)) as char),
            b @ 0x20..=0x7e => Key::Char(b as char),
            byte => Key::Char(Self::read_utf8_char(byte, &stdin)?),
        };
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that CSI `u` modifiers decode into ASCII control keys.
    #[test]
    fn test_parse_csi_u_sequence_decodes_ctrl_ascii() {
        assert_eq!(
            Terminal::parse_csi_u_sequence(b"65;5"),
            Some(Key::Ctrl('a'))
        );
    }

    /// Verify that modified CSI navigation keys map to control navigation variants.
    #[test]
    fn test_parse_modified_navigation_key_decodes_ctrl_home_and_end() {
        assert_eq!(
            Terminal::parse_modified_navigation_key(b"1;5", b'H'),
            Some(Key::CtrlHome)
        );
        assert_eq!(
            Terminal::parse_modified_navigation_key(b"1;5", b'F'),
            Some(Key::CtrlEnd)
        );
    }

    /// Verify that tilde-terminated CSI sequences decode delete and Ctrl-End.
    #[test]
    fn test_parse_tilde_key_decodes_delete_and_ctrl_end() {
        assert_eq!(Terminal::parse_tilde_key(b"3"), Key::Delete);
        assert_eq!(Terminal::parse_tilde_key(b"4;5"), Key::CtrlEnd);
    }

    /// Verify that tilde-terminated CSI sequences decode page-navigation keys.
    #[test]
    fn test_parse_tilde_key_decodes_page_up_and_page_down() {
        assert_eq!(Terminal::parse_tilde_key(b"5"), Key::PageUp);
        assert_eq!(Terminal::parse_tilde_key(b"6"), Key::PageDown);
        assert_eq!(Terminal::parse_tilde_key(b"5;5"), Key::PageUp);
        assert_eq!(Terminal::parse_tilde_key(b"6;5"), Key::PageDown);
    }
}
