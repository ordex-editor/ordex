#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! This is the main entry point for the ordex text editor.
//! It handles CLI argument parsing, file loading, terminal initialization,
//! and the main event loop.

// TODO: Write the asciidoctor doc for ordex (possibly using Hugo if asciidoctor alone is not
// enough).

mod cursor;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod signal;
mod text_buffer;
mod tui;
mod viewport;

use editor_state::EditorState;
use signal::SigwinchGuard;
use std::env;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::process;
use termion::event::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalSize {
    width: u16,
    height: u16,
}

const MIN_GUTTER_DIGITS: usize = 3;
const GUTTER_SEPARATOR_WIDTH: usize = 1;

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

/// Snapshot of all editor state that can affect what the terminal must redraw.
///
/// This is used to avoid full-screen redraws when only the message line changed
/// (for example, when typing a sequence prefix like `g`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderSnapshot {
    cursor_line: usize,
    cursor_column: usize,
    first_visible_line: usize,
    first_visible_column: usize,
    mode_name: String,
    file_name: String,
    modified: bool,
    buffer_lines: usize,
    buffer_chars: usize,
    pending_prefix: Option<String>,
    input_prompt: Option<char>,
    input_line: Option<String>,
    overwrite_prompt: Option<String>,
    quit_prompt: Option<String>,
    status_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderDecision {
    /// Nothing visible changed; skip rendering to avoid unnecessary cursor blink.
    None,
    /// Only command/message-row state changed; update that row without full redraw.
    MessageOnly,
    /// Cursor/content/status layout changed; perform full render.
    Full,
}

impl RenderSnapshot {
    /// Build a render snapshot from the current editor state.
    ///
    /// The snapshot contains only fields that affect terminal output so we can
    /// compare two states and choose the smallest valid redraw.
    fn capture(editor: &EditorState) -> Self {
        let file_name = editor
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("[No Name]")
            .to_string();

        Self {
            cursor_line: editor.cursor.line(),
            cursor_column: editor.cursor.column(),
            first_visible_line: editor.viewport.first_visible_line(),
            first_visible_column: editor.viewport.first_visible_column(),
            mode_name: editor.mode_name().to_string(),
            file_name,
            modified: editor.buffer.is_modified(),
            buffer_lines: editor.buffer.lines_count(),
            buffer_chars: editor.buffer.chars_count(),
            pending_prefix: editor.pending_prefix_label(),
            input_prompt: editor.input_prompt(),
            input_line: editor.input_line().map(|s| s.to_string()),
            overwrite_prompt: editor.overwrite_prompt(),
            quit_prompt: editor.quit_prompt(),
            status_message: editor.status_message.clone(),
        }
    }

    /// Decide the minimal redraw required between two snapshots.
    ///
    /// Returns:
    /// - `Full` when viewport/status/cursor/content changed,
    /// - `MessageOnly` when only message-row state changed,
    /// - `None` when nothing visible changed.
    fn decide(before: &Self, after: &Self) -> RenderDecision {
        // Any content/cursor/layout/mode change can affect the main viewport or
        // status bar, so it requires a full redraw.
        let full_changed = before.cursor_line != after.cursor_line
            || before.cursor_column != after.cursor_column
            || before.first_visible_line != after.first_visible_line
            || before.first_visible_column != after.first_visible_column
            || before.mode_name != after.mode_name
            || before.file_name != after.file_name
            || before.modified != after.modified
            || before.buffer_lines != after.buffer_lines
            || before.buffer_chars != after.buffer_chars;

        if full_changed {
            return RenderDecision::Full;
        }

        // If only prompt/message/prefix changed, redraw just the message row to
        // reduce cursor flicker from hide/show cycles.
        let message_changed = before.pending_prefix != after.pending_prefix
            || before.input_prompt != after.input_prompt
            || before.input_line != after.input_line
            || before.overwrite_prompt != after.overwrite_prompt
            || before.quit_prompt != after.quit_prompt
            || before.status_message != after.status_message;

        if message_changed {
            RenderDecision::MessageOnly
        } else {
            RenderDecision::None
        }
    }
}

