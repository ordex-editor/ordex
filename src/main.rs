#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! This is the main entry point for the ordex text editor.
//! It handles CLI argument parsing, file loading, terminal initialization,
//! and the main event loop.

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
#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderSnapshot {
    cursor_line: usize,
    cursor_column: usize,
    first_visible_line: usize,
    first_visible_row: usize,
    first_visible_column: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    mode_name: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderDecision {
    /// Nothing visible changed; skip rendering to avoid unnecessary cursor blink.
    None,
    /// Only command/message-row state changed; update that row without full redraw.
    MessageOnly,
    /// Only the status line and cursor position changed on the same visible row.
    CursorOnly,
    /// Only the status line, cursor, and old/new cursor gutters changed.
    VerticalCursor,
    /// Cursor/content/status layout changed; perform full render.
    Full,
}

/// One fully materialized screen row ready for terminal output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScreenRow {
    /// Source buffer line for this row, or `None` for EOF filler rows.
    line_idx: Option<usize>,
    /// Wrapped-row index within `line_idx`; `0` is the first screen row for a line.
    row_offset: usize,
    content: String,
}

/// Materialized popup geometry in 1-based terminal coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PopupLayout {
    start_x: u16,
    start_y: u16,
    width: u16,
    height: u16,
}

impl PopupLayout {
    /// Return whether the popup covers the given 1-based terminal cell.
    fn covers(self, x: u16, y: u16) -> bool {
        let end_x = self.start_x.saturating_add(self.width.saturating_sub(1));
        let end_y = self.start_y.saturating_add(self.height.saturating_sub(1));
        (self.start_x..=end_x).contains(&x) && (self.start_y..=end_y).contains(&y)
    }
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
            first_visible_row: editor.viewport.first_visible_row(),
            first_visible_column: editor.viewport.first_visible_column(),
            relative_line_numbers: editor.relative_line_numbers_enabled(),
            soft_wrap: editor.soft_wrap_enabled(),
            mode_name: editor.mode_name().to_string(),
            file_name,
            modified: editor.buffer.is_modified(),
            buffer_lines: editor.buffer.lines_count(),
            buffer_chars: editor.buffer.chars_count(),
            syntax_generation: editor.syntax_generation(),
            theme_name: editor.theme_name(),
            pending_prefix: editor.pending_prefix_label(),
            input_prompt: editor.input_prompt(),
            input_line: editor.input_line().map(|s| s.to_string()),
            input_cursor_col: editor.input_cursor_column(),
            overwrite_prompt: editor.overwrite_prompt(),
            quit_prompt: editor.quit_prompt(),
            status_message: editor.status_message.clone(),
            sequence_discovery_popup: editor.sequence_discovery_popup(),
        }
    }

    /// Decide the minimal redraw required between two snapshots.
    ///
    /// Returns:
    /// - `Full` when viewport/status/cursor/content changed,
    /// - `VerticalCursor` when only a stable vertical cursor move changed the
    ///   active gutter rows, status line, and terminal cursor,
    /// - `CursorOnly` when only a same-line cursor move changed the status/cursor,
    /// - `MessageOnly` when only message-row state changed,
    /// - `None` when nothing visible changed.
    fn decide(before: &Self, after: &Self) -> RenderDecision {
        let same_viewport = before.first_visible_line == after.first_visible_line
            && before.first_visible_row == after.first_visible_row
            && before.first_visible_column == after.first_visible_column;
        let same_buffer = before.buffer_lines == after.buffer_lines
            && before.buffer_chars == after.buffer_chars
            && before.syntax_generation == after.syntax_generation;
        let same_surface = before.relative_line_numbers == after.relative_line_numbers
            && before.soft_wrap == after.soft_wrap
            && before.mode_name == after.mode_name
            && before.file_name == after.file_name
            && before.modified == after.modified
            && before.theme_name == after.theme_name
            && before.sequence_discovery_popup == after.sequence_discovery_popup;
        let message_changed = before.pending_prefix != after.pending_prefix
            || before.input_prompt != after.input_prompt
            || before.input_line != after.input_line
            || before.input_cursor_col != after.input_cursor_col
            || before.overwrite_prompt != after.overwrite_prompt
            || before.quit_prompt != after.quit_prompt
            || before.status_message != after.status_message;
        let paints_content_cursor =
            before.mode_name.starts_with("VISUAL") || before.mode_name == "V-LINE";
        let stable_frame_surface = same_viewport && same_buffer && same_surface;
        let visual_cursor_changed = before.cursor_line == after.cursor_line
            && before.cursor_column != after.cursor_column
            && paints_content_cursor;
        let cursor_only_changed = before.cursor_line == after.cursor_line
            && before.cursor_column != after.cursor_column
            && !visual_cursor_changed
            && stable_frame_surface
            && !message_changed;
        let vertical_cursor_changed = before.cursor_line != after.cursor_line
            && stable_frame_surface
            && !message_changed
            && !before.relative_line_numbers
            && before.sequence_discovery_popup.is_none()
            && after.sequence_discovery_popup.is_none()
            && !paints_content_cursor;

        // Vertical cursor moves only need to repaint the old/new cursor gutters
        // when the viewport and all other themed surfaces stay unchanged.
        if vertical_cursor_changed {
            return RenderDecision::VerticalCursor;
        }
        if cursor_only_changed {
            return RenderDecision::CursorOnly;
        }

        // Any content/layout/mode change beyond the targeted cursor motions can
        // affect the main viewport or themed surfaces, so it requires a full redraw.
        let full_changed = visual_cursor_changed
            || before.cursor_line != after.cursor_line
            || before.cursor_column != after.cursor_column
            || before.first_visible_line != after.first_visible_line
            || before.first_visible_row != after.first_visible_row
            || before.first_visible_column != after.first_visible_column
            || before.relative_line_numbers != after.relative_line_numbers
            || before.soft_wrap != after.soft_wrap
            || before.mode_name != after.mode_name
            || before.file_name != after.file_name
            || before.modified != after.modified
            || before.buffer_lines != after.buffer_lines
            || before.buffer_chars != after.buffer_chars
            || before.syntax_generation != after.syntax_generation
            || before.theme_name != after.theme_name
            || before.sequence_discovery_popup != after.sequence_discovery_popup;

        if full_changed {
            return RenderDecision::Full;
        }

        // If only prompt/message/prefix changed, redraw just the message row to
        // avoid unnecessary full-screen work and cursor repositioning.
        if message_changed {
            RenderDecision::MessageOnly
        } else {
            RenderDecision::None
        }
    }
}

