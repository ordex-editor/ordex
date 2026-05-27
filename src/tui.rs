//! Terminal user interface facade.
//!
//! This module isolates all termion-specific code for terminal handling while
//! exposing a stable surface for the rest of the editor.

use std::io::{self, Write, stdout};
use std::panic;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::{AlternateScreen, IntoAlternateScreen};

pub(crate) use self::output::{
    CellStyle, CursorShape, TerminalBatch, finish_styled_output, push_styled_char, push_styled_text,
};
pub(crate) use input::InputEvent;

const SYNC_UPDATE_BEGIN: &str = "\u{1b}[?2026h";
const SYNC_UPDATE_END: &str = "\u{1b}[?2026l";
const ENABLE_BRACKETED_PASTE: &str = "\u{1b}[?2004h";
const DISABLE_BRACKETED_PASTE: &str = "\u{1b}[?2004l";
const RESET_CURSOR_COLOR: &str = "\u{1b}]112\u{7}";

mod input;
mod output;
mod unsafe_io;

/// Terminal wrapper with RAII cleanup.
///
/// Ensures the terminal is restored to normal mode even on panic.
pub(crate) struct Terminal {
    stdout: AlternateScreen<RawTerminal<io::Stdout>>,
}

/// Restore the terminal to a sane state during panic cleanup.
fn restore_terminal() {
    let mut stdout = stdout();
    // End any synchronized update frame before leaving the alternate screen so
    // supporting terminals present the final repaint immediately.
    let _ = write!(
        stdout,
        "{}{}{}{}{}{}{}",
        SYNC_UPDATE_END,
        DISABLE_BRACKETED_PASTE,
        termion::screen::ToMainScreen,
        CursorShape::Block.escape_sequence(),
        termion::cursor::Show,
        termion::style::Reset,
        RESET_CURSOR_COLOR
    );
    let _ = stdout.flush();
}

impl Terminal {
    /// Initialize the terminal in raw mode with the alternate screen enabled.
    ///
    /// Raw mode disables line buffering and echo, allowing character-by-character input.
    /// The alternate screen preserves the original shell content.
    pub(crate) fn new() -> io::Result<Self> {
        // Install panic cleanup before raw mode takes over terminal state.
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            restore_terminal();
            default_hook(info);
        }));

        let mut stdout = stdout().into_raw_mode()?.into_alternate_screen()?;
        write!(stdout, "{ENABLE_BRACKETED_PASTE}")?;
        stdout.flush()?;
        Ok(Self { stdout })
    }
}

impl Drop for Terminal {
    /// Restore the terminal to normal mode on drop.
    fn drop(&mut self) {
        // Reset cursor presentation before the alternate screen is released.
        let _ = write!(
            self.stdout,
            "{}{}{}{}{}",
            DISABLE_BRACKETED_PASTE,
            CursorShape::Block.escape_sequence(),
            termion::cursor::Show,
            termion::style::Reset,
            RESET_CURSOR_COLOR
        );
        let _ = self.stdout.flush();
    }
}
