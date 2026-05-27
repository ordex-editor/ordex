//! Terminal input parsing and key decoding.

use super::Terminal;
use super::unsafe_io;
use crate::unsafe_io::poll_fd;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Stdin, stdin};
use std::time::Duration;
use termion::event::Key;

thread_local! {
    /// Store same-thread lookahead bytes that were read while decoding one key
    /// sequence but belong to the next input event.
    ///
    /// Escape-sequence parsing and UTF-8 fallback sometimes need to "unread" one
    /// byte so the next `read_input_event*` call can consume it. The application
    /// loop reads terminal input on one thread (`src/app.rs`), so keeping this
    /// queue thread-local preserves the intended behavior without sharing parser
    /// state across unrelated threads or tests.
    static PENDING_BYTES: RefCell<VecDeque<u8>> = const { RefCell::new(VecDeque::new()) };
}

/// One normalized terminal input unit routed through the app event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InputEvent {
    Key(Key),
    Paste(String),
}

/// Return whether the current thread already has deferred lookahead bytes.
///
/// Timed reads check this before polling stdin so bytes that were pushed back by
/// the parser stay ordered ahead of any newly available terminal input.
fn pending_queue_has_bytes() -> bool {
    PENDING_BYTES.with(|queue| !queue.borrow().is_empty())
}

/// Pop one previously deferred lookahead byte for the current thread.
///
/// This keeps multi-byte sequence parsing and later top-level input reads in
/// sync when the parser had to hand one byte back to itself.
fn pop_pending_byte() -> Option<u8> {
    PENDING_BYTES.with(|queue| queue.borrow_mut().pop_front())
}

impl Terminal {
    // 50 ms matches neovim's default `ttimeoutlen` and covers even
    // high-latency SSH/tmux links while keeping bare-Esc responsive.
    const ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS: i32 = 50;
    const ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS: i32 = 50;
    const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

    /// Read one byte from stdin or from the pending byte queue.
    fn read_required_byte(stdin: &Stdin) -> io::Result<u8> {
        if let Some(byte) = pop_pending_byte() {
            return Ok(byte);
        }

        unsafe_io::read_byte(stdin)
    }

    /// Push one lookahead byte back into the pending queue.
    fn push_pending_byte(byte: u8) {
        PENDING_BYTES.with(|queue| queue.borrow_mut().push_back(byte));
    }

    /// Read an optional byte after waiting up to the requested timeout.
    fn read_optional_byte_with_timeout(stdin: &Stdin, timeout_ms: i32) -> io::Result<Option<u8>> {
        if pending_queue_has_bytes() {
            return Self::read_required_byte(stdin).map(Some);
        }
        if !Self::poll_readable(stdin, timeout_ms)? {
            return Ok(None);
        }
        Self::read_required_byte(stdin).map(Some)
    }

    /// Return whether stdin became ready before `timeout_ms`.
    ///
    /// Returns `true` when `poll` woke up before the timeout for any stdin
    /// read event, and `false` when the timeout elapsed first or readiness did
    /// not include input bytes.
    fn poll_readable(stdin: &Stdin, timeout_ms: i32) -> io::Result<bool> {
        let outcome = poll_fd(stdin, timeout_ms)?;
        Ok(outcome.ready && (outcome.revents & libc::POLLIN) != 0)
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

    /// Decode one `Esc`-prefixed printable ASCII byte into an Alt-modified key.
    fn parse_simple_alt_key(byte: u8) -> Option<Key> {
        match byte {
            b'b' | b'd' | b'f' | b'B' | b'D' | b'F' => Some(Key::Alt(byte as char)),
            _ => None,
        }
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

    /// Return whether one CSI `~` sequence starts bracketed paste collection.
    fn is_bracketed_paste_start(prefix: &[u8], final_byte: u8) -> bool {
        final_byte == b'~' && prefix == b"200"
    }

    /// Read one full bracketed-paste payload and normalize terminal line endings.
    fn read_bracketed_paste(stdin: &Stdin) -> io::Result<String> {
        let mut payload = Vec::new();
        loop {
            payload.push(Self::read_required_byte(stdin)?);
            if payload.ends_with(Self::BRACKETED_PASTE_END) {
                payload.truncate(payload.len() - Self::BRACKETED_PASTE_END.len());
                return Ok(Self::normalize_pasted_text(&payload));
            }
        }
    }

    /// Convert terminal paste bytes into editor text with `\n` line breaks.
    fn normalize_pasted_text(bytes: &[u8]) -> String {
        let mut normalized = String::with_capacity(bytes.len());
        let text = String::from_utf8_lossy(bytes);
        let mut chars = text.chars().peekable();

        // Terminals may send LF, CR, or CRLF during one bracketed paste, so fold
        // every line break shape into the editor's single `\n` representation.
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                normalized.push('\n');
            } else {
                normalized.push(ch);
            }
        }

        normalized
    }