/// Build the visible content rows for the current viewport.
///
/// A logical buffer line may produce multiple screen rows when soft wrap is
/// enabled, so the return value is row-based rather than line-based.
fn build_screen_rows(
    editor: &EditorState,
    content_height: usize,
    content_width: usize,
) -> Vec<ScreenRow> {
    if editor.soft_wrap_enabled() {
        return build_wrapped_screen_rows(editor, content_height, content_width);
    }

    build_unwrapped_screen_rows(editor, content_height, content_width)
}

/// Build screen rows for soft-wrapped rendering.
fn build_wrapped_screen_rows(
    editor: &EditorState,
    content_height: usize,
    content_width: usize,
) -> Vec<ScreenRow> {
    let mut rows = Vec::with_capacity(content_height);
    let width = content_width.max(1);
    let mut line_idx = editor.viewport.first_visible_line();
    let mut row_offset = editor.viewport.first_visible_row();

    // In wrapped mode one logical line can occupy several screen rows, so we
    // keep both the source line index and the row offset within that line.
    for _ in 0..content_height {
        if let Some(line) = editor.buffer.line_for_display(line_idx) {
            // `row_offset` identifies which wrapped slice of the line is visible.
            // Each row advances by `width` content columns, not terminal columns.
            let start = soft_wrap::row_start_column(row_offset, width);
            let content = line.chars().skip(start).take(width).collect::<String>();
            rows.push(ScreenRow {
                line_idx: Some(line_idx),
                row_offset,
                content,
            });

            let row_count = soft_wrap::wrap_row_count(line.chars().count(), width);
            if row_offset + 1 < row_count {
                row_offset += 1;
            } else {
                line_idx += 1;
                row_offset = 0;
            }
        } else {
            rows.push(ScreenRow {
                line_idx: None,
                row_offset: 0,
                content: String::new(),
            });
        }
    }

    rows
}

/// Build screen rows for non-wrapped rendering.
fn build_unwrapped_screen_rows(
    editor: &EditorState,
    content_height: usize,
    content_width: usize,
) -> Vec<ScreenRow> {
    let mut rows = Vec::with_capacity(content_height);
    let first_line = editor.viewport.first_visible_line();
    let first_col = editor.viewport.first_visible_column();
    for row in 0..content_height {
        let line_idx = first_line + row;
        if let Some(line) = editor.buffer.line_for_display(line_idx) {
            // In unwrapped mode every visible row corresponds to exactly one
            // logical line, so `row_offset` stays at 0 throughout.
            rows.push(ScreenRow {
                line_idx: Some(line_idx),
                row_offset: 0,
                content: line
                    .chars()
                    .skip(first_col)
                    .take(content_width)
                    .collect::<String>(),
            });
        } else {
            rows.push(ScreenRow {
                line_idx: None,
                row_offset: 0,
                content: String::new(),
            });
        }
    }

    rows
}

/// Format the gutter portion of one screen row.
fn format_screen_row_gutter(editor: &EditorState, row: &ScreenRow, gutter_digits: usize) -> String {
    match row.line_idx {
        Some(line_idx) if row.row_offset == 0 => {
            let number = editor.display_line_number(line_idx);
            format!("{number:>width$} ", width = gutter_digits)
        }
        Some(_) => format!("{:>width$} ", "", width = gutter_digits),
        None => format!("{:>width$} ", "~", width = gutter_digits),
    }
}

/// Return the starting buffer column for the visible content inside this row.
fn screen_row_start_column(editor: &EditorState, row: &ScreenRow, content_width: usize) -> usize {
    if editor.soft_wrap_enabled() {
        soft_wrap::row_start_column(row.row_offset, content_width.max(1))
    } else {
        editor.viewport.first_visible_column()
    }
}

/// Return the visible character offset of the active visual cursor cell on this row.
fn active_visual_cursor_offset(
    editor: &EditorState,
    row: &ScreenRow,
    content_width: usize,
) -> Option<usize> {
    if !editor.mode.is_visual() || row.line_idx != Some(editor.cursor.line()) {
        return None;
    }

    // Convert the logical cursor column into a row-local offset so wrapped rows
    // and horizontal scrolling share one visual-cursor path.
    let row_start = screen_row_start_column(editor, row, content_width);
    let offset = editor.cursor.column().checked_sub(row_start)?;

    // If the cursor is outside the visible slice, there is no cell to underline.
    (offset < row.content.chars().count()).then_some(offset)
}

