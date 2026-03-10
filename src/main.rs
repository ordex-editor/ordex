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
mod text_buffer;
mod tui;
mod viewport;

use editor_state::{EditorRequest, EditorState};
use signal::SigwinchGuard;
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
    first_visible_row: usize,
    first_visible_column: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    mode_name: String,
    file_name: String,
    modified: bool,
    buffer_lines: usize,
    buffer_chars: usize,
    pending_prefix: Option<String>,
    input_prompt: Option<char>,
    input_line: Option<String>,
    input_cursor_col: Option<usize>,
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

/// One fully materialized screen row ready for terminal output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScreenRow {
    /// Source buffer line for this row, or `None` for EOF filler rows.
    line_idx: Option<usize>,
    /// Wrapped-row index within `line_idx`; `0` is the first screen row for a line.
    row_offset: usize,
    content: String,
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
            pending_prefix: editor.pending_prefix_label(),
            input_prompt: editor.input_prompt(),
            input_line: editor.input_line().map(|s| s.to_string()),
            input_cursor_col: editor.input_cursor_column(),
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
            || before.first_visible_row != after.first_visible_row
            || before.first_visible_column != after.first_visible_column
            || before.relative_line_numbers != after.relative_line_numbers
            || before.soft_wrap != after.soft_wrap
            || before.mode_name != after.mode_name
            || before.file_name != after.file_name
            || before.modified != after.modified
            || before.buffer_lines != after.buffer_lines
            || before.buffer_chars != after.buffer_chars
            || before.input_cursor_col != after.input_cursor_col;

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
    if selection_range.is_none() {
        return Cow::Borrowed(&row.content);
    }

    let line_start = editor.buffer.line_to_char(line_idx);
    let row_start = screen_row_start_column(editor, row, content_width);
    let mut rendered = String::new();
    let mut selected_active = false;

    // Reverse-video swaps foreground/background colors for selected text while
    // the real terminal cursor marks the active visual endpoint.
    for (offset, ch) in row.content.chars().enumerate() {
        let char_idx = line_start + row_start + offset;
        let selected = selection_range.is_some_and(|(start, end)| (start..end).contains(&char_idx));

        if selected_active != selected {
            if selected_active {
                rendered.push_str(&format!("{}", termion::style::Reset));
                selected_active = false;
            }
            if selected {
                rendered.push_str(&format!("{}", termion::style::Invert));
                selected_active = true;
            }
        }

        rendered.push(ch);
    }

    if selected_active {
        rendered.push_str(&format!("{}", termion::style::Reset));
    }

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
    let sigwinch = SigwinchGuard::install()?;

    // Initialize editor state with terminal height
    let mut editor = EditorState::new(terminal_size.height as usize);

    if let Some(outcome) = &config_outcome {
        editor.replace_config(&outcome.settings);
    }

    if let Some(path) = &cli_args.file_path {
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
    let mut batch = tui::TerminalBatch::new();
    batch.hide_cursor();

    // Reserve bottom 2 lines for status bar and command/message line
    let content_height = size.height.saturating_sub(2) as usize;
    let layout = RenderLayout::from_size(size, editor.buffer.lines_count());

    // Update viewport width
    editor.viewport.set_width(layout.content_width.max(1));
    editor
        .viewport
        .ensure_cursor_visible(&editor.cursor, &editor.buffer);

    // Screen rows are built first so rendering can share the same wrapped-row
    // traversal for content, gutter numbering, and EOF markers.
    let screen_rows = build_screen_rows(editor, content_height, layout.content_width);
    for (row, screen_row) in screen_rows.iter().enumerate() {
        let y = (row + 1) as u16;
        let gutter = format_screen_row_gutter(editor, screen_row, layout.gutter_digits);
        let content = render_row_content(editor, screen_row, layout.content_width);
        let line_len = (gutter.chars().count() + screen_row.content.chars().count()) as u16;
        batch.write_at(1, y, format_args!("{gutter}{content}"));
        if line_len < size.width {
            batch.write_at(1 + line_len, y, termion::clear::UntilNewline);
        }
    }

    render_status_line(&mut batch, editor, size);

    // Render command/message line (last line)
    write_message_line(&mut batch, editor, size);

    // Position cursor (accounting for scroll offsets)
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    let cursor_x = cursor_x.clamp(1, size.width);
    let cursor_y = cursor_y.clamp(1, size.height);
    batch.write_at(cursor_x, cursor_y, "");
    batch.show_cursor();
    term.write_batch(&batch)
}

/// Render the inverted status line that shows mode, file state, and cursor position.
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

    let status_left = format!(" {} | {}{}", mode_str, modified, file_name);
    let status_right = pos_str;
    // Fill the line between the left and right segments before applying the
    // inverted status-bar styling in one batched write.
    let padding = size
        .width
        .saturating_sub((status_left.len() + status_right.len()) as u16) as usize;
    let status_line = format!("{}{:padding$}{}", status_left, "", status_right);
    batch.write_at(
        1,
        status_y,
        format_args!(
            "{}{}{}",
            termion::style::Invert,
            &status_line[..status_line.len().min(size.width as usize)],
            termion::style::Reset
        ),
    );
}

/// Render only the command/message line while preserving the visible cursor.
fn render_message_line(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    if let (Some(prompt), Some(cursor_col)) = (editor.input_prompt(), editor.input_cursor_column())
    {
        batch.hide_cursor();
        write_message_line(&mut batch, editor, size);
        let input_x = 1 + prompt.len_utf8() + cursor_col.saturating_sub(1);
        batch.write_at((input_x as u16).clamp(1, size.width), size.height, "");
        batch.show_cursor();
        return term.write_batch(&batch);
    }

    // Save/restore keeps the user's visible cursor position stable while writing
    // to the bottom message row.
    batch.hide_cursor();
    batch.save_cursor();
    write_message_line(&mut batch, editor, size);
    batch.restore_cursor();
    batch.show_cursor();
    term.write_batch(&batch)
}

/// Queue the bottom command/message row into the current terminal batch.
fn write_message_line(batch: &mut tui::TerminalBatch, editor: &EditorState, size: TerminalSize) {
    let msg_y = size.height;
    batch.write_at(1, msg_y, termion::clear::CurrentLine);

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
            batch.write_at(1, msg_y, &left_text);
        }

        let marker_text: String = marker
            .chars()
            .rev()
            .take(marker_len)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        batch.write_at(marker_x, msg_y, &marker_text);
    } else if !left_message.is_empty() {
        batch.write_at(1, msg_y, &left_message);
    }
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
