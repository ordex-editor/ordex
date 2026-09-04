//! Terminal input parsing and key decoding.
//!
//! Decoders in this module only ever see a [`BoundedInput`], so every read they
//! can perform has a deadline. The unbounded wait lives on `InputSource` in
//! [`byte_source`], whose stdin handle this module cannot reach.

use super::Terminal;
use byte_source::{BoundedInput, InputSource, pending_queue_has_bytes, push_pending_byte};
use std::io;
use std::time::Duration;
use termion::event::Key;

mod byte_source;

/// One normalized terminal input unit routed through the app event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InputEvent {
    Key(Key),
    Paste(String),
}

impl Terminal {
    // 50 ms matches neovim's default `ttimeoutlen` and covers even
    // high-latency SSH/tmux links while keeping bare-Esc responsive.
    const ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS: i32 = 50;
    const ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS: i32 = 50;
    // A lead byte can only ever start a multi-byte character, so waiting longer
    // costs no responsiveness and keeps a link that splits the sequence across
    // packets from decoding one character as several.
    const UTF8_CONTINUATION_BYTE_TIMEOUT_MS: i32 = 1_000;
    // Terminals stream a paste as fast as the tty accepts it, so a full second
    // of silence means the payload is over and the terminator is never coming.
    const BRACKETED_PASTE_IDLE_TIMEOUT_MS: i32 = 1_000;
    const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

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