/// Apply reverse-video highlighting to visible characters inside the active selection.
fn render_row_content<'a>(
    editor: &EditorState,
    row: &'a ScreenRow,
    content_width: usize,
) -> Cow<'a, str> {
    let Some(line_idx) = row.line_idx else {
        return Cow::Borrowed(&row.content);
    };

    let selection_range = editor.selection_range();
    let active_cursor = active_visual_cursor_offset(editor, row, content_width);
    let syntax_spans = editor.syntax_spans_for_line(line_idx);
    if selection_range.is_none() && active_cursor.is_none() && syntax_spans.is_empty() {
        return Cow::Borrowed(&row.content);
    }

    let line_start = editor.buffer.line_to_char(line_idx);
    let row_start = screen_row_start_column(editor, row, content_width);
    let mut rendered = String::new();
    let mut active_style = None;
    let mut span_idx = 0;
    let theme = editor.theme();
    let color_capability = editor.color_capability();

    // Reverse-video and underline must layer on top of syntax colors without
    // clobbering the current syntax span when wrapping or scrolling clips a row.
    for (offset, ch) in row.content.chars().enumerate() {
        let char_idx = line_start + row_start + offset;
        let column = row_start + offset;
        let underlined = active_cursor == Some(offset);
        let selected = !underlined
            && selection_range.is_some_and(|(start, end)| (start..end).contains(&char_idx));
        while span_idx < syntax_spans.len() && syntax_spans[span_idx].end_col <= column {
            span_idx += 1;
        }
        let syntax_span = syntax_spans
            .get(span_idx)
            .filter(|span| span.covers(column));
        let style = tui::CellStyle::from_syntax(
            syntax_span.map(|span| span.class),
            syntax_span.and_then(|span| span.modifier),
            selected,
            underlined,
        );
        tui::push_styled_char(
            &mut rendered,
            &mut active_style,
            style,
            theme,
            color_capability,
            ch,
        );
    }

    tui::finish_styled_output(&mut rendered, &mut active_style);
    Cow::Owned(rendered)
}

/// Return the screen-space cursor position for the current editor state.
fn cursor_screen_position(
    editor: &EditorState,
    layout: RenderLayout,
    content_height: usize,
    size: TerminalSize,
) -> (u16, u16) {
    if let (Some(prompt), Some(cursor_col)) = (editor.input_prompt(), editor.input_cursor_column())
    {
        // Input prompts temporarily own the cursor, so bypass viewport math and
        // place it directly on the message row.
        let input_x = 1 + prompt.len_utf8() + cursor_col.saturating_sub(1);
        return ((input_x as u16).clamp(1, size.width), size.height);
    }

    // Normal editing uses either wrapped or unwrapped cursor math depending on
    // whether one logical line may span several screen rows.
    if editor.soft_wrap_enabled() {
        wrapped_cursor_screen_position(editor, layout, content_height)
    } else {
        unwrapped_cursor_screen_position(editor, layout)
    }
}

/// Return the screen cursor position for wrapped rendering.
fn wrapped_cursor_screen_position(
    editor: &EditorState,
    layout: RenderLayout,
    content_height: usize,
) -> (u16, u16) {
    let line_len = editor.buffer.line_len(editor.cursor.line());
    // Convert the logical cursor into a visual row/column so rendering and
    // navigation share the same wrapped-layout interpretation.
    let cursor_visual = soft_wrap::visual_cursor(
        editor.cursor.column(),
        line_len,
        layout.content_width,
        editor.mode_uses_modal_bindings(),
        editor.cursor.line(),
    );
    let viewport_origin = soft_wrap::VisualPosition::new(
        editor.viewport.first_visible_line(),
        editor.viewport.first_visible_row(),
    );
    // The on-screen Y position is the number of wrapped rows between the
    // viewport origin and the cursor's wrapped row.
    let visual_row = soft_wrap::visual_rows_between(
        viewport_origin,
        cursor_visual.position,
        &editor.buffer,
        layout.content_width,
    );

    (
        // X is the gutter width plus the cursor's column inside its wrapped row.
        (layout.gutter_total_width + cursor_visual.column + 1) as u16,
        // Clamp to the last content row so the cursor never drops into the
        // status/message area even when the cursor sits just beyond the view.
        (visual_row.min(content_height.saturating_sub(1)) + 1) as u16,
    )
}