    /// Parse a CSI escape sequence that starts with `ESC [`.
    fn parse_csi_sequence(stdin: &Stdin) -> io::Result<InputEvent> {
        // We already received ESC + '[', so use the shorter intra-sequence timeout.
        let Some(first) =
            Self::read_optional_byte_with_timeout(stdin, Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS)?
        else {
            return Ok(InputEvent::Key(Key::Esc));
        };

        let mut seq = vec![first];
        while !Self::csi_final_byte(*seq.last().expect("sequence is non-empty")) && seq.len() < 16 {
            let Some(next) = Self::read_optional_byte_with_timeout(
                stdin,
                Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS,
            )?
            else {
                return Ok(InputEvent::Key(Key::Esc));
            };
            seq.push(next);
        }

        let Some(final_byte) = seq.last().copied() else {
            return Ok(InputEvent::Key(Key::Esc));
        };
        let prefix = &seq[..seq.len() - 1];

        if Self::is_bracketed_paste_start(prefix, final_byte) {
            return Ok(InputEvent::Paste(Self::read_bracketed_paste(stdin)?));
        }

        if let Some(key) = Self::parse_modified_navigation_key(prefix, final_byte) {
            return Ok(InputEvent::Key(key));
        }

        match final_byte {
            b'A' => return Ok(InputEvent::Key(Key::Up)),
            b'B' => return Ok(InputEvent::Key(Key::Down)),
            b'C' => return Ok(InputEvent::Key(Key::Right)),
            b'D' => return Ok(InputEvent::Key(Key::Left)),
            b'H' => return Ok(InputEvent::Key(Key::Home)),
            b'F' => return Ok(InputEvent::Key(Key::End)),
            b'Z' => return Ok(InputEvent::Key(Key::BackTab)),
            _ => {}
        }

        if final_byte == b'~' {
            return Ok(InputEvent::Key(Self::parse_tilde_key(prefix)));
        }

        if final_byte == b'u' {
            return Ok(InputEvent::Key(
                Self::parse_csi_u_sequence(prefix).unwrap_or(Key::Null),
            ));
        }

        Ok(InputEvent::Key(Key::Null))
    }