    /// Decode one `Esc`-prefixed byte into an Alt-modified key.
    ///
    /// Handles printable word-editing ASCII letters and the DEL byte (`0x7f`),
    /// which terminals send as Alt-Backspace.
    fn parse_simple_alt_key(byte: u8) -> Option<Key> {
        match byte {
            b'b' | b'd' | b'f' | b'B' | b'D' | b'F' => Some(Key::Alt(byte as char)),
            // ESC + DEL (0x7f) is the standard terminal encoding for Alt-Backspace.
            0x7f => Some(Key::Alt('\x7f')),
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
    ///
    /// Collection ends at the paste terminator, or once the stream stays silent
    /// for `BRACKETED_PASTE_IDLE_TIMEOUT_MS`, so a paste-start sequence whose
    /// terminator never arrives yields the bytes read so far instead of holding
    /// the event loop forever.
    fn read_bracketed_paste(input: &BoundedInput<'_>) -> io::Result<String> {
        let mut payload = Vec::new();
        loop {
            let Some(byte) = input.read_byte_within(Self::BRACKETED_PASTE_IDLE_TIMEOUT_MS)? else {
                return Ok(Self::normalize_pasted_text(&payload));
            };
            payload.push(byte);
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
    fn parse_csi_sequence(input: &BoundedInput<'_>) -> io::Result<InputEvent> {
        // We already received ESC + '[', so use the shorter intra-sequence timeout.
        let Some(first) = input.read_byte_within(Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS)? else {
            return Ok(InputEvent::Key(Key::Esc));
        };

        let mut seq = vec![first];
        while !Self::csi_final_byte(*seq.last().expect("sequence is non-empty")) && seq.len() < 16 {
            let Some(next) = input.read_byte_within(Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS)?
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
            return Ok(InputEvent::Paste(Self::read_bracketed_paste(input)?));
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
    fn parse_escape_sequence(input: &BoundedInput<'_>) -> io::Result<InputEvent> {
        let Some(second) = input.read_byte_within(Self::ESC_SEQUENCE_FIRST_BYTE_TIMEOUT_MS)? else {
            return Ok(InputEvent::Key(Key::Esc));
        };

        match second {
            b'[' => Self::parse_csi_sequence(input),
            b'O' => {
                // SS3 sequences carry Home/End and arrow keys in some terminal modes.
                let Some(third) =
                    input.read_byte_within(Self::ESC_SEQUENCE_NEXT_BYTE_TIMEOUT_MS)?
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
                push_pending_byte(byte);
                Ok(InputEvent::Key(Key::Esc))
            }
        }
    }

    /// Decode one UTF-8 character starting from the first already-read byte.
    ///
    /// A lead byte whose continuation bytes never arrive decodes as the lead
    /// byte itself once `UTF8_CONTINUATION_BYTE_TIMEOUT_MS` elapses, so a
    /// truncated multi-byte sequence cannot hold the event loop forever.
    fn read_utf8_char(first: u8, input: &BoundedInput<'_>) -> io::Result<char> {
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
            let Some(next) = input.read_byte_within(Self::UTF8_CONTINUATION_BYTE_TIMEOUT_MS)?
            else {
                return Ok(char::from(first));
            };
            // UTF-8 continuation bytes must have the `10xxxxxx` shape.
            if (next & 0b1100_0000) != 0b1000_0000 {
                // Put back the unexpected byte so input stream alignment is preserved.
                push_pending_byte(next);
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
    fn decode_input_event_from_first_byte(
        first: u8,
        input: &BoundedInput<'_>,
    ) -> io::Result<InputEvent> {
        // Interpret ASCII control bytes directly before deferring multibyte input
        // to the UTF-8 decoder.
        match first {
            b'\x1b' => Self::parse_escape_sequence(input),
            b'\n' | b'\r' => Ok(InputEvent::Key(Key::Char('\n'))),
            0x7f | 0x08 => Ok(InputEvent::Key(Key::Backspace)),
            0x01..=0x1a => Ok(InputEvent::Key(Key::Ctrl((b'a' + (first - 1)) as char))),
            b @ 0x20..=0x7e => Ok(InputEvent::Key(Key::Char(b as char))),
            byte => Ok(InputEvent::Key(Key::Char(Self::read_utf8_char(
                byte, input,
            )?))),
        }
    }

    /// Read the next normalized terminal input event.
    ///
    /// Standalone `Esc` stays responsive while common escape sequences decode
    /// into semantic navigation and editing keys, including jittered arrivals.
    pub(crate) fn read_input_event() -> io::Result<InputEvent> {
        let source = InputSource::new();
        let first = source.wait_for_byte()?;
        Self::decode_input_event_from_first_byte(first, &source.bounded())
    }

    /// Read the next normalized terminal input event before `timeout`.
    ///
    /// On macOS, PTY slave file descriptors can report `POLLIN` via `poll` even
    /// when no bytes are present (spurious wakeup).  To avoid blocking on the
    /// subsequent `read` call, this function uses a non-blocking read attempt
    /// after `poll` signals readiness and treats an `EAGAIN` result as a timeout.
    pub(crate) fn read_input_event_timeout(timeout: Duration) -> io::Result<Option<InputEvent>> {
        if pending_queue_has_bytes() {
            return Self::read_input_event().map(Some);
        }

        let source = InputSource::new();
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        if !source.poll_readable(timeout_ms)? {
            return Ok(None);
        }

        // poll() reported readiness, but on macOS PTY slaves, this can be
        // spurious.  A non-blocking read attempt surfaces that case as None
        // rather than blocking indefinitely.
        let Some(first) = source.try_read_byte()? else {
            return Ok(None);
        };
        Self::decode_input_event_from_first_byte(first, &source.bounded()).map(Some)
    }

    /// Return whether there is input available immediately without consuming it.
    ///
    /// Returns `true` when there are bytes in the pending queue or stdin has
    /// readable data right now (zero-timeout poll), and `false` otherwise.
    pub(crate) fn has_input_pending() -> io::Result<bool> {
        if pending_queue_has_bytes() {
            return Ok(true);
        }
        InputSource::new().poll_readable(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byte_source::{clear_pending_bytes, queue_pending_bytes};

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

    /// Verify that ESC + DEL (0x7f) decodes as Alt-Backspace.
    #[test]
    fn test_parse_simple_alt_key_decodes_alt_backspace() {
        assert_eq!(Terminal::parse_simple_alt_key(0x7f), Some(Key::Alt('\x7f')));
    }

    /// Verify that queued ESC + DEL bytes decode into the Alt-Backspace key event.
    #[test]
    fn test_read_input_event_timeout_decodes_alt_backspace() {
        queue_pending_bytes(&[0x1b, 0x7f]);
        assert_eq!(
            Terminal::read_input_event_timeout(Duration::ZERO).expect("read alt-backspace event"),
            Some(InputEvent::Key(Key::Alt('\x7f')))
        );
        clear_pending_bytes();
    }

    /// Verify timed reads consume queued lookahead bytes before polling stdin.
    #[test]
    fn test_read_input_event_timeout_drains_pending_queue() {
        queue_pending_bytes(b" ");
        assert_eq!(
            Terminal::read_input_event_timeout(Duration::ZERO).expect("read queued input event"),
            Some(InputEvent::Key(Key::Char(' ')))
        );
        clear_pending_bytes();
    }

    /// Verify bracketed paste becomes one normalized paste event with `\n` line breaks.
    #[test]
    fn test_read_input_event_timeout_decodes_bracketed_paste() {
        queue_pending_bytes(b"\x1b[200~line 1\r\nline 2\rline 3\n\x1b[201~");
        assert_eq!(
            Terminal::read_input_event_timeout(Duration::ZERO).expect("read paste event"),
            Some(InputEvent::Paste("line 1\nline 2\nline 3\n".to_string()))
        );
        clear_pending_bytes();
    }
}