/// Return the screen cursor position for non-wrapped rendering.
fn unwrapped_cursor_screen_position(editor: &EditorState, layout: RenderLayout) -> (u16, u16) {
    (
        // In unwrapped mode the horizontal position is just the logical column
        // relative to the leftmost visible buffer column.
        (layout.gutter_total_width
            + editor
                .cursor
                .column()
                .saturating_sub(editor.viewport.first_visible_column())
            + 1) as u16,
        // Each logical line maps to exactly one screen row in unwrapped mode.
        (editor
            .cursor
            .line()
            .saturating_sub(editor.viewport.first_visible_line())
            + 1) as u16,
    )
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
    let cli_args = parse_cli_args(&args[1..])?;
    let config_path = cli_args.config_path.clone();
    let config_outcome = cli_args
        .config_path
        .as_deref()
        .map(|config_path| config::load_config(Path::new(config_path)));

    if let Some(outcome) = &config_outcome {
        config::emit_startup_warnings(&outcome.report.warnings);
        if should_emit_config_summary(outcome) {
            emit_config_summary(outcome);
        }
        if !outcome.report.warnings.is_empty() && should_pause_for_warnings() {
            wait_for_warning_ack()?;
        }
    }

    // Initialize terminal
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    let mut terminal_size = TerminalSize::from_termion(termion::terminal_size()?);
    let signals = SignalGuard::install()?;

    // Initialize editor state with terminal height
    let mut editor = EditorState::new(terminal_size.height as usize);
    editor.set_color_capability(detect_color_capability());

    if let Some(outcome) = &config_outcome {
        editor.replace_config(&outcome.settings);
    }

    if let Some(path) = &cli_args.file_path {
        if std::path::Path::new(path).exists() {
            editor.load_file(path)?;
        } else {
            // New file with specified name
            editor.file_path = std::path::PathBuf::from(path);
            editor.refresh_syntax();
        }
    }

    let mut key_log = init_key_log()?;

    let mut needs_render = true;
    let mut needs_message_render = false;
    let mut needs_cursor_render = false;
    let mut needs_vertical_cursor_render = None;
    // The discovery popup can temporarily hide the terminal cursor when it lands
    // on top of the logical cursor cell. Track that across redraws so we only
    // emit `Show`/`Hide` when the visibility state actually changes.
    let mut cursor_hidden_by_overlay = false;
    signals.mark_resize_pending();

    // Main event loop
    loop {
        // Honor termination before any redraw so the shell regains a restored
        // terminal instead of one more TUI frame.
        if signals.take_termination_signal().is_some() {
            break;
        }

        // Refresh terminal dimensions only when SIGWINCH arrives.
        if signals.take_resize_pending() {
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
            render_editor(
                &mut term,
                &mut editor,
                terminal_size,
                &mut cursor_hidden_by_overlay,
            )?;

            // Clear status message after displaying
            editor.status_message = None;
            needs_render = false;
            needs_message_render = false;
            needs_cursor_render = false;
            needs_vertical_cursor_render = None;
        } else if let Some(previous_cursor_line) = needs_vertical_cursor_render.take() {
            render_vertical_cursor_motion(
                &mut term,
                &editor,
                terminal_size,
                previous_cursor_line,
                &mut cursor_hidden_by_overlay,
            )?;
            needs_cursor_render = false;
        } else if needs_cursor_render {
            render_status_cursor(
                &mut term,
                &editor,
                terminal_size,
                &mut cursor_hidden_by_overlay,
            )?;
            needs_cursor_render = false;
        } else if needs_message_render {
            render_message_line(
                &mut term,
                &editor,
                terminal_size,
                &mut cursor_hidden_by_overlay,
            )?;
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
                handle_editor_request(&mut editor, config_path.as_deref());
                log_key_event(&mut key_log, key, before_mode, &editor);
                if editor.should_quit {
                    break;
                }
                let after = RenderSnapshot::capture(&editor);
                match RenderSnapshot::decide(&before, &after) {
                    RenderDecision::Full => {
                        needs_render = true;
                        needs_message_render = false;
                        needs_cursor_render = false;
                        needs_vertical_cursor_render = None;
                    }
                    RenderDecision::VerticalCursor => {
                        if !needs_render {
                            needs_vertical_cursor_render = Some(before.cursor_line);
                            needs_message_render = false;
                            needs_cursor_render = false;
                        }
                    }
                    RenderDecision::CursorOnly => {
                        if !needs_render && needs_vertical_cursor_render.is_none() {
                            needs_cursor_render = true;
                        }
                    }
                    RenderDecision::MessageOnly => {
                        if !needs_render
                            && needs_vertical_cursor_render.is_none()
                            && !needs_cursor_render
                        {
                            needs_message_render = true;
                        }
                    }
                    RenderDecision::None => {}
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                // Signals interrupt the blocking read; the next loop iteration
                // decides whether that means resize handling or termination.
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Run deferred editor requests that need process-level state from `run()`.
///
/// The editor parses commands while handling keys, but it deliberately does not
/// own CLI arguments or perform config-file I/O directly. `pending_request`
/// bridges that boundary: `EditorState` records "what should happen next", and
/// the main loop executes it once it has returned to the layer that owns the
/// active config path and other application-wide resources.
fn handle_editor_request(editor: &mut EditorState, config_path: Option<&str>) {
    match editor.take_pending_request() {
        Some(EditorRequest::ReloadConfig) => reload_editor_config(editor, config_path),
        None => {}
    }
}

/// Reload configuration from the active config path and apply it immediately.
fn reload_editor_config(editor: &mut EditorState, config_path: Option<&str>) {
    let Some(config_path) = config_path else {
        editor.status_message = Some("No config file to reload".to_string());
        return;
    };

    let outcome = config::load_config(Path::new(config_path));
    editor.replace_config(&outcome.settings);
    editor.status_message = Some(reload_status_message(&outcome));
}

#[derive(Debug, Default)]
struct CliArgs {
    file_path: Option<String>,
    config_path: Option<String>,
}

/// Parse supported CLI flags and positional arguments.
fn parse_cli_args(args: &[String]) -> io::Result<CliArgs> {
    let mut parsed = CliArgs::default();
    let mut idx = 0;
    while idx < args.len() {
        let current = &args[idx];
        if current == "--config" {
            let Some(next) = args.get(idx + 1) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Missing value for --config",
                ));
            };
            parsed.config_path = Some(next.clone());
            idx += 2;
            continue;
        }

        if let Some(value) = current.strip_prefix("--config=") {
            if value.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Missing value for --config",
                ));
            }
            parsed.config_path = Some(value.to_string());
            idx += 1;
            continue;
        }

        if parsed.file_path.is_none() {
            parsed.file_path = Some(current.clone());
        }
        idx += 1;
    }
    if parsed.config_path.is_none() && !env_flag_enabled("ORDEX_DISABLE_DEFAULT_CONFIG") {
        parsed.config_path =
            find_default_config_path().map(|path| path.to_string_lossy().into_owned());
    }
    Ok(parsed)
}

/// Resolve the default XDG config path and return it only when the file exists.
fn find_default_config_path() -> Option<PathBuf> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let candidate = resolve_default_config_path(xdg_config_home.as_deref(), home.as_deref())?;
    candidate.is_file().then_some(candidate)
}

