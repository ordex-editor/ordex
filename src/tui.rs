//! Terminal user interface facade.
//!
//! This module isolates all termion-specific code for terminal handling while
//! exposing a stable surface for the rest of the editor.

use std::io::{self, Write, stdout};
use std::panic;
use std::sync::{Mutex, MutexGuard, OnceLock};
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
const SAVE_WINDOW_TITLE: &str = "\u{1b}[22;2t";
const RESTORE_WINDOW_TITLE: &str = "\u{1b}[23;2t";
static WINDOW_TITLE_RESTORE_ARMED: OnceLock<Mutex<bool>> = OnceLock::new();

mod input;
mod output;
mod unsafe_io;

/// Terminal wrapper with RAII cleanup.
///
/// Ensures the terminal is restored to normal mode even on panic.
pub(crate) struct Terminal {
    stdout: AlternateScreen<RawTerminal<io::Stdout>>,
    restore_window_title_on_exit: bool,
}

/// Return the shared title-restore flag used by drop and panic cleanup.
fn window_title_restore_slot() -> &'static Mutex<bool> {
    WINDOW_TITLE_RESTORE_ARMED.get_or_init(|| Mutex::new(false))
}

/// Acquire one mutex guard even when prior panic poisoning occurred.
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        // The title cache stores best-effort terminal metadata only.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Ask the terminal to save the current window title for later restoration.
///
/// This sends the xterm-compatible `CSI 22;2 t` control sequence. Supporting
/// terminals cache the current title internally so Ordex can restore it on exit
/// without reading from stdin or parsing a terminal response.
///
/// Returns `true` when the save sequence was successfully written to the output
/// stream, and `false` when writing failed so exit-time restoration must be
/// skipped.
fn arm_window_title_restore(writer: &mut impl Write) -> bool {
    write!(writer, "{SAVE_WINDOW_TITLE}").is_ok()
}

/// Restore the terminal title from terminal-managed saved state when available.
///
/// When `restore_armed` is `true`, this writes the xterm-compatible
/// `CSI 23;2 t` sequence so terminals that accepted the earlier save request
/// restore the pre-Ordex title. When `restore_armed` is `false`, this performs
/// no write so behavior stays a safe no-op on terminals or sessions where title
/// save was unavailable.
fn restore_window_title_if_armed(writer: &mut impl Write, restore_armed: bool) {
    if restore_armed {
        let _ = write!(writer, "{RESTORE_WINDOW_TITLE}");
    }
}

/// Restore the terminal to a sane state during panic cleanup.
fn restore_terminal() {
    let mut stdout = stdout();
    let restore_armed = {
        let guard = lock_unpoisoned(window_title_restore_slot());
        *guard
    };
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
        RESET_CURSOR_COLOR,
    );
    restore_window_title_if_armed(&mut stdout, restore_armed);
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
        let restore_window_title_on_exit = arm_window_title_restore(&mut stdout);
        {
            let mut slot = lock_unpoisoned(window_title_restore_slot());
            *slot = restore_window_title_on_exit;
        }
        stdout.flush()?;
        Ok(Self {
            stdout,
            restore_window_title_on_exit,
        })
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
        restore_window_title_if_armed(&mut self.stdout, self.restore_window_title_on_exit);
        let _ = self.stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that title restore is emitted when the restore flag is armed.
    #[test]
    fn test_restore_window_title_if_armed_emits_restore_escape() {
        let mut output = Vec::new();
        restore_window_title_if_armed(&mut output, true);
        assert_eq!(
            String::from_utf8(output).expect("utf8 output"),
            RESTORE_WINDOW_TITLE
        );
    }

    /// Verify that title restore is skipped when arming never succeeded.
    #[test]
    fn test_restore_window_title_if_armed_skips_when_unarmed() {
        let mut output = Vec::new();
        restore_window_title_if_armed(&mut output, false);
        assert!(output.is_empty());
    }
}