/// Entry point for the application
///
/// Delegates to run() and handles errors by printing to stderr
fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

/// Main application logic
///
/// Loads the file, initializes the terminal, and runs the event loop
fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let file_path = args.get(1);

    // Initialize terminal
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    let mut terminal_size = TerminalSize::from_termion(termion::terminal_size()?);
    let sigwinch = SigwinchGuard::install()?;

    // Initialize editor state with terminal height
    let mut editor = EditorState::new(terminal_size.height as usize);

    if let Some(path) = file_path {
        if std::path::Path::new(path).exists() {
            editor.load_file(path)?;
        } else {
            // New file with specified name
            editor.file_path = std::path::PathBuf::from(path);
        }
    }

    let mut key_log = init_key_log()?;

    let mut needs_render = true;
    let mut needs_message_render = false;
    sigwinch.mark_pending();

    // Main event loop
    loop {
        // Refresh terminal dimensions only when SIGWINCH arrives.
        if sigwinch.take_pending() {
            let current_size = TerminalSize::from_termion(termion::terminal_size()?);
            if current_size != terminal_size {
                terminal_size = current_size;
                let layout = RenderLayout::from_size(terminal_size, editor.buffer.lines_count());
                // Width tracks visible text columns, excluding the line-number gutter.
                editor.handle_resize(layout.content_width.max(1), terminal_size.height as usize);
                needs_render = true;
            }
        }

        if needs_render {
            // Render current view
            render_editor(&mut term, &mut editor, terminal_size)?;

            // Clear status message after displaying
            editor.status_message = None;
            needs_render = false;
            needs_message_render = false;
        } else if needs_message_render {
            render_message_line(&mut term, &editor, terminal_size)?;
            editor.status_message = None;
            needs_message_render = false;
        }

        // Block for input; SIGWINCH interrupts this read to trigger a resize redraw.
        match tui::Terminal::read_key() {
            Ok(key) => {
                let before_mode = editor.mode.mode_label();
                // Capture state before handling input so we can decide the minimal
                // redraw needed after applying the key.
                let before = RenderSnapshot::capture(&editor);
                editor.handle_key(key);
                log_key_event(&mut key_log, key, before_mode, &editor);
                if editor.should_quit {
                    break;
                }
                let after = RenderSnapshot::capture(&editor);
                match RenderSnapshot::decide(&before, &after) {
                    RenderDecision::Full => {
                        needs_render = true;
                        needs_message_render = false;
                    }
                    RenderDecision::MessageOnly => {
                        if !needs_render {
                            needs_message_render = true;
                        }
                    }
                    RenderDecision::None => {}
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Initialize optional key logging from `ORDEX_KEY_LOG`.
///
/// When set to a non-empty path, events are appended to that file.
fn init_key_log() -> io::Result<Option<File>> {
    match env::var("ORDEX_KEY_LOG") {
        Ok(path) if !path.trim().is_empty() => OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(Some)
            .map_err(|e| io::Error::other(format!("failed to open ORDEX_KEY_LOG file: {e}"))),
        _ => Ok(None),
    }
}

/// Append one key event to the debug key log (when enabled).
fn log_key_event(log: &mut Option<File>, key: Key, mode_before: &str, editor: &EditorState) {
    if let Some(log_file) = log.as_mut() {
        let _ = writeln!(
            log_file,
            "key={:?} mode_before={} mode_after={} cursor={}:{}",
            key,
            mode_before,
            editor.mode_name(),
            editor.cursor.line() + 1,
            editor.cursor.column() + 1
        );
    }
}

/// Terminal-size normalization helpers.
///
/// PTY backends may report 0x0 before size is explicitly set. We clamp to a
/// small usable size to keep rendering deterministic.
impl TerminalSize {
    fn from_termion((width, height): (u16, u16)) -> Self {
        // Height reserves 2 lines for status + message rows.
        Self {
            width: width.max(1),
            height: height.max(3),
        }
    }
}

/// Render the editor state to the terminal
fn render_editor(
    term: &mut tui::Terminal,
    editor: &mut EditorState,
    size: TerminalSize,
) -> io::Result<()> {
    term.hide_cursor()?;

    // Reserve bottom 2 lines for status bar and command/message line
    let content_height = size.height.saturating_sub(2) as usize;
    let layout = RenderLayout::from_size(size, editor.buffer.lines_count());

    // Update viewport width
    editor.viewport.set_width(layout.content_width.max(1));
    editor
        .viewport
        .ensure_cursor_visible(&editor.cursor, &editor.buffer);

    // Render visible lines from the buffer
    let first_line = editor.viewport.first_visible_line();
    let first_col = editor.viewport.first_visible_column();
    for row in 0..content_height {
        let line_idx = first_line + row;
        let y = (row + 1) as u16;

        // Write visible content first, then clear only the remainder of the row.
        let row_str = if let Some(line) = editor.buffer.line_for_display(line_idx) {
            let content = if layout.content_width == 0 {
                String::new()
            } else {
                line.chars()
                    .skip(first_col)
                    .take(layout.content_width)
                    .collect::<String>()
            };
            let number = line_idx + 1;
            format!("{number:>width$} {content}", width = layout.gutter_digits)
        } else {
            format!("{:>width$} ", "~", width = layout.gutter_digits)
        };

        let cut = row_str
            .char_indices()
            .nth(size.width as usize)
            .map_or(row_str.len(), |(idx, _)| idx);
        let visible_row = &row_str[..cut];
        let line_len = visible_row.chars().count() as u16;
        term.write_at(1, y, visible_row)?;
        if line_len < size.width {
            term.write_at(
                1 + line_len,
                y,
                &format!("{}", termion::clear::UntilNewline),
            )?;
        }
    }

    // Render status bar (second to last line)
    let status_y = size.height - 1;
    let mode_str = editor.mode_name();
    let pos_str = format!(
        "{}:{} ",
        editor.cursor.line() + 1,
        editor.cursor.column() + 1
    );
    let modified = if editor.buffer.is_modified() {
        "[+] "
    } else {
        ""
    };
    let file_name = editor
        .file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("[No Name]");

    let status_left = format!(" {} | {}{}", mode_str, modified, file_name);
    let status_right = pos_str;
    let padding = size
        .width
        .saturating_sub((status_left.len() + status_right.len()) as u16) as usize;
    let status_line = format!("{}{:padding$}{}", status_left, "", status_right);

    // Invert colors for status bar
    term.write_at(
        1,
        status_y,
        &format!(
            "{}{}{}",
            termion::style::Invert,
            &status_line[..status_line.len().min(size.width as usize)],
            termion::style::Reset
        ),
    )?;

    // Render command/message line (last line)
    write_message_line(term, editor, size)?;

    // Position cursor (accounting for scroll offsets)
    let cursor_x = if layout.content_width == 0 {
        size.width
    } else {
        (layout.gutter_total_width
            + editor
                .cursor
                .column()
                .saturating_sub(editor.viewport.first_visible_column())
            + 1) as u16
    }
    .clamp(1, size.width);
    let cursor_y = (editor
        .cursor
        .line()
        .saturating_sub(editor.viewport.first_visible_line())
        + 1) as u16;
    term.write_at(cursor_x, cursor_y, "")?;
    term.show_cursor()?;
    term.flush()?;

    Ok(())
}

fn render_message_line(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
) -> io::Result<()> {
    // Save/restore keeps the user's visible cursor position stable while writing
    // to the bottom message row.
    term.save_cursor()?;
    write_message_line(term, editor, size)?;
    term.restore_cursor()?;
    term.flush()
}

fn write_message_line(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
) -> io::Result<()> {
    let msg_y = size.height;
    term.write_at(1, msg_y, &format!("{}", termion::clear::CurrentLine))?;

    let left_message = if let Some(prompt) = editor.overwrite_prompt() {
        prompt
    } else if let Some(prompt) = editor.quit_prompt() {
        prompt
    } else if let (Some(prompt), Some(input)) = (editor.input_prompt(), editor.input_line()) {
        format!("{}{}", prompt, input)
    } else if let Some(ref msg) = editor.status_message {
        msg.clone()
    } else {
        String::new()
    };

    let pending_marker = editor.pending_prefix_label().map(|label| label.to_string());

    let width = size.width as usize;
    if let Some(marker) = pending_marker {
        const RIGHT_PADDING: usize = 10;
        let marker_len = marker.chars().count().min(width);
        let marker_x = (width.saturating_sub(marker_len + RIGHT_PADDING) + 1) as u16;
        // `usize::from(!left_message.is_empty())` converts a bool to 0 or 1:
        // - 1 when left-side content exists (reserve one separator space),
        // - 0 when left-side content is empty (no separator needed).
        // This keeps marker spacing predictable without branching.
        let max_left_len = width
            .saturating_sub(marker_len + RIGHT_PADDING + usize::from(!left_message.is_empty()));
        let left_text: String = left_message.chars().take(max_left_len).collect();

        if !left_text.is_empty() {
            term.write_at(1, msg_y, &left_text)?;
        }

        let marker_text: String = marker
            .chars()
            .rev()
            .take(marker_len)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        term.write_at(marker_x, msg_y, &marker_text)?;
    } else if !left_message.is_empty() {
        term.write_at(1, msg_y, &left_message)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use std::path::PathBuf;

    #[test]
    fn test_terminal_size_clamps_zero() {
        assert_eq!(
            TerminalSize::from_termion((0, 0)),
            TerminalSize {
                width: 1,
                height: 3
            }
        );
    }

    #[test]
    fn test_terminal_size_preserves_valid_dimensions() {
        assert_eq!(
            TerminalSize::from_termion((120, 40)),
            TerminalSize {
                width: 120,
                height: 40
            }
        );
    }

    #[test]
    fn test_terminal_size_clamps_small_height() {
        assert_eq!(
            TerminalSize::from_termion((80, 1)),
            TerminalSize {
                width: 80,
                height: 3
            }
        );
    }

    #[test]
    fn test_render_decision_message_only_for_pending_prefix_change() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.mode = Mode::Normal;
        after.handle_key(termion::event::Key::Char('g'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::MessageOnly);
    }

    #[test]
    fn test_render_decision_message_only_for_quit_prompt_change() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer.insert(0, "x");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer.insert(0, "x");
        after.mode = Mode::Command("q".to_string());
        after.handle_key(termion::event::Key::Char('\n'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::MessageOnly);
    }

    #[test]
    fn test_render_decision_none_for_noop_gg_when_already_at_top() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("hello");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("hello");
        after.handle_key(termion::event::Key::Char('g'));
        after.handle_key(termion::event::Key::Char('g'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::None);
    }

    #[test]
    fn test_render_decision_full_when_cursor_moves() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("ab");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("ab");
        after.handle_key(termion::event::Key::Char('l'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_layout_uses_minimum_gutter_digits() {
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, 9);
        assert_eq!(layout.gutter_digits, 3);
        assert_eq!(layout.gutter_total_width, 4);
        assert_eq!(layout.content_width, 76);
    }

    #[test]
    fn test_render_layout_expands_for_large_line_counts() {
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, 12_345);
        assert_eq!(layout.gutter_digits, 5);
        assert_eq!(layout.gutter_total_width, 6);
        assert_eq!(layout.content_width, 74);
    }

    #[test]
    fn test_render_layout_clamps_content_width_to_zero() {
        let size = TerminalSize {
            width: 2,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, 100);
        assert_eq!(layout.gutter_digits, 3);
        assert_eq!(layout.gutter_total_width, 4);
        assert_eq!(layout.content_width, 0);
    }
}
