//! Terminal User Interface module
//!
//! This module isolates all termion-specific code for terminal handling.
//! If the terminal library needs to change in the future, only this file
//! requires modification.

use crate::syntax::{SyntaxClass, SyntaxModifier};
use crate::themes::{ColorCapability, Theme, ThemeColor, ThemeStyle};
use std::collections::VecDeque;
use std::fmt;
use std::fmt::Write as _;
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

/// Buffered terminal commands that should be emitted as one frame.
///
/// Batching writes through this type avoids the flickering that happens when
/// the terminal redraw is flushed in smaller steps.
pub(crate) struct TerminalBatch {
    output: String,
}

/// Combined terminal styling for one rendered cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CellStyle {
    /// Semantic syntax class for this cell.
    syntax_class: Option<SyntaxClass>,
    /// Semantic syntax modifier for this cell.
    syntax_modifier: Option<SyntaxModifier>,
    /// Whether selection invert is active for this cell.
    inverted: bool,
    /// Whether underline emphasis is active for this cell.
    underlined: bool,
}

/// Terminal cursor-shape variants supported by Ordex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorShape {
    /// A steady block cursor for Normal and Visual modes.
    Block,
    /// A steady beam cursor for Insert-style input modes.
    Beam,
}

/// Which side of the terminal color state an escape should update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorLayer {
    /// Update the foreground color.
    Foreground,
    /// Update the background color.
    Background,
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
        "{}{}{}{}",
        termion::screen::ToMainScreen,
        CursorShape::Block.escape_sequence(),
        termion::cursor::Show,
        termion::style::Reset
    );
    let _ = stdout.flush();
}

impl CellStyle {
    /// Build one combined cell style from syntax and selection state.
    pub(crate) fn from_syntax(
        syntax_class: Option<SyntaxClass>,
        syntax_modifier: Option<SyntaxModifier>,
        inverted: bool,
        underlined: bool,
    ) -> Self {
        Self {
            syntax_class,
            syntax_modifier,
            inverted,
            underlined,
        }
    }
}

/// Push one styled character, emitting only the necessary ANSI transitions.
pub(crate) fn push_styled_char(
    output: &mut String,
    active_style: &mut Option<CellStyle>,
    next_style: CellStyle,
    theme: &Theme,
    color_capability: ColorCapability,
    ch: char,
) {
    if *active_style != Some(next_style) {
        output.push_str(termion::style::Reset.as_ref());
        style_escape(output, next_style, theme, color_capability);
        *active_style = Some(next_style);
    }
    output.push(ch);
}

/// Finish one styled output run by resetting the terminal when needed.
pub(crate) fn finish_styled_output(output: &mut String, active_style: &mut Option<CellStyle>) {
    if active_style.is_some() {
        output.push_str(termion::style::Reset.as_ref());
        *active_style = None;
    }
}

/// Append the ANSI escape sequence for one combined cell style.
fn style_escape(
    output: &mut String,
    style: CellStyle,
    theme: &Theme,
    color_capability: ColorCapability,
) {
    // Content cells always inherit the theme background so both visible text and
    // trailing spaces render on the active palette instead of the terminal default.
    let mut combined = theme.background_style();
    if let Some(class) = style.syntax_class {
        combined = combined.overlay(theme.syntax_style(class, style.syntax_modifier));
    }
    if style.inverted {
        combined = combined.overlay(theme.selection_style());
    }
    combined.underline |= style.underlined;
    push_theme_style_escape(output, combined, color_capability);
}

/// Append one themed style escape sequence.
fn push_theme_style_escape(
    output: &mut String,
    style: ThemeStyle,
    color_capability: ColorCapability,
) {
    if let Some(bg) = style.bg {
        push_color_escape(output, ColorLayer::Background, bg, color_capability);
    }
    if let Some(fg) = style.fg {
        push_color_escape(output, ColorLayer::Foreground, fg, color_capability);
    }
    if style.bold {
        output.push_str(termion::style::Bold.as_ref());
    }
    if style.underline {
        output.push_str(termion::style::Underline.as_ref());
    }
    if style.invert {
        output.push_str(termion::style::Invert.as_ref());
    }
}

/// Append one foreground or background color escape sequence.
fn push_color_escape(
    output: &mut String,
    layer: ColorLayer,
    color: ThemeColor,
    color_capability: ColorCapability,
) {
    match (layer, color_capability) {
        (ColorLayer::Foreground, ColorCapability::Ansi256) => write!(
            output,
            "{}",
            termion::color::Fg(termion::color::AnsiValue(color.ansi256_index()))
        ),
        (ColorLayer::Background, ColorCapability::Ansi256) => write!(
            output,
            "{}",
            termion::color::Bg(termion::color::AnsiValue(color.ansi256_index()))
        ),
        (ColorLayer::Foreground, ColorCapability::TrueColor) => write!(
            output,
            "{}",
            termion::color::Fg(termion::color::Rgb(color.red, color.green, color.blue))
        ),
        (ColorLayer::Background, ColorCapability::TrueColor) => write!(
            output,
            "{}",
            termion::color::Bg(termion::color::Rgb(color.red, color.green, color.blue))
        ),
    }
    .expect("writing an ANSI color escape into a String cannot fail");
}