/// Let users read startup warnings before entering the TUI screen.
fn wait_for_warning_ack() -> io::Result<()> {
    eprint!("Configuration warnings found. Press Enter to continue...");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

/// Return whether startup warning prompts should pause for user acknowledgement.
fn should_pause_for_warnings() -> bool {
    !env_flag_enabled("ORDEX_NO_WARNING_PAUSE")
}

/// Parse a boolean-like environment flag.
fn env_flag_enabled(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| {
        let normalized = value.to_string_lossy().trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
}

/// Print a human-readable startup summary for config loading.
fn emit_config_summary(outcome: &config::ConfigLoadOutcome) {
    let report = &outcome.report;
    let startup = if report.startup_allowed {
        "startup continues"
    } else {
        "startup blocked"
    };
    eprintln!(
        "Configuration loaded: {}.\n  Applied sections: {}\n  Skipped sections: {}\n  Defaults used: {}\n  Unknown settings ignored: {}\n  Warnings: {}",
        startup,
        report.applied_sections.len(),
        report.skipped_sections.len(),
        report.defaulted_keys.len(),
        report.ignored_unknown_keys.len(),
        report.warnings.len()
    );
}

/// Return whether config startup should print a summary banner.
fn should_emit_config_summary(outcome: &config::ConfigLoadOutcome) -> bool {
    let report = &outcome.report;
    !report.warnings.is_empty()
        || !report.skipped_sections.is_empty()
        || !report.defaulted_keys.is_empty()
        || !report.ignored_unknown_keys.is_empty()
        || !report.startup_allowed
}

/// Summarize runtime reload results in one TUI-safe status line.
fn reload_status_message(outcome: &config::ConfigLoadOutcome) -> String {
    match outcome.report.warnings.len() {
        0 => "Config reloaded".to_string(),
        1 => "Config reloaded with 1 warning".to_string(),
        count => format!("Config reloaded with {count} warnings"),
    }
}

/// Build the default config path from environment-derived directories.
fn resolve_default_config_path(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let base = if let Some(xdg) = xdg_config_home {
        xdg.to_path_buf()
    } else {
        home?.join(".config")
    };
    Some(base.join("ordex").join("config.cfg"))
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

/// Render only the status line and terminal cursor for same-line cursor motion.
fn render_status_cursor(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    render_status_line(&mut batch, editor, size);
    batch.set_cursor_shape(editor.cursor_shape());
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let content_height = size.height.saturating_sub(RESERVED_BOTTOM_ROWS) as usize;
    let layout = RenderLayout::from_size(size, editor.buffer.lines_count());
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    batch.goto(
        cursor_x.clamp(1, size.width),
        cursor_y.clamp(1, size.height),
    );
    term.write_batch(&batch)
}

/// Render the status line plus the gutters affected by a vertical cursor move.
fn render_vertical_cursor_motion(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
    previous_cursor_line: usize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let content_height = size.height.saturating_sub(RESERVED_BOTTOM_ROWS) as usize;
    let layout = RenderLayout::from_size(size, editor.buffer.lines_count());

    // Repaint the previous and new cursor gutters first so the active-line
    // styling updates without clearing the rest of the viewport.
    render_cursor_transition_gutters(
        &mut batch,
        editor,
        layout,
        content_height,
        previous_cursor_line,
    );
    render_status_line(&mut batch, editor, size);
    batch.set_cursor_shape(editor.cursor_shape());
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    batch.goto(
        cursor_x.clamp(1, size.width),
        cursor_y.clamp(1, size.height),
    );
    term.write_batch(&batch)
}

/// Repaint only the visible gutter rows touched by a vertical cursor transition.
fn render_cursor_transition_gutters(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    layout: RenderLayout,
    content_height: usize,
    previous_cursor_line: usize,
) {
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let screen_rows = build_screen_rows(editor, content_height, layout.content_width);
    for (row_index, screen_row) in screen_rows.iter().enumerate() {
        let Some(line_idx) = screen_row.line_idx else {
            continue;
        };
        if line_idx != previous_cursor_line && line_idx != editor.cursor.line() {
            continue;
        }

        // Only the old and new cursor gutters change on a stable vertical move,
        // so we can rewrite those cells without repainting the whole content row.
        let y = (row_index + 1) as u16;
        let gutter = format_screen_row_gutter(editor, screen_row, layout.gutter_digits);
        let gutter_style = theme.gutter_style(line_idx == editor.cursor.line());
        batch.write_styled_at(1, y, gutter_style, color_capability, &gutter);
    }
}

/// Detect terminal color capability from standard environment hints.
fn detect_color_capability() -> themes::ColorCapability {
    if env_flag_enabled("ORDEX_TRUECOLOR") {
        return themes::ColorCapability::TrueColor;
    }
    themes::detect_color_capability(
        env::var("COLORTERM").ok().as_deref(),
        env::var("TERM").ok().as_deref(),
    )
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
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let cursor_shape = editor.cursor_shape();
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let cursor_was_visible = !*cursor_hidden_by_overlay;

    // Hide the terminal cursor while a full frame is being repainted so large
    // themed redraws never show it stepping through intermediate row positions.
    if cursor_was_visible {
        batch.hide_cursor();
    }

    // Reserve bottom 2 lines for status bar and command/message line
    let content_height = size.height.saturating_sub(RESERVED_BOTTOM_ROWS) as usize;
    let layout = prepare_viewport_for_render(editor, size);

    // Screen rows are built first so rendering can share the same wrapped-row
    // traversal for content, gutter numbering, and EOF markers.
    let screen_rows = build_screen_rows(editor, content_height, layout.content_width);
    for (row, screen_row) in screen_rows.iter().enumerate() {
        let y = (row + 1) as u16;
        // Clear the row with the active theme background before repainting the
        // gutter and content. This preserves a fully themed backdrop without
        // streaming a full terminal-width run of spaces into every frame.
        batch.clear_to_eol_styled_at(1, y, theme.background_style(), color_capability);
        let gutter = format_screen_row_gutter(editor, screen_row, layout.gutter_digits);
        let content = render_row_content(editor, screen_row, layout.content_width);
        let gutter_width = gutter.chars().count() as u16;
        let gutter_style = if screen_row.line_idx.is_some() {
            theme.gutter_style(screen_row.line_idx == Some(editor.cursor.line()))
        } else {
            theme.eof_marker_style()
        };
        batch.write_styled_at(1, y, gutter_style, color_capability, &gutter);
        batch.write_at(1 + gutter_width, y, &content);
    }

    render_status_line(&mut batch, editor, size);

    // Render command/message line (last line)
    write_message_line(&mut batch, editor, size);
    let popup_layout = render_sequence_discovery_popup(&mut batch, editor, size);

    batch.set_cursor_shape(cursor_shape);

    // Position cursor (accounting for scroll offsets)
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    let cursor_x = cursor_x.clamp(1, size.width);
    let cursor_y = cursor_y.clamp(1, size.height);
    let cursor_covered_by_popup =
        popup_layout.is_some_and(|popup| popup.covers(cursor_x, cursor_y));
    if cursor_covered_by_popup {
        *cursor_hidden_by_overlay = true;
    } else {
        // Restore the cursor after the full redraw, whether this frame hid it
        // proactively or a previous popup frame had left it hidden.
        if *cursor_hidden_by_overlay || cursor_was_visible {
            batch.show_cursor();
            *cursor_hidden_by_overlay = false;
        }
        batch.goto(cursor_x, cursor_y);
    }
    term.write_batch(&batch)
}

/// Render the themed status line that shows mode, file state, and cursor position.
fn render_status_line(batch: &mut tui::TerminalBatch, editor: &EditorState, size: TerminalSize) {
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
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let width = size.width as usize;
    let mode_segment = format!(" {} ", mode_str);
    let left_rest = format!(" {}{}", modified, file_name);
    let mode_width = mode_segment.chars().count();
    let right_width = pos_str.chars().count().min(width);
    let show_right = width >= mode_width + 2 + right_width;

    batch.clear_to_eol_styled_at(1, status_y, theme.statusline_base_style(), color_capability);
    batch.write_styled_at(
        1,
        status_y,
        theme.statusline_mode_style(mode_str),
        color_capability,
        truncate_display_width(&mode_segment, width),
    );

    let left_rest_x = mode_segment.chars().count() as u16 + 1;
    let max_left_rest_width = if show_right {
        let right_x = size.width.saturating_sub(right_width as u16) + 1;
        right_x.saturating_sub(left_rest_x) as usize
    } else {
        width.saturating_sub(left_rest_x as usize).saturating_add(1)
    };
    if left_rest_x <= size.width && max_left_rest_width > 0 {
        batch.write_styled_at(
            left_rest_x,
            status_y,
            theme.statusline_base_style(),
            color_capability,
            truncate_display_width(&left_rest, max_left_rest_width),
        );
    }

    if show_right {
        let right_x = size.width.saturating_sub(right_width as u16) + 1;
        batch.write_styled_at(
            right_x,
            status_y,
            theme.statusline_base_style(),
            color_capability,
            truncate_display_width(&pos_str, right_width),
        );
    }
}

/// Render only the command/message line while preserving the visible cursor.
fn render_message_line(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let cursor_shape = editor.cursor_shape();
    if let (Some(prompt), Some(cursor_col)) = (editor.input_prompt(), editor.input_cursor_column())
    {
        write_message_line(&mut batch, editor, size);
        batch.set_cursor_shape(cursor_shape);
        if *cursor_hidden_by_overlay {
            batch.show_cursor();
            *cursor_hidden_by_overlay = false;
        }
        let input_x = 1 + prompt.len_utf8() + cursor_col.saturating_sub(1);
        batch.goto((input_x as u16).clamp(1, size.width), size.height);
        return term.write_batch(&batch);
    }

    // Message-only redraws can restore the editing cursor explicitly because the
    // viewport and cursor location are unchanged from the previous full render.
    // If an earlier popup frame hid the cursor, this redraw is also responsible
    // for making it visible again before repositioning it.
    write_message_line(&mut batch, editor, size);
    batch.set_cursor_shape(cursor_shape);
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let content_height = size.height.saturating_sub(RESERVED_BOTTOM_ROWS) as usize;
    let layout = RenderLayout::from_size(size, editor.buffer.lines_count());
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    batch.goto(
        cursor_x.clamp(1, size.width),
        cursor_y.clamp(1, size.height),
    );
    term.write_batch(&batch)
}

/// Queue the bottom command/message row into the current terminal batch.
fn write_message_line(batch: &mut tui::TerminalBatch, editor: &EditorState, size: TerminalSize) {
    let msg_y = size.height;

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
    batch.clear_to_eol_styled_at(
        1,
        msg_y,
        editor.theme().message_line_style(),
        editor.color_capability(),
    );
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
        let left_text = truncate_display_width(&left_message, max_left_len);
        if !left_text.is_empty() {
            batch.write_styled_at(
                1,
                msg_y,
                editor.theme().message_line_style(),
                editor.color_capability(),
                left_text,
            );
        }

        let marker_text = truncate_right_display_width(&marker, marker_len);
        batch.write_styled_at(
            marker_x,
            msg_y,
            editor.theme().pending_prefix_style(),
            editor.color_capability(),
            marker_text,
        );
    } else if !left_message.is_empty() {
        batch.write_styled_at(
            1,
            msg_y,
            editor.theme().message_line_style(),
            editor.color_capability(),
            truncate_display_width(&left_message, width),
        );
    }
}

/// Render the bottom-right shortcut discovery overlay when a sequence is pending.
fn render_sequence_discovery_popup(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    size: TerminalSize,
) -> Option<PopupLayout> {
    let Some(popup) = editor.sequence_discovery_popup() else {
        return None;
    };
    let content_height = size.height.saturating_sub(RESERVED_BOTTOM_ROWS) as usize;
    let lines = sequence_discovery_popup_lines(&popup, size.width as usize, content_height);
    let Some(box_width) = lines.first().map(|line| line.chars().count()) else {
        return None;
    };

    // Anchor the popup to the bottom-right of the content area so it stays above
    // the status and message rows without affecting cursor placement logic.
    let start_x = (size.width as usize).saturating_sub(box_width) + 1;
    let start_y = content_height.saturating_sub(lines.len()) + 1;
    for (index, line) in lines.iter().enumerate() {
        batch.write_styled_at(
            start_x as u16,
            (start_y + index) as u16,
            editor.theme().popup_style(),
            editor.color_capability(),
            line,
        );
    }
    Some(PopupLayout {
        start_x: start_x as u16,
        start_y: start_y as u16,
        width: box_width as u16,
        height: lines.len() as u16,
    })
}

/// Build a boxed shortcut-discovery popup, truncating to the visible terminal area.
fn sequence_discovery_popup_lines(
    popup: &SequenceDiscoveryPopup,
    max_width: usize,
    max_height: usize,
) -> Vec<String> {
    // The popup needs left/right borders plus at least one inner column, and it
    // also needs a top border, one body row, and a bottom border to stay legible.
    if max_width < POPUP_MIN_WIDTH || max_height < POPUP_MIN_HEIGHT {
        return Vec::new();
    }

    // The body lists only continuations because the typed prefix now lives in the
    // top border title, which keeps the popup compact.
    let mut body_lines = Vec::new();
    body_lines.extend(
        popup
            .entries
            .iter()
            .map(|entry| format!(" {}{}{} ", entry.keys, POPUP_ENTRY_GAP, entry.action)),
    );
    let visible_body_height = body_lines
        .len()
        .min(max_height.saturating_sub(POPUP_BORDER_INSET));
    let inner_width = body_lines
        .iter()
        .take(visible_body_height)
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        .max(popup.prefix.chars().count() + POPUP_TITLE_PADDING * 2)
        .min(max_width.saturating_sub(POPUP_BORDER_INSET));

    if inner_width == 0 {
        return Vec::new();
    }

    let mut lines = vec![popup_top_border(&popup.prefix, inner_width)];
    for line in body_lines.into_iter().take(visible_body_height) {
        let truncated = truncate_display_width(&line, inner_width);
        lines.push(format!(
            "{POPUP_VERTICAL}{truncated:<inner_width$}{POPUP_VERTICAL}"
        ));
    }
    lines.push(format!(
        "{POPUP_BOTTOM_LEFT}{}{POPUP_BOTTOM_RIGHT}",
        POPUP_HORIZONTAL.to_string().repeat(inner_width)
    ));
    lines
}

/// Build the popup's titled top border using Unicode box-drawing characters.
fn popup_top_border(title: &str, inner_width: usize) -> String {
    let available_title_width = inner_width.saturating_sub(POPUP_TITLE_PADDING * 2);
    let visible_title = truncate_display_width(title, available_title_width);
    let title_width = visible_title.chars().count();
    let left_fill = POPUP_TITLE_PADDING;
    let right_fill = inner_width.saturating_sub(left_fill + title_width);
    format!(
        "{POPUP_TOP_LEFT}{}{visible_title}{}{POPUP_TOP_RIGHT}",
        POPUP_HORIZONTAL.to_string().repeat(left_fill),
        POPUP_HORIZONTAL.to_string().repeat(right_fill)
    )
}

/// Truncate a string to at most `max_chars` Unicode scalar values without allocating.
fn truncate_display_width(input: &str, max_chars: usize) -> &str {
    if input.chars().count() <= max_chars {
        return input;
    }

    let end = input
        .char_indices()
        .nth(max_chars)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(input.len());
    &input[..end]
}

/// Keep only the last `max_chars` Unicode scalar values without allocating.
fn truncate_right_display_width(input: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    if input.chars().count() <= max_chars {
        return input;
    }

    let start = input
        .char_indices()
        .nth_back(max_chars - 1)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(0);
    &input[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use std::path::{Path, PathBuf};

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
    fn test_render_decision_full_for_sequence_popup_change() {
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
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_message_only_for_quit_prompt_change() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer.insert(0, "x");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer.insert(0, "x");
        after.mode = Mode::command_with_text("q");
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
    fn test_render_decision_cursor_only_when_cursor_moves_on_same_line() {
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
        assert_eq!(decision, RenderDecision::CursorOnly);
    }

    /// Verify that stable vertical motion uses the targeted gutter redraw path.
    #[test]
    fn test_render_decision_vertical_cursor_when_cursor_moves_lines_without_other_changes() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("a\nb");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("a\nb");

        // A plain `j` motion changes the active line but not the viewport.
        after.handle_key(termion::event::Key::Char('j'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::VerticalCursor);
    }

    /// Verify that relative line numbers still force a full redraw on vertical motion.
    #[test]
    fn test_render_decision_full_when_vertical_cursor_move_updates_relative_numbers() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("a\nb");
        before.apply_config(&crate::config::ConfigSettings {
            relative_line_numbers: Some(true),
            ..crate::config::ConfigSettings::default()
        });

        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("a\nb");
        after.apply_config(&crate::config::ConfigSettings {
            relative_line_numbers: Some(true),
            ..crate::config::ConfigSettings::default()
        });

        // Relative numbering changes every visible gutter when the cursor line moves.
        after.handle_key(termion::event::Key::Char('j'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_visual_cursor_moves_on_same_line() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("ab");
        before.handle_key(termion::event::Key::Char('v'));

        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("ab");
        after.handle_key(termion::event::Key::Char('v'));
        after.handle_key(termion::event::Key::Char('l'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_same_line_motion_clears_pending_prefix() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("abca");
        before.handle_key(termion::event::Key::Char('f'));

        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("abca");
        after.handle_key(termion::event::Key::Char('f'));
        after.handle_key(termion::event::Key::Char('a'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_relative_line_numbers_change() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("a\nb");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("a\nb");
        after.apply_config(&crate::config::ConfigSettings {
            relative_line_numbers: Some(true),
            ..crate::config::ConfigSettings::default()
        });

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_soft_wrap_changes() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("a.txt");
        before.buffer = crate::text_buffer::TextBuffer::from_str("abcdefghijklmnopqrstuvwxyz");
        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("a.txt");
        after.buffer = crate::text_buffer::TextBuffer::from_str("abcdefghijklmnopqrstuvwxyz");
        after.apply_config(&crate::config::ConfigSettings {
            soft_wrap: Some(false),
            ..crate::config::ConfigSettings::default()
        });

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_only_syntax_generation_changes() {
        let mut before = EditorState::new(24);
        before.file_path = PathBuf::from("sample.rs");
        before.buffer = crate::text_buffer::TextBuffer::from_str("fn main() {}\n");
        before.refresh_syntax();

        let mut after = EditorState::new(24);
        after.file_path = PathBuf::from("sample.rs");
        after.buffer = crate::text_buffer::TextBuffer::from_str("fn main() {}\n");
        after.refresh_syntax();
        after.refresh_syntax();

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_sequence_discovery_popup_lines_format_entries() {
        let popup = SequenceDiscoveryPopup {
            prefix: "g".to_string(),
            entries: vec![
                crate::editor_state::SequenceDiscoveryEntry {
                    keys: "g".to_string(),
                    action: "Move to first line".to_string(),
                },
                crate::editor_state::SequenceDiscoveryEntry {
                    keys: "$".to_string(),
                    action: "Move line end".to_string(),
                },
            ],
        };

        let lines = sequence_discovery_popup_lines(&popup, 80, 10);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("┌─g"));
        assert!(lines[0].ends_with('┐'));
        assert!(lines[1].contains("g  Move to first line"));
        assert!(lines[2].contains("$  Move line end"));
        assert!(lines[3].starts_with('└'));
        assert!(lines[3].ends_with('┘'));
    }

    #[test]
    fn test_sequence_discovery_popup_lines_respect_small_terminal_height() {
        let popup = SequenceDiscoveryPopup {
            prefix: "d".to_string(),
            entries: vec![
                crate::editor_state::SequenceDiscoveryEntry {
                    keys: "iw".to_string(),
                    action: "Delete inner word".to_string(),
                },
                crate::editor_state::SequenceDiscoveryEntry {
                    keys: "a(".to_string(),
                    action: "Delete around paren".to_string(),
                },
            ],
        };

        let lines = sequence_discovery_popup_lines(&popup, 40, 4);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains('d'));
        assert!(lines[1].contains("iw  Delete inner word"));
    }

    #[test]
    fn test_popup_layout_covers_cells_inside_box() {
        let popup = PopupLayout {
            start_x: 10,
            start_y: 5,
            width: 8,
            height: 4,
        };

        assert!(popup.covers(10, 5));
        assert!(popup.covers(17, 8));
        assert!(!popup.covers(9, 5));
        assert!(!popup.covers(18, 8));
    }

    #[test]
    fn test_truncate_display_width_returns_borrowed_prefix() {
        let input = "héllo";

        assert_eq!(truncate_display_width(input, 3), "hél");
        assert!(std::ptr::eq(
            truncate_display_width(input, 3).as_ptr(),
            input.as_ptr()
        ));
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

    /// Verify that insert-like modes map to the beam cursor.
    #[test]
    fn test_editor_cursor_shape_uses_beam_for_insert_like_modes() {
        let mut editor = EditorState::new(24);
        editor.handle_key(Key::Char('i'));
        assert_eq!(editor.cursor_shape(), tui::CursorShape::Beam);

        let mut command_editor = EditorState::new(24);
        command_editor.handle_key(Key::Char(':'));
        assert_eq!(command_editor.cursor_shape(), tui::CursorShape::Beam);

        let mut search_editor = EditorState::new(24);
        search_editor.handle_key(Key::Char('/'));
        assert_eq!(search_editor.cursor_shape(), tui::CursorShape::Beam);
    }

    /// Verify that normal and visual modes map to the block cursor.
    #[test]
    fn test_editor_cursor_shape_uses_block_for_normal_and_visual_modes() {
        let editor = EditorState::new(24);
        assert_eq!(editor.cursor_shape(), tui::CursorShape::Block);

        let mut visual_editor = EditorState::new(24);
        visual_editor.handle_key(Key::Char('v'));
        assert_eq!(visual_editor.cursor_shape(), tui::CursorShape::Block);
    }

    #[test]
    fn test_render_row_content_underlines_visual_cursor_without_inverting_it() {
        let mut editor = EditorState::new(24);
        editor.buffer = crate::text_buffer::TextBuffer::from_str("XYZ");
        editor.handle_key(termion::event::Key::Char('v'));
        editor.handle_key(termion::event::Key::Char('l'));

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "XYZ".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 10).into_owned();
        assert!(rendered.contains("\u{1b}[7mX"));
        assert!(rendered.contains("\u{1b}[4mY"));
        assert!(
            !rendered.contains("\u{1b}[7m\u{1b}[4mY"),
            "active visual cell should not stay inverted under the terminal cursor"
        );
    }

    #[test]
    fn test_render_row_content_visual_entry_uses_underline_without_invert() {
        let mut editor = EditorState::new(24);
        editor.buffer = crate::text_buffer::TextBuffer::from_str("XYZ");
        editor.handle_key(termion::event::Key::Char('v'));

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "XYZ".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 10).into_owned();
        assert!(rendered.contains("\u{1b}[4mX"));
        assert!(
            !rendered.contains("\u{1b}[7m\u{1b}[4mX"),
            "single-cell visual selection should keep the cursor cell uninverted"
        );
    }

    #[test]
    fn resolve_default_config_path_prefers_xdg_home() {
        let path = resolve_default_config_path(
            Some(Path::new("/tmp/custom-xdg")),
            Some(Path::new("/home/alice")),
        );
        assert_eq!(
            path,
            Some(PathBuf::from("/tmp/custom-xdg/ordex/config.cfg"))
        );
    }

    #[test]
    fn resolve_default_config_path_falls_back_to_home() {
        let path = resolve_default_config_path(None, Some(Path::new("/home/alice")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/alice/.config/ordex/config.cfg"))
        );
    }

    #[test]
    fn resolve_default_config_path_requires_base_directory() {
        assert_eq!(resolve_default_config_path(None, None), None);
    }
}
