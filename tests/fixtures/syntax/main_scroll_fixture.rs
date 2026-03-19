#![allow(clippy::question_mark)]

//! Frozen `src/main.rs` scroll fixture.
//!
//! This snapshot covers the region used by the syntax-scroll regressions so the
//! tests keep exercising the same text even when Ordex's real main module
//! changes later.

// TODO: Write the asciidoctor doc for ordex (possibly using Hugo if asciidoctor alone is not
// enough).

mod config;
mod cursor;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod signal;
mod soft_wrap;
mod syntax;
mod text_buffer;
mod themes;
mod tui;
mod viewport;

use editor_state::{EditorRequest, EditorState, SequenceDiscoveryPopup};
use signal::SignalGuard;
use std::borrow::Cow;
use std::env;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use termion::event::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalSize {
    width: u16,
    height: u16,
}

const MIN_GUTTER_DIGITS: usize = 3;
const GUTTER_SEPARATOR_WIDTH: usize = 1;
const RESERVED_BOTTOM_ROWS: u16 = 2;
const POPUP_MIN_WIDTH: usize = 4;
const POPUP_MIN_HEIGHT: usize = 3;
const POPUP_BORDER_INSET: usize = 2;
const POPUP_TITLE_PADDING: usize = 1;
const POPUP_ENTRY_GAP: &str = "  ";
const POPUP_TOP_LEFT: char = '┌';
const POPUP_TOP_RIGHT: char = '┐';
const POPUP_BOTTOM_LEFT: char = '└';
const POPUP_BOTTOM_RIGHT: char = '┘';
const POPUP_HORIZONTAL: char = '─';
const POPUP_VERTICAL: char = '│';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderLayout {
    gutter_digits: usize,
    gutter_total_width: usize,
    content_width: usize,
}

impl RenderLayout {
    fn from_size(size: TerminalSize, total_lines: usize) -> Self {
        let gutter_digits = total_lines.max(1).to_string().len().max(MIN_GUTTER_DIGITS);
        let gutter_total_width = gutter_digits + GUTTER_SEPARATOR_WIDTH;
        let content_width = (size.width as usize).saturating_sub(gutter_total_width);
        Self {
            gutter_digits,
            gutter_total_width,
            content_width,
        }
    }
}

/// Synchronize viewport width with the current render layout before painting.
fn prepare_viewport_for_render(editor: &mut EditorState, size: TerminalSize) -> RenderLayout {
    let layout = RenderLayout::from_size(size, editor.buffer.lines_count());
    let content_width = layout.content_width.max(1);
    let width_changed = editor.viewport.width() != content_width;

    // Gutter-width changes alter the effective content width, which can change
    // wrapped rows or horizontal scrolling even when the cursor itself is stable.
    editor.viewport.set_width(content_width);
    if width_changed {
        editor
            .viewport
            .ensure_cursor_visible(&editor.cursor, &editor.buffer);
    }
    layout
}

/// Snapshot of all editor state that can affect what the terminal must redraw.
///
/// This is used to avoid full-screen redraws when only the message line changed
/// (for example, when typing a sequence prefix like `g`).
/// Semantic mode identity used for redraw decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Normal,
    Visual(mode::VisualKind),
    Insert,
    Command,
    Search,
}

impl RenderMode {
    /// Capture the stable redraw-relevant identity of one editor mode.
    fn capture(mode: &mode::Mode) -> Self {
        match mode {
            mode::Mode::Normal => RenderMode::Normal,
            mode::Mode::Visual(kind) => RenderMode::Visual(*kind),
            mode::Mode::Insert => RenderMode::Insert,
            mode::Mode::Command(_) => RenderMode::Command,
            mode::Mode::Search(_) => RenderMode::Search,
        }
    }

    /// Return whether this mode paints the active cursor directly into content.
    fn paints_content_cursor(self) -> bool {
        matches!(self, RenderMode::Visual(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderSnapshot {
    cursor_line: usize,
    cursor_column: usize,
    first_visible_line: usize,
    first_visible_row: usize,
    first_visible_column: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    mode: RenderMode,
    file_name: String,
    modified: bool,
    buffer_lines: usize,
    buffer_chars: usize,
    syntax_generation: u64,
    theme_name: &'static str,
    pending_prefix: Option<String>,
    input_prompt: Option<char>,
    input_line: Option<String>,
    input_cursor_col: Option<usize>,
    overwrite_prompt: Option<String>,
    quit_prompt: Option<String>,
    status_message: Option<String>,
    sequence_discovery_popup: Option<SequenceDiscoveryPopup>,
}