    /// Parse an escape sequence that starts with `ESC`.
    fn parse_escape_sequence(stdin: &Stdin) -> io::Result<InputEvent> {
        let Some(second) =
            Self::read_optional_byte_with_timeout(stdin, Self::ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS)?
        else {
            return Ok(InputEvent::Key(Key::Esc));
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
                    return Ok(InputEvent::Key(Key::Esc));
                };
                Ok(InputEvent::Key(match third {
                    b'H' => Key::Home,
                    b'F' => Key::End,
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    b'C' => Key::Right,
                    b'D' => Key::Left,
                    _ => Key::Esc,
                }))
            }
            b @ 0x01..=0x1a => Ok(InputEvent::Key(Key::Alt((b'a' + (b - 1)) as char))),
            byte => {
                if let Some(key) = Self::parse_simple_alt_key(byte) {
                    return Ok(InputEvent::Key(key));
                }
                // Preserve non-Alt followers after ESC so `Esc` then `:` keeps the `:`.
                Self::push_pending_byte(byte);
                Ok(InputEvent::Key(Key::Esc))
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

    /// Decode one normalized input event after the first byte was already read.
    fn decode_input_event_from_first_byte(first: u8, stdin: &Stdin) -> io::Result<InputEvent> {
        // Interpret ASCII control bytes directly before deferring multibyte input
        // to the UTF-8 decoder.
        match first {
            b'\x1b' => Self::parse_escape_sequence(stdin),
            b'\n' | b'\r' => Ok(InputEvent::Key(Key::Char('\n'))),
            0x7f | 0x08 => Ok(InputEvent::Key(Key::Backspace)),
            0x01..=0x1a => Ok(InputEvent::Key(Key::Ctrl((b'a' + (first - 1)) as char))),
            b @ 0x20..=0x7e => Ok(InputEvent::Key(Key::Char(b as char))),
            byte => Ok(InputEvent::Key(Key::Char(Self::read_utf8_char(
                byte, stdin,
            )?))),
        }
    }

    /// Read the next normalized terminal input event.
    ///
    /// Standalone `Esc` stays responsive while common escape sequences decode
    /// into semantic navigation and editing keys, including jittered arrivals.
    pub(crate) fn read_input_event() -> io::Result<InputEvent> {
        let stdin = stdin();
        let first = Self::read_required_byte(&stdin)?;
        Self::decode_input_event_from_first_byte(first, &stdin)
    }

    /// Read the next normalized terminal input event before `timeout`.
    pub(crate) fn read_input_event_timeout(timeout: Duration) -> io::Result<Option<InputEvent>> {
        if pending_queue_has_bytes() {
            return Self::read_input_event().map(Some);
        }

        let stdin = stdin();
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        if !Self::poll_readable(&stdin, timeout_ms)? {
            return Ok(None);
        }

        let first = Self::read_required_byte(&stdin)?;
        Self::decode_input_event_from_first_byte(first, &stdin).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Queue one raw byte slice into the thread-local pending-byte buffer for tests.
    fn queue_pending_bytes(bytes: &[u8]) {
        PENDING_BYTES.with(|queue| {
            let mut queue = queue.borrow_mut();
            queue.clear();
            queue.extend(bytes.iter().copied());
        });
    }

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

    /// Verify that common Meta word-editing keys decode from plain `Esc` prefixes.
    #[test]
    fn test_parse_simple_alt_key_decodes_meta_word_editing_keys() {
        assert_eq!(Terminal::parse_simple_alt_key(b'b'), Some(Key::Alt('b')));
        assert_eq!(Terminal::parse_simple_alt_key(b'd'), Some(Key::Alt('d')));
        assert_eq!(Terminal::parse_simple_alt_key(b'f'), Some(Key::Alt('f')));
        assert_eq!(Terminal::parse_simple_alt_key(b':'), None);
    }

    /// Verify timed reads consume queued lookahead bytes before polling stdin.
    #[test]
    fn test_read_input_event_timeout_drains_pending_queue() {
        queue_pending_bytes(b" ");
        assert_eq!(
            Terminal::read_input_event_timeout(Duration::ZERO).expect("read queued input event"),
            Some(InputEvent::Key(Key::Char(' ')))
        );
        PENDING_BYTES.with(|queue| queue.borrow_mut().clear());
    }

    /// Verify bracketed paste becomes one normalized paste event with `\n` line breaks.
    #[test]
    fn test_read_input_event_timeout_decodes_bracketed_paste() {
        queue_pending_bytes(b"\x1b[200~line 1\r\nline 2\rline 3\n\x1b[201~");
        assert_eq!(
            Terminal::read_input_event_timeout(Duration::ZERO).expect("read paste event"),
            Some(InputEvent::Paste("line 1\nline 2\nline 3\n".to_string()))
        );
        PENDING_BYTES.with(|queue| queue.borrow_mut().clear());
    }
}