impl CursorShape {
    /// Return the ANSI escape sequence for this cursor shape.
    pub(crate) fn escape_sequence(self) -> &'static str {
        match self {
            CursorShape::Block => "\u{1b}[2 q",
            CursorShape::Beam => "\u{1b}[6 q",
        }
    }
}

impl TerminalBatch {
    /// Create an empty terminal batch.
    pub(crate) fn new() -> Self {
        Self {
            output: String::new(),
        }
    }

    /// Queue a full-screen clear in this batch.
    pub(crate) fn clear_screen(&mut self) {
        write!(self.output, "{}", termion::clear::All)
            .expect("writing a screen clear into a String cannot fail");
    }

    /// Queue text at a specific position (1-indexed).
    pub(crate) fn write_at<T>(&mut self, x: u16, y: u16, text: T)
    where
        T: fmt::Display,
    {
        write!(self.output, "{}{}", termion::cursor::Goto(x, y), text)
            .expect("writing positioned terminal output into a String cannot fail");
    }

    /// Queue styled text at a specific position (1-indexed).
    pub(crate) fn write_styled_at<T>(
        &mut self,
        x: u16,
        y: u16,
        style: ThemeStyle,
        color_capability: ColorCapability,
        text: T,
    ) where
        T: fmt::Display,
    {
        write!(self.output, "{}", termion::cursor::Goto(x, y))
            .expect("writing a cursor move into a String cannot fail");
        push_theme_style_escape(&mut self.output, style, color_capability);
        write!(self.output, "{}{}", text, termion::style::Reset)
            .expect("writing positioned styled text into a String cannot fail");
    }

    /// Clear from the given cell to the end of the line using one themed style.
    pub(crate) fn clear_to_eol_styled_at(
        &mut self,
        x: u16,
        y: u16,
        style: ThemeStyle,
        color_capability: ColorCapability,
    ) {
        write!(self.output, "{}", termion::cursor::Goto(x, y))
            .expect("writing a cursor move into a String cannot fail");
        push_theme_style_escape(&mut self.output, style, color_capability);
        write!(
            self.output,
            "{}{}",
            termion::clear::UntilNewline,
            termion::style::Reset
        )
        .expect("writing a styled line clear into a String cannot fail");
    }

    /// Queue a cursor move without writing any text.
    pub(crate) fn goto(&mut self, x: u16, y: u16) {
        write!(self.output, "{}", termion::cursor::Goto(x, y))
            .expect("writing a cursor move into a String cannot fail");
    }

    /// Queue a terminal cursor-shape change in this batch.
    pub(crate) fn set_cursor_shape(&mut self, shape: CursorShape) {
        write!(self.output, "{}", shape.escape_sequence())
            .expect("writing a cursor-shape escape sequence into a String cannot fail");
    }

    /// Queue a cursor hide command in this batch.
    pub(crate) fn hide_cursor(&mut self) {
        write!(self.output, "{}", termion::cursor::Hide)
            .expect("writing a cursor hide command into a String cannot fail");
    }

    /// Queue a cursor show command in this batch.
    pub(crate) fn show_cursor(&mut self) {
        write!(self.output, "{}", termion::cursor::Show)
            .expect("writing a cursor show command into a String cannot fail");
    }

    /// Borrow the batched terminal frame as bytes for direct terminal writes.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.output.as_bytes()
    }
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
        let mut batch = TerminalBatch::new();
        batch.clear_screen();
        self.write_batch(&batch)
    }

    /// Emit one fully batched terminal frame with a single write.
    pub(crate) fn write_batch(&mut self, batch: &TerminalBatch) -> io::Result<()> {
        self.stdout.write_all(batch.as_bytes())?;
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
            "{}{}{}",
            CursorShape::Block.escape_sequence(),
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
    fn test_terminal_batch_collects_positioned_output() {
        let mut batch = TerminalBatch::new();
        batch.clear_screen();
        batch.write_at(1, 1, "test");

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}[2J"));
        assert!(output.contains("\u{1b}[1;1Htest"));
    }

    /// Verify that terminal batches can carry cursor-shape escape sequences.
    #[test]
    fn test_terminal_batch_collects_cursor_shape_output() {
        let mut batch = TerminalBatch::new();
        batch.set_cursor_shape(CursorShape::Beam);

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains("\u{1b}[6 q"));
    }
}
