//! Terminal rendering helpers and redraw decisions.

use crate::completion::CompletionPopup;
use crate::cursor::Cursor;
use crate::dialogs::{HoverPopup, PickerPopup, PickerPopupEntry, SignatureHelpPopup};
use crate::editor_state::{DiagnosticCounts, EditorState, SequenceDiscoveryPopup};
use crate::mode;
use crate::soft_wrap;
use crate::themes::ThemeStyle;
use crate::tui;
use std::borrow::Cow;
use std::io;

const MIN_GUTTER_DIGITS: usize = 3;
const GUTTER_MARKER_WIDTH: usize = 1;
const GUTTER_SEPARATOR_WIDTH: usize = 1;
const DIAGNOSTIC_GUTTER_DOT: char = '●';
const RESERVED_TOP_ROWS: u16 = 1;
const CONTENT_START_ROW: u16 = RESERVED_TOP_ROWS + 1;
const RESERVED_BOTTOM_ROWS: u16 = 2;
const MIN_TERMINAL_HEIGHT: u16 = RESERVED_TOP_ROWS + RESERVED_BOTTOM_ROWS + 1;
const MIN_WIDTH_FOR_TAB_MODIFIED_MARKER: usize = 40;
const POPUP_MIN_WIDTH: usize = 4;
const POPUP_MIN_HEIGHT: usize = 3;
const POPUP_BORDER_INSET: usize = 2;
const POPUP_TITLE_PADDING: usize = 1;
const POPUP_ENTRY_GAP: &str = "  ";
const POPUP_TOP_LEFT: char = '┌';
const POPUP_TOP_RIGHT: char = '┐';
const POPUP_LEFT_TEE: char = '├';
const POPUP_RIGHT_TEE: char = '┤';
const POPUP_BOTTOM_LEFT: char = '└';
const POPUP_BOTTOM_RIGHT: char = '┘';
const POPUP_HORIZONTAL: char = '─';
const POPUP_VERTICAL: char = '│';
const COMPLETION_POPUP_MAX_WIDTH: usize = 48;
const COMPLETION_POPUP_MAX_HEIGHT: usize = 20;
const COMPLETION_POPUP_MIN_PREFERRED_BELOW_ENTRIES: usize = 10;
const TEXT_POPUP_MAX_WIDTH: usize = 100;
const TEXT_POPUP_MAX_HEIGHT: usize = 16;
const TEXT_POPUP_MIN_PREFERRED_BELOW_LINES: usize = 6;
const TEXT_POPUP_HORIZONTAL_MARGIN: usize = 1;
const BUFFER_SWITCH_POPUP_MAX_WIDTH: usize = 84;
const BUFFER_SWITCH_POPUP_MAX_HEIGHT: usize = 20;
const BUFFER_SWITCH_POPUP_MIN_HEIGHT: usize = 5;
const BUFFER_SWITCH_POPUP_NON_RESULT_ROWS: usize = 4;
const BUFFER_SWITCH_POPUP_COMPACT_NON_RESULT_ROWS: usize = 3;
const BUFFER_SWITCH_POPUP_HORIZONTAL_MARGIN: usize = 4;
const BUFFER_SWITCH_POPUP_VERTICAL_MARGIN: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalSize {
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl TerminalSize {
    /// Clamp raw terminal dimensions to a small usable editing area.
    pub(crate) fn from_termion((width, height): (u16, u16)) -> Self {
        Self {
            width: width.max(1),
            height: height.max(MIN_TERMINAL_HEIGHT),
        }
    }

    /// Return the number of rows available for buffer content.
    fn content_height(self) -> usize {
        self.height
            .saturating_sub(RESERVED_TOP_ROWS + RESERVED_BOTTOM_ROWS) as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderLayout {
    gutter_digits: usize,
    gutter_total_width: usize,
    content_width: usize,
}

impl RenderLayout {
    fn from_size(size: TerminalSize, total_lines: usize) -> Self {
        let gutter_digits = total_lines.max(1).to_string().len().max(MIN_GUTTER_DIGITS);
        let gutter_total_width = GUTTER_MARKER_WIDTH + gutter_digits + GUTTER_SEPARATOR_WIDTH;
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
    let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
    editor.sync_viewport_width_for_render(layout.content_width.max(1));
    layout
}

/// Update editor viewport dimensions after a terminal resize.
pub(crate) fn resize_editor(editor: &mut EditorState, size: TerminalSize) {
    let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
    // Width tracks visible text columns, excluding the line-number gutter.
    editor.handle_resize(layout.content_width.max(1), size.height as usize);
}

/// Semantic mode identity used for redraw decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Normal,
    Visual(mode::VisualKind),
    Insert,
    Command,
    Search,
    BufferSwitch,
    FilePicker,
    SearchPicker,
    LocationPicker,
    DiagnosticPicker,
    CodeActionPicker,
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
            mode::Mode::BufferSwitch(_) => RenderMode::BufferSwitch,
            mode::Mode::FilePicker(_) => RenderMode::FilePicker,
            mode::Mode::SearchPicker(_) => RenderMode::SearchPicker,
            mode::Mode::LocationPicker(_) => RenderMode::LocationPicker,
            mode::Mode::DiagnosticPicker(_) => RenderMode::DiagnosticPicker,
            mode::Mode::CodeActionPicker(_) => RenderMode::CodeActionPicker,
        }
    }

    /// Return whether this mode paints the active cursor directly into content.
    ///
    /// Returns `true` when the cursor is rendered as part of the text content
    /// itself, and `false` when the terminal cursor remains the visible indicator.
    fn paints_content_cursor(self) -> bool {
        matches!(self, RenderMode::Visual(_))
    }
}

/// Snapshot of all editor state that can affect what the terminal must redraw.
///
/// This is used to avoid full-screen redraws when only the message line changed
/// (for example, when typing a sequence prefix like `g`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderSnapshot {
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
    read_only: bool,
    buffer_lines: usize,
    buffer_chars: usize,
    syntax_generation: u64,
    theme_name: &'static str,
    visible_match: Option<(usize, usize, usize, usize)>,
    visible_search_matches: Vec<(usize, usize)>,
    substitute_preview_revision: u64,
    cursor_diagnostic: Option<(crate::lsp::LspDiagnosticSeverity, String)>,
    diagnostic_counts: DiagnosticCounts,
    pending_prefix: Option<String>,
    input_prompt: Option<char>,
    input_line: Option<String>,
    input_cursor_col: Option<usize>,
    swap_recovery_prompt: Option<String>,
    soft_read_only_save_prompt: Option<String>,
    overwrite_prompt: Option<String>,
    quit_prompt: Option<String>,
    session_open_prompt: Option<String>,
    buffer_close_prompt: Option<String>,
    status_message: Option<String>,
    message_line_needs_clear: bool,
    status_overlay_needs_clear: bool,
    redraw_requested: bool,
    lsp_progress_lines: Vec<String>,
    sequence_discovery_popup: Option<SequenceDiscoveryPopup>,
    picker_popup: Option<PickerPopup>,
    completion_popup: Option<CompletionPopup>,
    hover_popup: Option<HoverPopup>,
    signature_help_popup: Option<SignatureHelpPopup>,
}

impl RenderSnapshot {
    /// Build a render snapshot from the current editor state.
    ///
    /// The snapshot contains only fields that affect terminal output so we can
    /// compare two states and choose the smallest valid redraw.
    pub(crate) fn capture(editor: &EditorState) -> Self {
        Self {
            cursor_line: editor.cursor_line(),
            cursor_column: editor.cursor_column(),
            first_visible_line: editor.first_visible_line(),
            first_visible_row: editor.first_visible_row(),
            first_visible_column: editor.first_visible_column(),
            relative_line_numbers: editor.relative_line_numbers_enabled(),
            soft_wrap: editor.soft_wrap_enabled(),
            mode: RenderMode::capture(editor.mode()),
            file_name: editor.file_name().to_string(),
            modified: editor.is_modified(),
            read_only: editor.is_read_only(),
            buffer_lines: editor.render_buffer_line_count(),
            buffer_chars: editor.render_buffer_char_count(),
            syntax_generation: editor.syntax_generation(),
            theme_name: editor.theme_name(),
            visible_match: editor.visible_match_snapshot(),
            visible_search_matches: editor.search_highlight_snapshot(),
            substitute_preview_revision: editor.substitute_preview_revision(),
            cursor_diagnostic: editor
                .cursor_diagnostic()
                .map(|diagnostic| (diagnostic.severity, diagnostic.message.clone())),
            diagnostic_counts: editor.active_diagnostic_counts(),
            pending_prefix: editor.pending_prefix_label(),
            input_prompt: editor.input_prompt(),
            input_line: editor.input_line().map(str::to_string),
            input_cursor_col: editor.input_cursor_column(),
            swap_recovery_prompt: editor.swap_recovery_prompt().map(str::to_string),
            soft_read_only_save_prompt: editor.soft_read_only_save_prompt(),
            overwrite_prompt: editor.overwrite_prompt(),
            quit_prompt: editor.quit_prompt(),
            session_open_prompt: editor.session_open_prompt(),
            buffer_close_prompt: editor.buffer_close_prompt(),
            status_message: editor.status_message().map(str::to_string),
            message_line_needs_clear: editor.message_line_needs_clear(),
            status_overlay_needs_clear: editor.status_overlay_needs_clear(),
            redraw_requested: editor.redraw_requested(),
            lsp_progress_lines: editor.lsp_progress_lines().to_vec(),
            sequence_discovery_popup: editor.sequence_discovery_popup(),
            picker_popup: editor.picker_popup(),
            completion_popup: editor.completion_popup(),
            hover_popup: editor.hover_popup().cloned(),
            signature_help_popup: editor.signature_help_popup().cloned(),
        }
    }

    /// Return the captured cursor line for targeted vertical redraw decisions.
    pub(crate) fn cursor_line(&self) -> usize {
        self.cursor_line
    }

    /// Return the captured single-line status message, if any.
    fn single_line_status_message(&self) -> Option<&str> {
        self.status_message
            .as_deref()
            .filter(|message| !message.contains('\n'))
    }

    /// Return the captured multi-line status overlay message, if any.
    fn multiline_status_message(&self) -> Option<&str> {
        self.status_message
            .as_deref()
            .filter(|message| message.contains('\n'))
    }

    /// Decide the minimal redraw required between two snapshots.
    ///
    /// Returns:
    /// - `Full` when viewport/status/cursor/content changed,
    /// - `VerticalCursor` when only a stable vertical cursor move changed the
    ///   active content rows, status line, and terminal cursor,
    /// - `CursorOnly` when only a same-line cursor move changed the status/cursor,
    /// - `MessageOnly` when only message-row state changed,
    /// - `None` when nothing visible changed.
    pub(crate) fn decide(before: &Self, after: &Self) -> RenderDecision {
        let same_viewport = before.first_visible_line == after.first_visible_line
            && before.first_visible_row == after.first_visible_row
            && before.first_visible_column == after.first_visible_column;
        let same_buffer = before.buffer_lines == after.buffer_lines
            && before.buffer_chars == after.buffer_chars
            && before.syntax_generation == after.syntax_generation;
        let same_surface = before.relative_line_numbers == after.relative_line_numbers
            && before.soft_wrap == after.soft_wrap
            && before.mode == after.mode
            && before.file_name == after.file_name
            && before.modified == after.modified
            && before.read_only == after.read_only
            && before.theme_name == after.theme_name
            && before.visible_match == after.visible_match
            && before.visible_search_matches == after.visible_search_matches
            && before.substitute_preview_revision == after.substitute_preview_revision
            && before.cursor_diagnostic == after.cursor_diagnostic
            && before.multiline_status_message() == after.multiline_status_message()
            && before.status_overlay_needs_clear == after.status_overlay_needs_clear
            && before.diagnostic_counts == after.diagnostic_counts
            && before.lsp_progress_lines == after.lsp_progress_lines
            && before.sequence_discovery_popup == after.sequence_discovery_popup
            && before.picker_popup == after.picker_popup
            && before.completion_popup == after.completion_popup
            && before.hover_popup == after.hover_popup
            && before.signature_help_popup == after.signature_help_popup;
        let message_changed = before.pending_prefix != after.pending_prefix
            || before.input_prompt != after.input_prompt
            || before.input_line != after.input_line
            || before.input_cursor_col != after.input_cursor_col
            || before.swap_recovery_prompt != after.swap_recovery_prompt
            || before.soft_read_only_save_prompt != after.soft_read_only_save_prompt
            || before.overwrite_prompt != after.overwrite_prompt
            || before.quit_prompt != after.quit_prompt
            || before.session_open_prompt != after.session_open_prompt
            || before.buffer_close_prompt != after.buffer_close_prompt
            || before.single_line_status_message() != after.single_line_status_message()
            || before.message_line_needs_clear
            || after.message_line_needs_clear;
        let overlay_changed = before.multiline_status_message() != after.multiline_status_message()
            || before.status_overlay_needs_clear != after.status_overlay_needs_clear;
        let paints_content_cursor = before.mode.paints_content_cursor();
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
            && before.completion_popup.is_none()
            && after.completion_popup.is_none()
            && before.hover_popup.is_none()
            && after.hover_popup.is_none()
            && before.signature_help_popup.is_none()
            && after.signature_help_popup.is_none()
            && !paints_content_cursor;

        // Vertical cursor moves only need to repaint the old/new cursor rows
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
            || before.mode != after.mode
            || before.file_name != after.file_name
            || before.modified != after.modified
            || before.read_only != after.read_only
            || before.buffer_lines != after.buffer_lines
            || before.buffer_chars != after.buffer_chars
            || before.syntax_generation != after.syntax_generation
            || before.theme_name != after.theme_name
            || before.visible_match != after.visible_match
            || before.visible_search_matches != after.visible_search_matches
            || before.substitute_preview_revision != after.substitute_preview_revision
            || before.cursor_diagnostic != after.cursor_diagnostic
            || overlay_changed
            || before.redraw_requested
            || after.redraw_requested
            || before.diagnostic_counts != after.diagnostic_counts
            || before.lsp_progress_lines != after.lsp_progress_lines
            || before.sequence_discovery_popup != after.sequence_discovery_popup
            || before.picker_popup != after.picker_popup
            || before.completion_popup != after.completion_popup
            || before.hover_popup != after.hover_popup
            || before.signature_help_popup != after.signature_help_popup;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderDecision {
    /// Nothing visible changed; skip rendering to avoid unnecessary cursor blink.
    None,
    /// Only command/message-row state changed; update that row without full redraw.
    MessageOnly,
    /// Only the status line and cursor position changed on the same visible row.
    CursorOnly,
    /// Only the status line, cursor, and old/new cursor rows changed.
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

    /// Return whether `self` overlaps `other` on the terminal grid.
    fn overlaps(self, other: Self) -> bool {
        let self_end_x = self.start_x.saturating_add(self.width.saturating_sub(1));
        let self_end_y = self.start_y.saturating_add(self.height.saturating_sub(1));
        let other_end_x = other.start_x.saturating_add(other.width.saturating_sub(1));
        let other_end_y = other.start_y.saturating_add(other.height.saturating_sub(1));
        self.start_x <= other_end_x
            && other.start_x <= self_end_x
            && self.start_y <= other_end_y
            && other.start_y <= self_end_y
    }
}

/// Materialized overlay rows in 1-based terminal coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OverlayLayout {
    rows: Vec<OverlayRow>,
}

/// One rendered overlay row used for exact cursor coverage checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayRow {
    start_x: u16,
    y: u16,
    width: u16,
}

impl OverlayLayout {
    /// Return whether the overlay writes over the given 1-based terminal cell.
    fn covers(&self, x: u16, y: u16) -> bool {
        self.rows.iter().any(|row| {
            let end_x = row.start_x.saturating_add(row.width.saturating_sub(1));
            row.y == y && (row.start_x..=end_x).contains(&x)
        })
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
    let mut line_idx = editor.first_visible_line();
    let mut row_offset = editor.first_visible_row();

    // In wrapped mode one logical line can occupy several screen rows, so we
    // keep both the source line index and the row offset within that line.
    for _ in 0..content_height {
        if let Some(line) = editor.render_buffer().line_for_display(line_idx) {
            // `row_offset` identifies which wrapped slice of the line is visible.
            // Each row advances by `width` content columns, not terminal columns.
            let start = soft_wrap::row_start_column(row_offset, width);
            let content = line.chars().skip(start).take(width).collect::<String>();
            rows.push(ScreenRow {
                line_idx: Some(line_idx),
                row_offset,
                content,
            });

            let row_count = soft_wrap::wrap_row_count(line.chars_count(), width);
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
    let first_line = editor.first_visible_line();
    let first_col = editor.first_visible_column();
    for row in 0..content_height {
        let line_idx = first_line + row;
        if let Some(line) = editor.render_buffer().line_for_display(line_idx) {
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

/// One preformatted gutter row split into marker and line-number segments.
struct ScreenRowGutter {
    marker: char,
    marker_severity: Option<crate::lsp::LspDiagnosticSeverity>,
    number_text: String,
}

/// Format the gutter segments for one screen row.
fn format_screen_row_gutter(
    editor: &EditorState,
    row: &ScreenRow,
    gutter_digits: usize,
) -> ScreenRowGutter {
    match row.line_idx {
        Some(line_idx) if row.row_offset == 0 => ScreenRowGutter {
            marker: editor
                .line_diagnostic_severity(line_idx)
                .map(|_| DIAGNOSTIC_GUTTER_DOT)
                .unwrap_or(' '),
            marker_severity: editor.line_diagnostic_severity(line_idx),
            number_text: format!(
                "{:>width$} ",
                editor.display_line_number(line_idx),
                width = gutter_digits
            ),
        },
        Some(_) => ScreenRowGutter {
            marker: ' ',
            marker_severity: None,
            number_text: format!("{:>width$} ", "", width = gutter_digits),
        },
        None => ScreenRowGutter {
            marker: ' ',
            marker_severity: None,
            number_text: format!("{:>width$} ", "~", width = gutter_digits),
        },
    }
}

/// Return the starting buffer column for the visible content inside this row.
fn screen_row_start_column(editor: &EditorState, row: &ScreenRow, content_width: usize) -> usize {
    if editor.soft_wrap_enabled() {
        soft_wrap::row_start_column(row.row_offset, content_width.max(1))
    } else {
        editor.first_visible_column()
    }
}

/// Return whether this visible row belongs to the cursor's logical line.
///
/// Returns `true` when the row displays the same logical line as the cursor and
/// `false` when it belongs to another line or an EOF filler row.
fn screen_row_is_current_line(editor: &EditorState, row: &ScreenRow) -> bool {
    row.line_idx == Some(editor.cursor_line())
}

/// Return the background style used to clear and pad one visible row.
fn row_background_style(editor: &EditorState, row: &ScreenRow) -> ThemeStyle {
    let mut style = editor.theme().background_style();
    if screen_row_is_current_line(editor, row) {
        style = style.overlay(editor.theme().current_line_style());
    }
    style
}

/// Render one visible screen row, including its gutter, content, and trailing space.
fn render_screen_row(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    layout: RenderLayout,
    screen_row: &ScreenRow,
    y: u16,
) {
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let gutter = format_screen_row_gutter(editor, screen_row, layout.gutter_digits);
    let content = render_row_content(editor, screen_row, layout.content_width);
    let number_style = screen_row
        .line_idx
        .map(|line_idx| theme.gutter_style(line_idx == editor.cursor_line()))
        .unwrap_or_else(|| theme.eof_marker_style());
    let marker_style = screen_row
        .line_idx
        .and_then(|line_idx| {
            gutter.marker_severity.map(|severity| {
                theme.diagnostic_marker_style(severity, line_idx == editor.cursor_line())
            })
        })
        .unwrap_or(number_style);

    // Clear first so trailing cells inherit the same current-line or buffer background.
    batch.clear_to_eol_styled_at(
        1,
        y,
        row_background_style(editor, screen_row),
        color_capability,
    );
    batch.write_styled_at(
        1,
        y,
        marker_style,
        color_capability,
        gutter.marker.to_string(),
    );
    batch.write_styled_at(2, y, number_style, color_capability, &gutter.number_text);
    batch.write_at(1 + layout.gutter_total_width as u16, y, &content);
    paint_trailing_cursor_cell(batch, editor, screen_row, layout, y);
}

/// Apply selection highlighting to visible characters inside the active selection.
fn render_row_content<'a>(
    editor: &EditorState,
    row: &'a ScreenRow,
    content_width: usize,
) -> Cow<'a, str> {
    let Some(line_idx) = row.line_idx else {
        return Cow::Borrowed(&row.content);
    };

    let has_selection = editor.selection_range().is_some()
        || matches!(editor.mode(), mode::Mode::Visual(mode::VisualKind::Block));
    let syntax_spans = editor.render_syntax_spans_for_line(line_idx);
    let current_line = screen_row_is_current_line(editor, row);
    if !has_selection
        && syntax_spans.is_empty()
        && !editor.line_has_visible_match(line_idx)
        && !editor.line_has_visible_search_match(line_idx)
        && !editor.line_has_visible_substitute_preview_match(line_idx)
        && editor.line_diagnostic_severity(line_idx).is_none()
    {
        return render_plain_row_content(editor, &row.content, current_line);
    }

    let line_start = editor.render_buffer().line_to_char(line_idx);
    let row_start = screen_row_start_column(editor, row, content_width);
    let mut rendered = String::new();
    let mut active_style = None;
    let mut span_idx = 0;
    let search_spans = editor.visible_search_match_spans(line_idx);
    let mut search_span_idx = 0;
    let preview_spans = editor.visible_substitute_preview_spans(line_idx);
    let mut preview_span_idx = 0;
    let theme = editor.theme();
    let color_capability = editor.color_capability();

    // Selection must layer on top of syntax colors without clobbering the
    // current syntax span when wrapping or scrolling clips a row.
    for (offset, ch) in row.content.chars().enumerate() {
        let char_idx = line_start + row_start + offset;
        let column = row_start + offset;
        let selected = editor.selection_contains_cell(line_idx, column);
        let match_role = editor.visible_match_role(char_idx);
        let diagnostic_severity = editor.diagnostic_severity_at_position(line_idx, column);
        while span_idx < syntax_spans.len() && syntax_spans[span_idx].end_col <= column {
            span_idx += 1;
        }
        // Visible search spans are stored in ascending column order, so once a
        // span ends at or before this column it cannot affect later cells.
        while search_span_idx < search_spans.len()
            && search_spans[search_span_idx].end_col <= column
        {
            search_span_idx += 1;
        }
        while preview_span_idx < preview_spans.len()
            && preview_spans[preview_span_idx].end_col <= column
        {
            preview_span_idx += 1;
        }
        let syntax_span = syntax_spans
            .get(span_idx)
            .filter(|span| span.covers(column));
        let search_match = search_spans
            .get(search_span_idx)
            .is_some_and(|span| span.covers(column))
            || preview_spans
                .get(preview_span_idx)
                .is_some_and(|span| span.covers(column));
        let style = tui::CellStyle::from_syntax(
            syntax_span.map(|span| span.class),
            syntax_span.and_then(|span| span.modifier),
            selected,
            current_line,
            match_role,
            search_match,
            diagnostic_severity,
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

/// Render one plain-text row with the active theme background and foreground.
fn render_plain_row_content<'a>(
    editor: &EditorState,
    content: &'a str,
    current_line: bool,
) -> Cow<'a, str> {
    if content.is_empty() {
        return Cow::Borrowed(content);
    }

    let mut rendered = String::with_capacity(content.len() + 32);
    let mut active_style = None;
    tui::push_styled_text(
        &mut rendered,
        &mut active_style,
        tui::CellStyle::from_syntax(None, None, false, current_line, None, false, None),
        editor.theme(),
        editor.color_capability(),
        content,
    );

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
    if let Some(popup) = editor.picker_popup() {
        let popup = layout_picker_popup(&popup, size);
        return (popup.cursor_x, popup.cursor_y);
    }

    if let (Some(prompt), Some(cursor_col)) = (editor.input_prompt(), editor.input_cursor_column())
    {
        // Input prompts temporarily own the cursor, so bypass viewport math and
        // place it directly on the message row.
        let input_x = 1 + prompt.len_utf8() + cursor_col.saturating_sub(1);
        return ((input_x as u16).clamp(1, size.width), size.height);
    }

    // Normal editing uses either wrapped or unwrapped cursor math depending on
    // whether one logical line may span several screen rows.
    buffer_screen_position(
        editor,
        layout,
        content_height,
        editor.cursor_line(),
        editor.cursor_column(),
    )
}

/// Return the screen position for one buffer location under the active wrap mode.
fn buffer_screen_position(
    editor: &EditorState,
    layout: RenderLayout,
    content_height: usize,
    line: usize,
    column: usize,
) -> (u16, u16) {
    if editor.soft_wrap_enabled() {
        wrapped_buffer_screen_position(editor, layout, content_height, line, column)
    } else {
        unwrapped_buffer_screen_position(editor, layout, line, column)
    }
}

/// Return the screen position for one wrapped buffer location.
fn wrapped_buffer_screen_position(
    editor: &EditorState,
    layout: RenderLayout,
    content_height: usize,
    line: usize,
    column: usize,
) -> (u16, u16) {
    let line_len = editor.buffer().line_len(line);
    // Convert the logical buffer location into a visual row/column so overlays
    // and the terminal cursor share the same wrapped-layout interpretation.
    let cursor_visual = soft_wrap::visual_cursor(
        column,
        line_len,
        layout.content_width,
        editor.mode_uses_modal_bindings(),
        line,
    );
    let viewport_origin =
        soft_wrap::VisualPosition::new(editor.first_visible_line(), editor.first_visible_row());
    // The on-screen Y position is the number of wrapped rows between the
    // viewport origin and the target location's wrapped row.
    let visual_row = soft_wrap::visual_rows_between(
        viewport_origin,
        cursor_visual.position,
        editor.buffer(),
        layout.content_width,
    );

    (
        // X is the gutter width plus the location's column inside its wrapped row.
        (layout.gutter_total_width + cursor_visual.column + 1) as u16,
        // Clamp to the last content row so the position never drops into the
        // status/message area even when it sits just beyond the view.
        (visual_row.min(content_height.saturating_sub(1)) as u16) + CONTENT_START_ROW,
    )
}

/// Return the screen position for one non-wrapped buffer location.
fn unwrapped_buffer_screen_position(
    editor: &EditorState,
    layout: RenderLayout,
    line: usize,
    column: usize,
) -> (u16, u16) {
    (
        // In unwrapped mode the horizontal position is the logical column
        // relative to the leftmost visible buffer column.
        (layout.gutter_total_width + column.saturating_sub(editor.first_visible_column()) + 1)
            as u16,
        // Each logical line maps to exactly one screen row in unwrapped mode.
        (line.saturating_sub(editor.first_visible_line()) as u16) + CONTENT_START_ROW,
    )
}

/// Return the effective terminal cursor color for the current editor state.
fn cursor_color(
    editor: &EditorState,
    cursor_shape: tui::CursorShape,
) -> Option<crate::themes::ThemeColor> {
    let theme = editor.theme();
    if cursor_shape == tui::CursorShape::Block && editor.cursor_on_visible_search_match() {
        return theme
            .search_match_style()
            .fg
            .or(theme.cursor_color(cursor_shape));
    }

    theme.cursor_color(cursor_shape)
}

/// Render only the status line and terminal cursor for same-line cursor motion.
pub(crate) fn render_status_cursor(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let cursor_shape = editor.cursor_shape();
    let color_capability = editor.color_capability();
    render_status_line(&mut batch, editor, size);
    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(cursor_color(editor, cursor_shape), color_capability);
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let content_height = size.content_height();
    let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    batch.goto(
        cursor_x.clamp(1, size.width),
        cursor_y.clamp(1, size.height),
    );
    term.write_batch(&batch)
}

/// Render the status line plus the rows affected by a vertical cursor move.
pub(crate) fn render_vertical_cursor_motion(
    term: &mut tui::Terminal,
    editor: &mut EditorState,
    size: TerminalSize,
    previous_cursor_line: usize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let cursor_was_visible = !*cursor_hidden_by_overlay;
    let content_height = size.content_height();
    let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
    let cursor_shape = editor.cursor_shape();
    let color_capability = editor.color_capability();
    editor.prepare_syntax_view(content_height);

    // Even this smaller multi-row update jumps through multiple screen rows, so
    // hide the cursor while the batch is being applied to avoid visible stepping.
    if cursor_was_visible {
        batch.hide_cursor();
    }

    // Repaint the previous and new cursor rows first so the active-line
    // styling updates without clearing the rest of the viewport.
    render_cursor_transition_rows(
        &mut batch,
        editor,
        layout,
        content_height,
        previous_cursor_line,
    );
    render_status_line(&mut batch, editor, size);
    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(cursor_color(editor, cursor_shape), color_capability);
    if *cursor_hidden_by_overlay || cursor_was_visible {
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

/// Repaint only the visible rows touched by a vertical cursor transition.
fn render_cursor_transition_rows(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    layout: RenderLayout,
    content_height: usize,
    previous_cursor_line: usize,
) {
    let screen_rows = build_screen_rows(editor, content_height, layout.content_width);
    for (row_index, screen_row) in screen_rows.iter().enumerate() {
        let Some(line_idx) = screen_row.line_idx else {
            continue;
        };
        if line_idx != previous_cursor_line && line_idx != editor.cursor_line() {
            continue;
        }

        // Current-line highlighting affects the full content row, so the old and
        // new logical cursor lines are repainted in place without redrawing the
        // rest of the viewport.
        let y = CONTENT_START_ROW + row_index as u16;
        render_screen_row(batch, editor, layout, screen_row, y);
    }
}

/// Return the visible trailing cursor-cell offset when the cursor sits past row content.
fn trailing_cursor_cell_offset(
    editor: &EditorState,
    row: &ScreenRow,
    content_width: usize,
) -> Option<usize> {
    if row.line_idx != Some(editor.cursor_line()) {
        return None;
    }
    let row_start = screen_row_start_column(editor, row, content_width);
    let cursor_col = editor.cursor_column();
    let content_len = row.content.chars().count();
    if cursor_col < row_start + content_len || cursor_col >= row_start + content_width {
        return None;
    }
    Some(cursor_col - row_start)
}

/// Paint a real themed space under the terminal cursor when no character exists there.
fn paint_trailing_cursor_cell(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    row: &ScreenRow,
    layout: RenderLayout,
    y: u16,
) {
    if let Some(offset) = trailing_cursor_cell_offset(editor, row, layout.content_width) {
        // Blank cursor cells need one explicit themed space so light-theme cursors
        // do not sit on an attribute-less empty area after line clears.
        batch.write_styled_at(
            (layout.gutter_total_width + offset + 1) as u16,
            y,
            row_background_style(editor, row),
            editor.color_capability(),
            " ",
        );
    }
}

/// Return one tab label for a buffer summary using the chosen detail level.
fn format_buffer_tab_label(
    summary: &crate::editor_state::BufferSummary,
    show_modified: bool,
) -> String {
    let path_label = trim_tab_path_label(&summary.display_path);
    let modified = if show_modified && summary.modified {
        "+"
    } else {
        ""
    };
    format!(" {path_label}{modified} ")
}

/// Return a compact tab label path like `s/t/module.rs` for one display path.
fn trim_tab_path_label(path_label: &str) -> String {
    // Scratch buffers already expose a compact synthetic label such as
    // `[No Name]`, so path compression would only make that UI text worse.
    if path_label.starts_with('[') {
        return path_label.to_string();
    }

    let path = std::path::Path::new(path_label);
    let mut components = path.components().peekable();
    let mut trimmed = String::new();
    while let Some(component) = components.next() {
        let part = component.as_os_str().to_string_lossy();
        if components.peek().is_none() {
            if !trimmed.is_empty() && !trimmed.ends_with(std::path::MAIN_SEPARATOR) {
                trimmed.push(std::path::MAIN_SEPARATOR);
            }
            trimmed.push_str(&part);
            break;
        }

        if trimmed.is_empty() && matches!(component, std::path::Component::RootDir) {
            trimmed.push(std::path::MAIN_SEPARATOR);
            continue;
        }
        if !trimmed.is_empty() && !trimmed.ends_with(std::path::MAIN_SEPARATOR) {
            trimmed.push(std::path::MAIN_SEPARATOR);
        }
        if let Some(ch) = part.chars().next() {
            trimmed.push(ch);
        }
    }

    if trimmed.is_empty() {
        path_label.to_string()
    } else {
        trimmed
    }
}

/// Return the terminal width consumed by one contiguous tab range plus edge markers.
fn tab_range_width(
    summaries: &[crate::editor_state::BufferSummary],
    start: usize,
    end: usize,
    show_modified: bool,
) -> usize {
    let left_marker = usize::from(start > 0);
    let right_marker = usize::from(end + 1 < summaries.len());
    let tab_width = summaries[start..=end]
        .iter()
        .map(|summary| {
            format_buffer_tab_label(summary, show_modified)
                .chars()
                .count()
        })
        .sum::<usize>();
    let piece_count = end + 1 - start + left_marker + right_marker;
    let separators = piece_count.saturating_sub(1);
    let marker_width = (left_marker + right_marker) * " ... ".chars().count();
    tab_width + separators + marker_width
}

/// Return the widest contiguous tab range that fits while keeping the active tab visible.
fn visible_tab_range(
    summaries: &[crate::editor_state::BufferSummary],
    width: usize,
    show_modified: bool,
) -> (usize, usize) {
    let active_index = summaries
        .iter()
        .position(|summary| summary.active)
        .unwrap_or(0);
    let mut start = active_index;
    let mut end = active_index;

    // Expand around the active tab so narrow terminals still keep context near the
    // current buffer while preserving the original open-buffer ordering.
    loop {
        let mut best_candidate = None;
        if start > 0 {
            let candidate_width = tab_range_width(summaries, start - 1, end, show_modified);
            if candidate_width <= width {
                best_candidate = Some((start - 1, end, candidate_width));
            }
        }
        if end + 1 < summaries.len() {
            let candidate_width = tab_range_width(summaries, start, end + 1, show_modified);
            if candidate_width <= width {
                match best_candidate {
                    Some((_, _, best_width)) if best_width <= candidate_width => {}
                    _ => best_candidate = Some((start, end + 1, candidate_width)),
                }
            }
        }

        let Some((next_start, next_end, _)) = best_candidate else {
            break;
        };
        start = next_start;
        end = next_end;
    }

    (start, end)
}

/// Render the persistent top-row tab strip that lists open buffers in order.
fn render_tab_line(batch: &mut tui::TerminalBatch, editor: &EditorState, size: TerminalSize) {
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let width = size.width as usize;
    let summaries = editor.buffer_summaries();
    let strip_style = theme.statusline_base_style();
    let active_style = theme.statusline_mode_style("NORMAL");
    let separator = "|";
    let ellipsis = " ... ";
    let show_modified = width >= MIN_WIDTH_FOR_TAB_MODIFIED_MARKER;
    let (start, end) = visible_tab_range(&summaries, width, show_modified);
    let mut pieces: Vec<(String, bool)> = Vec::new();
    if start > 0 {
        pieces.push((ellipsis.to_string(), false));
    }
    for summary in &summaries[start..=end] {
        pieces.push((
            format_buffer_tab_label(summary, show_modified),
            summary.active,
        ));
    }
    if end + 1 < summaries.len() {
        pieces.push((ellipsis.to_string(), false));
    }

    batch.clear_to_eol_styled_at(1, 1, strip_style, color_capability);
    let mut x = 1u16;

    // Tabs are written piece-by-piece so the active buffer can use accent styling
    // while the full strip still inherits the shared chrome background.
    for (index, (piece, active)) in pieces.iter().enumerate() {
        if x > size.width {
            break;
        }
        let remaining = width.saturating_sub((x - 1) as usize);
        let visible = truncate_display_width(piece, remaining);
        if !visible.is_empty() {
            batch.write_styled_at(
                x,
                1,
                if *active { active_style } else { strip_style },
                color_capability,
                visible,
            );
            x += visible.chars().count() as u16;
        }
        if index + 1 == pieces.len() || x > size.width {
            continue;
        }

        let remaining = width.saturating_sub((x - 1) as usize);
        let visible_separator = truncate_display_width(separator, remaining);
        if visible_separator.is_empty() {
            break;
        }
        batch.write_styled_at(x, 1, strip_style, color_capability, visible_separator);
        x += visible_separator.chars().count() as u16;
    }
}

/// Render the editor state to the terminal.
pub(crate) fn render_editor(
    term: &mut tui::Terminal,
    editor: &mut EditorState,
    size: TerminalSize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let cursor_shape = editor.cursor_shape();
    let color_capability = editor.color_capability();
    let cursor_was_visible = !*cursor_hidden_by_overlay;

    // Hide the terminal cursor while a full frame is being repainted so large
    // themed redraws never show it stepping through intermediate row positions.
    if cursor_was_visible {
        batch.hide_cursor();
    }

    let content_height = size.content_height();
    let layout = prepare_viewport_for_render(editor, size);
    editor.prepare_syntax_view(content_height);
    render_tab_line(&mut batch, editor, size);

    // Screen rows are built first so rendering can share the same wrapped-row
    // traversal for content, gutter numbering, and EOF markers.
    let screen_rows = build_screen_rows(editor, content_height, layout.content_width);
    for (row, screen_row) in screen_rows.iter().enumerate() {
        let y = CONTENT_START_ROW + row as u16;
        render_screen_row(&mut batch, editor, layout, screen_row, y);
    }

    render_top_right_overlays(&mut batch, editor, size, layout);
    render_status_line(&mut batch, editor, size);
    write_message_line(&mut batch, editor, size);
    let progress_layout = render_lsp_progress_overlay(&mut batch, editor, size);
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    let cursor_x = cursor_x.clamp(1, size.width);
    let cursor_y = cursor_y.clamp(1, size.height);
    let picker_popup = editor.picker_popup();
    let completion_popup = editor.completion_popup();
    let hover_popup = editor.hover_popup();
    let signature_help_popup = editor.signature_help_popup();
    let popup_layouts = if let Some(popup) = picker_popup.as_ref() {
        vec![render_picker_popup(&mut batch, popup, editor, size)]
    } else if completion_popup.is_some() || signature_help_popup.is_some() {
        let mut layouts = Vec::new();
        let completion_rendered = completion_popup
            .as_ref()
            .and_then(|popup| completion_popup_layout(popup, editor, size, layout, content_height));
        let signature_rendered = if let Some(popup) = signature_help_popup {
            let forced_side = completion_rendered.as_ref().map(|layout| {
                if layout.layout.start_y > cursor_y {
                    TextPopupSide::Above
                } else {
                    TextPopupSide::Below
                }
            });
            signature_help_popup_layout(
                popup,
                editor,
                size,
                layout,
                cursor_y,
                content_height,
                forced_side,
            )
            .or_else(|| {
                // When both popups cannot coexist, keep signature help visible and
                // drop completion so the active call context still has priority.
                signature_help_popup_layout(
                    popup,
                    editor,
                    size,
                    layout,
                    cursor_y,
                    content_height,
                    None,
                )
            })
        } else {
            None
        };
        let suppress_completion = signature_help_popup.is_some()
            && completion_rendered.is_some()
            && signature_rendered.is_some()
            && completion_rendered.as_ref().is_some_and(|completion| {
                signature_rendered
                    .as_ref()
                    .is_some_and(|signature| completion.layout.overlaps(signature.layout))
            });
        if let Some(rendered) = completion_rendered.as_ref()
            && !suppress_completion
        {
            layouts.push(render_completion_popup(&mut batch, rendered, editor));
        }
        if let (Some(popup), Some(rendered)) = (signature_help_popup, signature_rendered.as_ref()) {
            layouts.push(render_signature_help_popup(
                &mut batch, popup, rendered, editor,
            ));
        }
        layouts
    } else if let Some(popup) = hover_popup {
        render_hover_popup(
            &mut batch,
            popup,
            editor,
            size,
            cursor_x,
            cursor_y,
            content_height,
        )
        .into_iter()
        .collect()
    } else {
        render_sequence_discovery_popup(&mut batch, editor, size)
            .into_iter()
            .collect()
    };

    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(cursor_color(editor, cursor_shape), color_capability);

    // Position cursor after all content so overlays can decide whether it must hide.
    let cursor_covered_by_popup = picker_popup.is_none()
        && (progress_layout.is_some_and(|popup| popup.covers(cursor_x, cursor_y))
            || popup_layouts
                .iter()
                .any(|popup| popup.covers(cursor_x, cursor_y)));
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

/// Render the bottom-right LSP progress overlay and return its covered area.
fn render_lsp_progress_overlay(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    size: TerminalSize,
) -> Option<OverlayLayout> {
    let lines = lsp_progress_overlay_lines(
        editor.lsp_progress_lines(),
        size.width as usize,
        size.content_height(),
    );
    let box_height = lines.len();
    if box_height == 0 {
        return None;
    }
    let content_bottom = size.height.saturating_sub(RESERVED_BOTTOM_ROWS);
    let start_y = content_bottom
        .saturating_sub(box_height as u16)
        .saturating_add(1);
    let popup_style = editor.theme().popup_style();
    let overlay_style = editor.theme().background_style().overlay(ThemeStyle {
        fg: popup_style.fg,
        bg: None,
        bold: popup_style.bold,
        underline: popup_style.underline,
        undercurl: false,
        reverse: false,
    });
    let mut rows = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let width = line.chars().count() as u16;
        let start_x = size.width.saturating_sub(width).saturating_add(1);
        rows.push(OverlayRow {
            start_x,
            y: start_y + index as u16,
            width,
        });
        batch.write_styled_at(
            start_x,
            start_y + index as u16,
            overlay_style,
            editor.color_capability(),
            line,
        );
    }
    Some(OverlayLayout { rows })
}

/// Return the laid out completion popup for the current frame.
fn completion_popup_layout(
    popup: &CompletionPopup,
    editor: &EditorState,
    size: TerminalSize,
    layout: RenderLayout,
    content_height: usize,
) -> Option<CompletionPopupLayout> {
    let anchor_cursor = Cursor::from_char_index(
        editor.buffer(),
        popup.anchor_char_idx.min(editor.buffer().chars_count()),
    );
    // Convert the saved buffer anchor back into screen coordinates each frame so
    // the popup stays visually stable while still respecting scrolling and wrap.
    let (anchor_x, anchor_y) = buffer_screen_position(
        editor,
        layout,
        content_height,
        anchor_cursor.line(),
        anchor_cursor.column(),
    );
    let rendered = layout_completion_popup(
        popup,
        size,
        anchor_x.clamp(1, size.width),
        anchor_y.clamp(1, size.height),
    )?;
    Some(rendered)
}

/// Render one already laid out completion popup and return its covered area.
fn render_completion_popup(
    batch: &mut tui::TerminalBatch,
    rendered: &CompletionPopupLayout,
    editor: &EditorState,
) -> PopupLayout {
    let popup_style = editor.theme().popup_style();
    let selected_style = popup_style.overlay(editor.theme().selection_style());
    for (index, line) in rendered.lines.iter().enumerate() {
        batch.write_styled_at(
            rendered.layout.start_x,
            rendered.layout.start_y + index as u16,
            if line.selected {
                selected_style
            } else {
                popup_style
            },
            editor.color_capability(),
            &line.text,
        );
    }
    rendered.layout
}

/// Render the cursor-anchored hover popup and return its covered area.
fn render_hover_popup(
    batch: &mut tui::TerminalBatch,
    popup: &HoverPopup,
    editor: &EditorState,
    size: TerminalSize,
    cursor_x: u16,
    cursor_y: u16,
    content_height: usize,
) -> Option<PopupLayout> {
    let rendered = layout_hover_popup(popup, size, cursor_x, cursor_y, content_height)?;
    let popup_style = editor.theme().popup_style();
    for (index, line) in rendered.lines.iter().enumerate() {
        batch.write_styled_at(
            rendered.layout.start_x,
            rendered.layout.start_y + index as u16,
            popup_style,
            editor.color_capability(),
            &line.text,
        );
    }
    Some(rendered.layout)
}

/// Return the laid out signature-help popup for the current frame.
fn signature_help_popup_layout(
    popup: &SignatureHelpPopup,
    editor: &EditorState,
    size: TerminalSize,
    layout: RenderLayout,
    cursor_y: u16,
    content_height: usize,
    forced_side: Option<TextPopupSide>,
) -> Option<TextPopupLayout> {
    let anchor_cursor = Cursor::from_char_index(
        editor.buffer(),
        popup.anchor_char_idx.min(editor.buffer().chars_count()),
    );
    let (anchor_x, _) = buffer_screen_position(
        editor,
        layout,
        content_height,
        anchor_cursor.line(),
        anchor_cursor.column(),
    );
    layout_text_popup(
        &popup.title,
        signature_help_popup_line_iter(popup),
        size,
        anchor_x.clamp(1, size.width),
        cursor_y,
        content_height,
        forced_side,
    )
}

/// Render one already laid out signature-help popup and return its covered area.
fn render_signature_help_popup(
    batch: &mut tui::TerminalBatch,
    popup: &SignatureHelpPopup,
    rendered: &TextPopupLayout,
    editor: &EditorState,
) -> PopupLayout {
    let popup_style = editor.theme().popup_style();
    let highlight_style = popup_style.overlay(editor.theme().selection_style());
    for (index, line) in rendered.lines.iter().enumerate() {
        let y = rendered.layout.start_y + index as u16;
        // The border rows use the plain popup style. Only the wrapped signature
        // body rows may carry an active-parameter highlight.
        if line.source_line_index.is_none() {
            batch.write_styled_at(
                rendered.layout.start_x,
                y,
                popup_style,
                editor.color_capability(),
                &line.text,
            );
            continue;
        }
        // Signature help uses the same boxed text-popup layout as hover, but it
        // overlays the server-selected active parameter with a contrasting
        // background so the current argument stays visible while typing.
        let highlight_range = signature_help_highlight_range(popup, line);
        write_popup_highlighted_body_range(
            batch,
            (rendered.layout.start_x, y),
            &line.text,
            popup_style,
            highlight_style,
            highlight_range,
            editor.color_capability(),
        );
    }
    rendered.layout
}

/// Return the ordered signature-help body lines shown inside the popup.
fn signature_help_popup_line_iter<'a>(
    popup: &'a SignatureHelpPopup,
) -> impl Iterator<Item = TextPopupSourceLine<'a>> + 'a {
    std::iter::once(TextPopupSourceLine {
        text: popup.signature_line.as_str(),
        source_line_index: 0,
    })
    .chain(
        popup
            .documentation_lines
            .iter()
            .enumerate()
            .map(|(index, line)| TextPopupSourceLine {
                text: line.as_str(),
                source_line_index: index + 1,
            }),
    )
}

/// Return the highlighted character range inside one visible signature-help body line.
fn signature_help_highlight_range(
    popup: &SignatureHelpPopup,
    line: &TextPopupLine,
) -> Option<(usize, usize)> {
    if line.source_line_index != Some(0) {
        return None;
    }
    let (highlight_start, highlight_end) = popup.active_parameter_range?;
    let line_end = line.start_char.saturating_add(line.text.chars().count());
    let visible_start = highlight_start.max(line.start_char);
    let visible_end = highlight_end.min(line_end);
    (visible_start < visible_end).then_some((
        visible_start.saturating_sub(line.start_char),
        visible_end.saturating_sub(line.start_char),
    ))
}

/// Render one popup row with a highlighted character range inside its body text.
fn write_popup_highlighted_body_range(
    batch: &mut tui::TerminalBatch,
    position: (u16, u16),
    line: &str,
    popup_style: ThemeStyle,
    highlight_style: ThemeStyle,
    highlight_range: Option<(usize, usize)>,
    color_capability: crate::themes::ColorCapability,
) {
    let (start_x, y) = position;
    let Some((highlight_start, highlight_end)) = highlight_range else {
        batch.write_styled_at(start_x, y, popup_style, color_capability, line);
        return;
    };
    let Some(segments) = split_popup_border_segments(line) else {
        batch.write_styled_at(start_x, y, popup_style, color_capability, line);
        return;
    };
    let prefix = slice_display_width(segments.body, 0, highlight_start);
    let highlight = slice_display_width(
        segments.body,
        highlight_start,
        highlight_end - highlight_start,
    );
    let suffix_start = highlight_start + highlight.chars().count();
    let suffix = slice_display_width(
        segments.body,
        suffix_start,
        segments.body.chars().count().saturating_sub(suffix_start),
    );
    batch.write_styled_at(
        start_x,
        y,
        popup_style,
        color_capability,
        segments.left_border,
    );
    batch.write_styled_at(
        start_x + segments.left_border.chars().count() as u16,
        y,
        popup_style,
        color_capability,
        prefix,
    );
    batch.write_styled_at(
        start_x + segments.left_border.chars().count() as u16 + prefix.chars().count() as u16,
        y,
        highlight_style,
        color_capability,
        highlight,
    );
    batch.write_styled_at(
        start_x
            + segments.left_border.chars().count() as u16
            + prefix.chars().count() as u16
            + highlight.chars().count() as u16,
        y,
        popup_style,
        color_capability,
        suffix,
    );
    batch.write_styled_at(
        start_x
            + segments.left_border.chars().count() as u16
            + segments.body.chars().count() as u16,
        y,
        popup_style,
        color_capability,
        segments.right_border,
    );
}

/// Render the themed status line that shows mode, file state, and cursor position.
fn render_status_line(batch: &mut tui::TerminalBatch, editor: &EditorState, size: TerminalSize) {
    let status_y = size.height - 1;
    let mode_str = editor.mode_name();
    let pos_str = format!(
        "{}:{} ",
        editor.cursor_line() + 1,
        editor.cursor_column() + 1
    );
    let file_state_segments = build_statusline_file_state_segments(editor);
    let file_state_width = file_state_segments
        .iter()
        .map(StatusLineSegment::display_width)
        .sum::<usize>();
    let diagnostic_segments = build_statusline_diagnostic_segments(editor);
    let diagnostic_width = diagnostic_segments
        .iter()
        .map(StatusLineSegment::display_width)
        .sum::<usize>();
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let width = size.width as usize;
    let mode_segment = format!(" {} ", mode_str);
    let mode_width = mode_segment.chars().count();
    let right_width = pos_str.chars().count().min(width);
    let show_right = width >= mode_width + diagnostic_width + 2 + right_width;

    batch.clear_to_eol_styled_at(1, status_y, theme.statusline_base_style(), color_capability);
    batch.write_styled_at(
        1,
        status_y,
        theme.statusline_mode_style(mode_str),
        color_capability,
        truncate_display_width(&mode_segment, width),
    );

    let mut left_rest_x = mode_segment.chars().count() as u16 + 1;
    let max_file_state_width = if show_right {
        let right_x = size.width.saturating_sub(right_width as u16) + 1;
        right_x
            .saturating_sub(left_rest_x)
            .saturating_sub(diagnostic_width as u16) as usize
    } else {
        width
            .saturating_sub(left_rest_x as usize)
            .saturating_sub(diagnostic_width)
            .saturating_add(1)
    };
    if left_rest_x <= size.width && max_file_state_width > 0 {
        left_rest_x = write_statusline_segments(
            batch,
            left_rest_x,
            status_y,
            size.width,
            max_file_state_width.min(file_state_width),
            &file_state_segments,
            color_capability,
        );
    }

    // Write the left-side counts after the filename so they stay on the left
    // side of the status line without interrupting the path label.
    if left_rest_x <= size.width {
        write_statusline_segments(
            batch,
            left_rest_x,
            status_y,
            size.width,
            diagnostic_width,
            &diagnostic_segments,
            color_capability,
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

/// One styled status-line fragment written as an independent segment.
struct StatusLineSegment {
    text: String,
    style: ThemeStyle,
    display_width: usize,
    allow_truncation: bool,
}

impl StatusLineSegment {
    /// Build one segment whose display width matches its Unicode scalar count.
    fn new(text: String, style: ThemeStyle) -> Self {
        let display_width = text.chars().count();
        Self {
            text,
            style,
            display_width,
            allow_truncation: true,
        }
    }

    /// Build one non-truncating segment with an explicit terminal width.
    fn fixed_width(text: String, style: ThemeStyle, display_width: usize) -> Self {
        Self {
            text,
            style,
            display_width,
            allow_truncation: false,
        }
    }

    /// Return the number of terminal cells reserved for this segment.
    fn display_width(&self) -> usize {
        self.display_width
    }
}

/// Build the left-side file-state fragments for the status line.
fn build_statusline_file_state_segments(editor: &EditorState) -> Vec<StatusLineSegment> {
    const READ_ONLY_INDICATOR: &str = "🔒";
    const READ_ONLY_INDICATOR_WIDTH: usize = 2;

    let base_style = editor.theme().statusline_base_style();
    let mut segments = vec![StatusLineSegment::new(" ".to_string(), base_style)];
    if editor.is_modified() {
        segments.push(StatusLineSegment::new("[+] ".to_string(), base_style));
    }
    segments.push(StatusLineSegment::new(
        editor.file_name().to_string(),
        base_style,
    ));
    if editor.is_read_only() {
        segments.push(StatusLineSegment::new(" ".to_string(), base_style));
        segments.push(StatusLineSegment::fixed_width(
            READ_ONLY_INDICATOR.to_string(),
            editor.theme().statusline_readonly_style(),
            READ_ONLY_INDICATOR_WIDTH,
        ));
    }
    segments
}

/// Build the left-side diagnostic-count fragments for the status line.
fn build_statusline_diagnostic_segments(editor: &EditorState) -> Vec<StatusLineSegment> {
    let base_style = editor.theme().statusline_base_style();
    let counts = editor.active_diagnostic_counts();
    let mut segments = Vec::new();
    // Keep each piece separate so the dots can stay severity-colored without
    // changing the status-line styling of the adjacent numeric counts.
    for (severity, count) in [
        (crate::lsp::LspDiagnosticSeverity::Error, counts.errors),
        (crate::lsp::LspDiagnosticSeverity::Warning, counts.warnings),
    ] {
        if count == 0 {
            continue;
        }
        let count_text = format!(" {count}");
        segments.push(StatusLineSegment {
            text: DIAGNOSTIC_GUTTER_DOT.to_string(),
            style: statusline_diagnostic_dot_style(editor, severity),
            display_width: 1,
            allow_truncation: true,
        });
        segments.push(StatusLineSegment {
            text: count_text.clone(),
            style: base_style,
            display_width: count_text.chars().count(),
            allow_truncation: true,
        });
        segments.push(StatusLineSegment {
            text: " ".to_string(),
            style: base_style,
            display_width: 1,
            allow_truncation: true,
        });
    }
    if !segments.is_empty() {
        segments.insert(
            0,
            StatusLineSegment {
                text: " ".to_string(),
                style: base_style,
                display_width: 1,
                allow_truncation: true,
            },
        );
    }
    segments
}

/// Write status-line segments until `max_width` columns have been filled.
fn write_statusline_segments(
    batch: &mut tui::TerminalBatch,
    start_x: u16,
    y: u16,
    terminal_width: u16,
    max_width: usize,
    segments: &[StatusLineSegment],
    color_capability: crate::themes::ColorCapability,
) -> u16 {
    let mut x = start_x;
    let mut remaining = max_width;
    for segment in segments {
        if x > terminal_width || remaining == 0 {
            break;
        }
        let remaining_terminal_width = usize::from(terminal_width - x + 1).min(remaining);
        if remaining_terminal_width == 0 {
            break;
        }

        // Fixed-width Unicode markers either fit cleanly or stay hidden so the
        // rest of the status line keeps its column accounting.
        if segment.display_width() > remaining_terminal_width && !segment.allow_truncation {
            continue;
        }

        let visible = truncate_display_width(&segment.text, remaining_terminal_width);
        if visible.is_empty() {
            continue;
        }
        batch.write_styled_at(x, y, segment.style, color_capability, visible);
        let visible_width = visible.chars().count();
        x += visible_width as u16;
        remaining = remaining.saturating_sub(visible_width);
    }
    x
}

/// Return the status-line style for one severity dot.
fn statusline_diagnostic_dot_style(
    editor: &EditorState,
    severity: crate::lsp::LspDiagnosticSeverity,
) -> ThemeStyle {
    let accent = editor.theme().diagnostic_accent_style(severity);
    let base = editor.theme().statusline_base_style();
    ThemeStyle {
        fg: accent.fg,
        bg: base.bg,
        bold: accent.bold || base.bold,
        underline: false,
        undercurl: false,
        reverse: false,
    }
}

/// Render the top-right status and diagnostic overlays inside the buffer area.
fn render_top_right_overlays(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    size: TerminalSize,
    layout: RenderLayout,
) {
    let status_lines = editor
        .status_overlay_message()
        .map(|message| {
            top_right_overlay_visible_lines(message, layout.content_width, size.content_height())
        })
        .unwrap_or_default();
    let mut rows_used = 0usize;
    if !status_lines.is_empty() {
        let style = editor
            .theme()
            .diagnostic_message_style(crate::lsp::LspDiagnosticSeverity::Error);
        rows_used += render_right_aligned_overlay_lines(
            batch,
            editor,
            size,
            layout,
            rows_used,
            &status_lines,
            style,
        );
    }

    let Some(diagnostic) = editor.cursor_diagnostic() else {
        return;
    };
    let remaining_height = size.content_height().saturating_sub(rows_used);
    if remaining_height == 0 {
        return;
    }
    let diagnostic_lines = top_right_overlay_visible_lines(
        &diagnostic.message,
        layout.content_width,
        // Reserve one row for the separator only when a status overlay is
        // already using the rows above the diagnostic block.
        remaining_height.saturating_sub(usize::from(!status_lines.is_empty())),
    );
    if diagnostic_lines.is_empty() {
        return;
    }
    if !status_lines.is_empty() && rows_used < size.content_height() {
        let separator_width = status_lines
            .iter()
            .chain(diagnostic_lines.iter())
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let separator = "—".repeat(separator_width);
        batch.write_styled_at(
            (1 + layout.gutter_total_width + layout.content_width - separator_width) as u16,
            CONTENT_START_ROW + rows_used as u16,
            editor.theme().diagnostic_message_style(diagnostic.severity),
            editor.color_capability(),
            &separator,
        );
        rows_used += 1;
    }
    render_right_aligned_overlay_lines(
        batch,
        editor,
        size,
        layout,
        rows_used,
        &diagnostic_lines,
        editor.theme().diagnostic_message_style(diagnostic.severity),
    );
}

/// Return the visible right-aligned overlay lines capped to the current viewport.
fn top_right_overlay_visible_lines(
    message: &str,
    content_width: usize,
    content_height: usize,
) -> Vec<&str> {
    // Split first so embedded newlines do not skew the alignment math for the
    // later lines of a multi-line diagnostic message.
    message
        .lines()
        .take(content_height)
        .map(|line| truncate_right_display_width(line, content_width))
        .collect()
}

/// Render a block of right-aligned overlay lines and return the number of rows used.
fn render_right_aligned_overlay_lines(
    batch: &mut tui::TerminalBatch,
    editor: &EditorState,
    size: TerminalSize,
    layout: RenderLayout,
    row_offset: usize,
    lines: &[&str],
    style: ThemeStyle,
) -> usize {
    let overlay_right_edge = 1 + layout.gutter_total_width + layout.content_width;
    for (index, line) in lines.iter().enumerate() {
        // Every line anchors to the same right edge so varying line lengths still
        // read as one block instead of a ragged overlay staircase.
        let line_width = line.chars().count();
        let start_x = overlay_right_edge.saturating_sub(line_width) as u16;
        batch.write_styled_at(
            start_x.clamp(1, size.width),
            CONTENT_START_ROW + row_offset as u16 + index as u16,
            style,
            editor.color_capability(),
            line,
        );
    }
    lines.len()
}

/// Render only the command/message line while preserving the visible cursor.
pub(crate) fn render_message_line(
    term: &mut tui::Terminal,
    editor: &EditorState,
    size: TerminalSize,
    cursor_hidden_by_overlay: &mut bool,
) -> io::Result<()> {
    let mut batch = tui::TerminalBatch::new();
    let cursor_shape = editor.cursor_shape();
    let color_capability = editor.color_capability();
    if let (Some(prompt), Some(cursor_col)) = (editor.input_prompt(), editor.input_cursor_column())
    {
        write_message_line(&mut batch, editor, size);
        batch.set_cursor_shape(cursor_shape);
        batch.set_cursor_color(cursor_color(editor, cursor_shape), color_capability);
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
    batch.set_cursor_color(cursor_color(editor, cursor_shape), color_capability);
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let content_height = size.content_height();
    let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
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
    let swap_prompt_active = editor.swap_recovery_prompt().is_some();
    let message_style = if swap_prompt_active {
        editor.theme().message_line_swap_alert_style()
    } else {
        editor.theme().message_line_style()
    };

    let left_message = if let Some(prompt) = editor.swap_recovery_prompt() {
        prompt.to_string()
    } else if let Some(prompt) = editor.soft_read_only_save_prompt() {
        prompt
    } else if let Some(prompt) = editor.overwrite_prompt() {
        prompt
    } else if let Some(prompt) = editor.quit_prompt() {
        prompt
    } else if let Some(prompt) = editor.session_open_prompt() {
        prompt
    } else if let Some(prompt) = editor.buffer_close_prompt() {
        prompt
    } else if let (Some(prompt), Some(input)) = (editor.input_prompt(), editor.input_line()) {
        format!("{}{}", prompt, input)
    } else if let Some(label) = editor.macro_recording_label() {
        label
    } else if let Some(msg) = editor
        .status_message()
        .filter(|message| !message.contains('\n'))
    {
        msg.to_string()
    } else {
        String::new()
    };

    let pending_marker = editor.pending_prefix_label();
    let width = size.width as usize;
    batch.clear_to_eol_styled_at(1, msg_y, message_style, editor.color_capability());
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
                message_style,
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
            message_style,
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
    let content_height = size.content_height();
    let lines = sequence_discovery_popup_lines(&popup, size.width as usize, content_height);
    let Some(box_width) = lines.first().map(|line| line.chars().count()) else {
        return None;
    };

    // Anchor the popup to the bottom-right of the content area so it stays above
    // the status and message rows without affecting cursor placement logic.
    let start_x = (size.width as usize).saturating_sub(box_width) + 1;
    let start_y = content_height.saturating_sub(lines.len()) + CONTENT_START_ROW as usize;
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

/// Render the centered picker overlay and return its covered area.
fn render_picker_popup(
    batch: &mut tui::TerminalBatch,
    popup: &PickerPopup,
    editor: &EditorState,
    size: TerminalSize,
) -> PopupLayout {
    let rendered = layout_picker_popup(popup, size);
    let popup_style = editor.theme().popup_style();
    let selected_style = popup_style.overlay(editor.theme().selection_style());
    let active_style = popup_style.overlay(ThemeStyle {
        fg: editor
            .theme()
            .statusline_mode_style("NORMAL")
            .bg
            .or(editor.theme().selection_style().bg)
            .or(popup_style.fg),
        bg: None,
        bold: true,
        underline: false,
        undercurl: false,
        reverse: false,
    });
    for (index, line) in rendered.lines.iter().enumerate() {
        let y = rendered.layout.start_y + index as u16;
        if line.active && !line.selected {
            write_popup_active_line(
                batch,
                rendered.layout.start_x,
                y,
                &line.text,
                popup_style,
                active_style,
                editor.color_capability(),
            );
            continue;
        }
        batch.write_styled_at(
            rendered.layout.start_x,
            y,
            if line.selected {
                selected_style
            } else if line.active {
                active_style
            } else {
                popup_style
            },
            editor.color_capability(),
            &line.text,
        );
    }
    rendered.layout
}

/// Render one active popup row with unaccented borders and highlighted inner text.
fn write_popup_active_line(
    batch: &mut tui::TerminalBatch,
    start_x: u16,
    y: u16,
    line: &str,
    popup_style: ThemeStyle,
    active_style: ThemeStyle,
    color_capability: crate::themes::ColorCapability,
) {
    let Some(segments) = split_popup_border_segments(line) else {
        batch.write_styled_at(start_x, y, active_style, color_capability, line);
        return;
    };
    batch.write_styled_at(
        start_x,
        y,
        popup_style,
        color_capability,
        segments.left_border,
    );
    batch.write_styled_at(
        start_x + 1,
        y,
        active_style,
        color_capability,
        segments.body,
    );
    batch.write_styled_at(
        start_x + 1 + segments.body.chars().count() as u16,
        y,
        popup_style,
        color_capability,
        segments.right_border,
    );
}

/// One fully laid out picker popup with cursor placement.
struct PickerPopupLayout {
    lines: Vec<PickerPopupLine>,
    layout: PopupLayout,
    cursor_x: u16,
    cursor_y: u16,
}

/// One fully laid out completion popup.
struct CompletionPopupLayout {
    lines: Vec<CompletionPopupLine>,
    layout: PopupLayout,
}

/// One visible wrapped source line inside a generic text popup.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TextPopupLine {
    text: String,
    /// Original unwrapped body-line index, or `None` for popup border rows.
    source_line_index: Option<usize>,
    start_char: usize,
}

/// One unwrapped source line consumed by the generic text-popup layout helper.
struct TextPopupSourceLine<'a> {
    text: &'a str,
    source_line_index: usize,
}

/// Preferred side for one cursor-anchored text popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextPopupSide {
    Above,
    Below,
}

/// One fully laid out generic text popup.
struct TextPopupLayout {
    lines: Vec<TextPopupLine>,
    layout: PopupLayout,
}

/// Visible completion-entry slice chosen for the current popup frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompletionPopupWindow {
    start_index: usize,
    visible_entry_count: usize,
}

/// One rendered picker popup row with its selected-state styling hint.
#[derive(Clone)]
struct PickerPopupLine {
    text: String,
    selected: bool,
    active: bool,
}

/// One rendered completion popup row with its selected-state styling hint.
#[derive(Clone)]
struct CompletionPopupLine {
    text: String,
    selected: bool,
}

/// Borrowed slices for the left border, body, and right border of one popup row.
struct PopupBorderSegments<'a> {
    left_border: &'a str,
    body: &'a str,
    right_border: &'a str,
}

/// Split one boxed popup line into left border, body, and right border slices.
fn split_popup_border_segments(line: &str) -> Option<PopupBorderSegments<'_>> {
    let mut chars = line.char_indices();
    let (_, first) = chars.next()?;
    let (body_start, _) = chars.next()?;
    let (last_start, last) = line.char_indices().next_back()?;
    if first != POPUP_VERTICAL || last != POPUP_VERTICAL || body_start > last_start {
        return None;
    }
    Some(PopupBorderSegments {
        left_border: &line[..body_start],
        body: &line[body_start..last_start],
        right_border: &line[last_start..],
    })
}

/// Return the bounded popup height for a given terminal content height.
fn picker_popup_box_height(content_height: usize) -> usize {
    let available_height = content_height.saturating_sub(BUFFER_SWITCH_POPUP_VERTICAL_MARGIN * 2);
    available_height
        .clamp(1, BUFFER_SWITCH_POPUP_MAX_HEIGHT)
        .min(content_height.max(1))
}

/// Return the number of picker entries that fit in the current popup height.
fn picker_popup_entry_capacity(box_height: usize) -> usize {
    if box_height >= BUFFER_SWITCH_POPUP_MIN_HEIGHT {
        return box_height.saturating_sub(BUFFER_SWITCH_POPUP_NON_RESULT_ROWS);
    }
    if box_height >= BUFFER_SWITCH_POPUP_COMPACT_NON_RESULT_ROWS {
        return box_height.saturating_sub(BUFFER_SWITCH_POPUP_COMPACT_NON_RESULT_ROWS);
    }
    0
}

/// Return the PageUp/PageDown step for the current terminal content height.
pub(crate) fn picker_popup_page_step(content_height: usize) -> usize {
    let visible_entries = picker_popup_entry_capacity(picker_popup_box_height(content_height));
    visible_entries.saturating_sub(1).max(1)
}

/// Return the number of picker rows that can be shown for the current content height.
pub(crate) fn picker_popup_visible_entries(content_height: usize) -> usize {
    picker_popup_entry_capacity(picker_popup_box_height(content_height))
}

/// Build a centered fixed-size picker popup that keeps the query cursor visible.
fn layout_picker_popup(popup: &PickerPopup, size: TerminalSize) -> PickerPopupLayout {
    let max_width = size.width as usize;
    let max_height = size.content_height();
    let available_width = max_width.saturating_sub(BUFFER_SWITCH_POPUP_HORIZONTAL_MARGIN * 2);
    let box_width = available_width
        .clamp(POPUP_MIN_WIDTH, BUFFER_SWITCH_POPUP_MAX_WIDTH)
        .min(max_width.max(1));
    let box_height = picker_popup_box_height(max_height);
    let inner_width = box_width.saturating_sub(POPUP_BORDER_INSET).max(1);
    let entry_capacity = picker_popup_entry_capacity(box_height);
    let show_separator = box_height >= BUFFER_SWITCH_POPUP_MIN_HEIGHT;
    let entry_lines = if popup.entries.is_empty() || entry_capacity == 0 {
        vec![PickerPopupLine {
            text: format_popup_line(&format!(" {} ", popup.empty_message), inner_width),
            selected: false,
            active: false,
        }]
    } else {
        popup
            .entries
            .iter()
            .map(|entry| format_picker_entry(entry, inner_width))
            .collect::<Vec<_>>()
    };
    let blank_row = PickerPopupLine {
        text: format_popup_line("", inner_width),
        selected: false,
        active: false,
    };
    let mut entry_lines = entry_lines;
    while entry_lines.len() < entry_capacity {
        entry_lines.push(blank_row.clone());
    }
    entry_lines.truncate(entry_capacity);

    // The query view follows the input cursor once it extends beyond the fixed popup width.
    let query_prefix = popup.query_label.as_str();
    let query_suffix = popup.query_suffix.as_str();
    let reserved_suffix_width = if query_suffix.is_empty() {
        0
    } else {
        query_suffix.chars().count() + 1
    };
    let available_query_width = inner_width
        .saturating_sub(query_prefix.chars().count() + reserved_suffix_width)
        .max(1);
    let query_window_start = popup
        .cursor_column
        .saturating_sub(available_query_width.saturating_sub(1));
    let visible_query =
        slice_display_width(&popup.query, query_window_start, available_query_width);
    let query_line =
        format_picker_query_line(query_prefix, visible_query, query_suffix, inner_width, true);

    let mut lines = Vec::with_capacity(box_height.max(1));
    let query_row_index = if box_height == 1 {
        lines.push(PickerPopupLine {
            text: format_picker_query_line(
                query_prefix,
                visible_query,
                query_suffix,
                box_width,
                false,
            ),
            selected: false,
            active: false,
        });
        0
    } else {
        lines.push(PickerPopupLine {
            text: popup_top_border(&popup.title, inner_width),
            selected: false,
            active: false,
        });
        lines.extend(entry_lines);
        if show_separator {
            lines.push(PickerPopupLine {
                text: popup_separator_line(inner_width),
                selected: false,
                active: false,
            });
        }

        // Compact layouts drop the separator and/or result rows first, but they
        // always reserve the last body row for the query so the input stays usable.
        let query_row_index = lines.len();
        lines.push(PickerPopupLine {
            text: query_line,
            selected: false,
            active: false,
        });
        if lines.len() < box_height {
            lines.push(PickerPopupLine {
                text: format!(
                    "{POPUP_BOTTOM_LEFT}{}{POPUP_BOTTOM_RIGHT}",
                    POPUP_HORIZONTAL.to_string().repeat(inner_width)
                ),
                selected: false,
                active: false,
            });
        }
        query_row_index
    };

    let start_x = ((max_width.saturating_sub(box_width)) / 2 + 1) as u16;
    let start_y = ((max_height.saturating_sub(box_height)) / 2) as u16 + CONTENT_START_ROW;
    let visible_cursor_column = popup.cursor_column.saturating_sub(query_window_start);
    let mut raw_cursor_x = start_x + query_prefix.chars().count() as u16;
    // Boxed layouts place the query inside the left border, so the cursor needs
    // one extra column of offset before the visible query text begins.
    if box_height > 1 {
        raw_cursor_x += 1;
    }
    // The query cursor advances after the visible input characters inside the
    // current horizontal window rather than staying anchored to the full query.
    raw_cursor_x += visible_cursor_column as u16;
    let cursor_x = raw_cursor_x.min(start_x + box_width.saturating_sub(1) as u16);
    let cursor_y = start_y + query_row_index as u16;

    PickerPopupLayout {
        lines,
        layout: PopupLayout {
            start_x,
            start_y,
            width: box_width as u16,
            height: box_height as u16,
        },
        cursor_x,
        cursor_y,
    }
}

/// Build a compact completion popup anchored below the cursor, or above when needed.
fn layout_completion_popup(
    popup: &CompletionPopup,
    size: TerminalSize,
    cursor_x: u16,
    cursor_y: u16,
) -> Option<CompletionPopupLayout> {
    if popup.entries.is_empty() {
        return None;
    }

    let reserved_entry_count = popup.reserved_entry_count.max(popup.entries.len());
    let placement_entry_count = popup.placement_entry_count.max(reserved_entry_count);
    let content_bottom = size.height.saturating_sub(RESERVED_BOTTOM_ROWS);
    let rows_below = content_bottom.saturating_sub(cursor_y) as usize;
    let rows_above = cursor_y.saturating_sub(CONTENT_START_ROW) as usize;
    let below_entry_capacity =
        placement_entry_count.min(completion_popup_entry_capacity(rows_below));
    let above_entry_capacity =
        placement_entry_count.min(completion_popup_entry_capacity(rows_above));

    // Near the bottom edge, a cramped 1-3 entry popup reads better above the
    // cursor when the upper side can show at least as many suggestions.
    let (visible_entry_capacity, start_y) = if below_entry_capacity > 0
        && below_entry_capacity < COMPLETION_POPUP_MIN_PREFERRED_BELOW_ENTRIES
        && above_entry_capacity >= below_entry_capacity
    {
        let box_height = completion_popup_box_height(above_entry_capacity);
        (
            above_entry_capacity,
            cursor_y.saturating_sub(box_height as u16),
        )
    } else if below_entry_capacity > 0 {
        (below_entry_capacity, cursor_y + 1)
    } else if above_entry_capacity > 0 {
        let box_height = completion_popup_box_height(above_entry_capacity);
        (
            above_entry_capacity,
            cursor_y.saturating_sub(box_height as u16),
        )
    } else {
        return None;
    };

    if visible_entry_capacity == 0 {
        return None;
    }

    let selected_index = popup.entries.iter().position(|entry| entry.selected);
    let window =
        completion_popup_window(reserved_entry_count, visible_entry_capacity, selected_index);
    // Width follows the widest candidate in the full session so horizontal size
    // stays stable while the visible entry window scrolls around the selection.
    let max_inner_width = COMPLETION_POPUP_MAX_WIDTH
        .saturating_sub(POPUP_BORDER_INSET)
        .min(size.width.saturating_sub(POPUP_BORDER_INSET as u16) as usize)
        .max(1);
    let placement_inner_width = popup.placement_inner_width.max(popup.reserved_inner_width);
    let detail_column = completion_popup_detail_column(&popup.entries);
    let inner_width = popup
        .entries
        .iter()
        .map(|entry| completion_popup_entry_width(entry, detail_column))
        .max()
        .unwrap_or(1)
        .max(popup.reserved_inner_width)
        .min(max_inner_width)
        .max(1);
    let box_width = inner_width + POPUP_BORDER_INSET;
    let placement_box_width = placement_inner_width.min(max_inner_width) + POPUP_BORDER_INSET;
    let box_height = completion_popup_box_height(window.visible_entry_count);
    let max_start_x = size
        .width
        .saturating_sub(placement_box_width as u16)
        .saturating_add(1);
    let start_x = cursor_x.min(max_start_x).max(1);

    let mut lines = Vec::with_capacity(box_height);
    lines.push(CompletionPopupLine {
        text: format!(
            "{POPUP_TOP_LEFT}{}{POPUP_TOP_RIGHT}",
            POPUP_HORIZONTAL.to_string().repeat(inner_width)
        ),
        selected: false,
    });
    for entry in popup
        .entries
        .iter()
        .skip(window.start_index)
        .take(window.visible_entry_count)
    {
        lines.push(format_completion_entry(entry, detail_column, inner_width));
    }
    while lines.len() < box_height.saturating_sub(1) {
        lines.push(CompletionPopupLine {
            text: format_popup_line("", inner_width),
            selected: false,
        });
    }
    lines.push(CompletionPopupLine {
        text: format!(
            "{POPUP_BOTTOM_LEFT}{}{POPUP_BOTTOM_RIGHT}",
            POPUP_HORIZONTAL.to_string().repeat(inner_width)
        ),
        selected: false,
    });

    Some(CompletionPopupLayout {
        lines,
        layout: PopupLayout {
            start_x,
            start_y,
            width: box_width as u16,
            height: box_height as u16,
        },
    })
}

/// Layout one cursor-anchored hover popup above or below the cursor.
fn layout_hover_popup(
    popup: &HoverPopup,
    size: TerminalSize,
    cursor_x: u16,
    cursor_y: u16,
    content_height: usize,
) -> Option<TextPopupLayout> {
    layout_text_popup(
        &popup.title,
        popup
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| TextPopupSourceLine {
                text: line.as_str(),
                source_line_index: index,
            }),
        size,
        cursor_x,
        cursor_y,
        content_height,
        None,
    )
}

/// Layout one boxed text popup above or below the cursor.
fn layout_text_popup<'a, I>(
    title: &str,
    popup_lines: I,
    size: TerminalSize,
    cursor_x: u16,
    cursor_y: u16,
    content_height: usize,
    forced_side: Option<TextPopupSide>,
) -> Option<TextPopupLayout>
where
    I: IntoIterator<Item = TextPopupSourceLine<'a>>,
{
    if (size.width as usize) < POPUP_MIN_WIDTH
        || content_height + POPUP_BORDER_INSET < POPUP_MIN_HEIGHT
    {
        return None;
    }

    let content_bottom = size.height.saturating_sub(RESERVED_BOTTOM_ROWS);
    let rows_below = content_bottom.saturating_sub(cursor_y) as usize;
    let rows_above = cursor_y.saturating_sub(CONTENT_START_ROW) as usize;
    // Hover can contain full signatures and doc blocks, so let it grow close to
    // the right edge instead of constraining it to the narrower completion width.
    let max_inner_width = TEXT_POPUP_MAX_WIDTH
        .saturating_sub(POPUP_BORDER_INSET)
        .min(
            size.width
                .saturating_sub(TEXT_POPUP_HORIZONTAL_MARGIN as u16)
                .saturating_sub(1) as usize,
        )
        .max(1);
    let wrapped_lines = wrap_text_popup_lines(popup_lines, max_inner_width);
    if wrapped_lines.is_empty() {
        return None;
    }

    let below_line_capacity = wrapped_lines
        .len()
        .min(text_popup_line_capacity(rows_below));
    let above_line_capacity = wrapped_lines
        .len()
        .min(text_popup_line_capacity(rows_above));
    let (visible_line_capacity, start_y) = match forced_side {
        Some(TextPopupSide::Above) if above_line_capacity > 0 => {
            let box_height = text_popup_box_height(above_line_capacity);
            (
                above_line_capacity,
                cursor_y.saturating_sub(box_height as u16),
            )
        }
        Some(TextPopupSide::Below) if below_line_capacity > 0 => {
            (below_line_capacity, cursor_y + 1)
        }
        Some(_) => return None,
        _ if below_line_capacity > 0
            && below_line_capacity < TEXT_POPUP_MIN_PREFERRED_BELOW_LINES
            && above_line_capacity >= below_line_capacity =>
        {
            let box_height = text_popup_box_height(above_line_capacity);
            (
                above_line_capacity,
                cursor_y.saturating_sub(box_height as u16),
            )
        }
        _ if below_line_capacity > 0 => (below_line_capacity, cursor_y + 1),
        _ if above_line_capacity > 0 => {
            let box_height = text_popup_box_height(above_line_capacity);
            (
                above_line_capacity,
                cursor_y.saturating_sub(box_height as u16),
            )
        }
        _ => return None,
    };
    if visible_line_capacity == 0 {
        return None;
    }

    // Only the visible prefix influences layout, so compute width directly from
    // the wrapped slice instead of allocating a second vector.
    let visible_line_count = wrapped_lines.len().min(visible_line_capacity);
    let inner_width = wrapped_lines
        .iter()
        .take(visible_line_count)
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + POPUP_TITLE_PADDING * 2)
        .min(max_inner_width)
        .max(1);
    let box_width = inner_width + POPUP_BORDER_INSET;
    let box_height = text_popup_box_height(visible_line_count);
    let max_start_x = size
        .width
        .saturating_sub(box_width as u16)
        .saturating_add(1);
    let start_x = cursor_x.min(max_start_x).max(1);

    let mut lines = Vec::with_capacity(box_height);
    lines.push(TextPopupLine {
        text: popup_top_border(title, inner_width),
        source_line_index: None,
        start_char: 0,
    });
    // Rendering reuses the wrapped iterator so the visible rows and measured
    // width stay in lockstep even when the popup is clipped by height.
    for line in wrapped_lines.into_iter().take(visible_line_count) {
        lines.push(TextPopupLine {
            text: format_popup_line(&line.text, inner_width),
            source_line_index: line.source_line_index,
            start_char: line.start_char,
        });
    }
    lines.push(TextPopupLine {
        text: format!(
            "{POPUP_BOTTOM_LEFT}{}{POPUP_BOTTOM_RIGHT}",
            POPUP_HORIZONTAL.to_string().repeat(inner_width)
        ),
        source_line_index: None,
        start_char: 0,
    });
    Some(TextPopupLayout {
        lines,
        layout: PopupLayout {
            start_x,
            start_y,
            width: box_width as u16,
            height: box_height as u16,
        },
    })
}

/// Return the capped number of completion entries that fit in `available_rows`.
fn completion_popup_entry_capacity(available_rows: usize) -> usize {
    if available_rows < POPUP_MIN_HEIGHT {
        return 0;
    }
    available_rows
        .saturating_sub(POPUP_BORDER_INSET)
        .min(COMPLETION_POPUP_MAX_HEIGHT.saturating_sub(POPUP_BORDER_INSET))
}

/// Return the total boxed height for a completion popup body of `entry_count` rows.
fn completion_popup_box_height(entry_count: usize) -> usize {
    entry_count + POPUP_BORDER_INSET
}

/// Return the visible completion-entry window for the current selection.
fn completion_popup_window(
    entry_count: usize,
    visible_entry_capacity: usize,
    selected_index: Option<usize>,
) -> CompletionPopupWindow {
    let visible_entry_count = entry_count.min(visible_entry_capacity);
    let centered_start = selected_index
        .map(|index| index.saturating_sub(visible_entry_count.saturating_sub(1) / 2))
        .unwrap_or(0);
    let start_index = centered_start.min(entry_count.saturating_sub(visible_entry_count));
    CompletionPopupWindow {
        start_index,
        visible_entry_count,
    }
}

/// Format the picker query row with an optional right-aligned suffix.
fn format_picker_query_line(
    query_prefix: &str,
    visible_query: &str,
    query_suffix: &str,
    width: usize,
    boxed: bool,
) -> String {
    let left = format!("{query_prefix}{visible_query}");
    let suffix_width = query_suffix.chars().count();
    let left_width = left.chars().count();
    let body = if suffix_width > 0 && left_width + suffix_width < width {
        format!(
            "{left}{}{query_suffix}",
            " ".repeat(width - left_width - suffix_width)
        )
    } else {
        truncate_display_width(&left, width).to_string()
    };
    if boxed {
        format_popup_line(&body, width)
    } else {
        truncate_display_width(&body, width).to_string()
    }
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

    // The body lists only continuations because the typed prefix lives in the
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

/// Build the borderless LSP progress overlay lines anchored above the bottom bars.
fn lsp_progress_overlay_lines(
    progress_lines: &[String],
    max_width: usize,
    max_height: usize,
) -> Vec<String> {
    if progress_lines.is_empty() || max_width == 0 || max_height == 0 {
        return Vec::new();
    }
    progress_lines
        .iter()
        .take(max_height)
        .map(|line| truncate_display_width(line, max_width).to_string())
        .collect()
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

/// Format one picker row and indicate whether it should use selected-row styling.
fn format_picker_entry(entry: &PickerPopupEntry, inner_width: usize) -> PickerPopupLine {
    let active = if entry.primary_marker { '%' } else { ' ' };
    let modified = if entry.secondary_marker { '+' } else { ' ' };
    PickerPopupLine {
        text: format_popup_line(
            &format!(" {active}{modified} {} ", entry.label),
            inner_width,
        ),
        selected: entry.selected,
        active: entry.primary_marker,
    }
}

/// Format one completion row using the compact cursor-anchored popup style.
fn format_completion_entry(
    entry: &crate::completion::CompletionPopupEntry,
    detail_column: usize,
    inner_width: usize,
) -> CompletionPopupLine {
    CompletionPopupLine {
        text: format_popup_line(
            &completion_popup_entry_text(entry, detail_column),
            inner_width,
        ),
        selected: entry.selected,
    }
}

/// Return the inner popup width required to render `entry`.
fn completion_popup_entry_width(
    entry: &crate::completion::CompletionPopupEntry,
    detail_column: usize,
) -> usize {
    completion_popup_entry_text(entry, detail_column)
        .chars()
        .count()
}

/// Build the visible popup text for one completion entry.
fn completion_popup_entry_text(
    entry: &crate::completion::CompletionPopupEntry,
    detail_column: usize,
) -> String {
    if let Some(detail) = &entry.detail {
        return format!(" {:<detail_column$}  {} ", entry.label, detail);
    }
    format!(" {} ", entry.label)
}

/// Return the shared label width used to align popup details in one popup body.
fn completion_popup_detail_column(
    all_entries: &[crate::completion::CompletionPopupEntry],
) -> usize {
    all_entries
        .iter()
        .map(|entry| entry.label.chars().count())
        .max()
        .unwrap_or(0)
}

/// Build the separator line that visually splits the results from the query row.
fn popup_separator_line(inner_width: usize) -> String {
    format!(
        "{POPUP_LEFT_TEE}{}{POPUP_RIGHT_TEE}",
        POPUP_HORIZONTAL.to_string().repeat(inner_width)
    )
}

/// Wrap one popup body line with borders and right padding.
fn format_popup_line(content: &str, inner_width: usize) -> String {
    let truncated = truncate_display_width(content, inner_width);
    format!("{POPUP_VERTICAL}{truncated:<inner_width$}{POPUP_VERTICAL}")
}

/// Wrap generic text-popup lines into display-width chunks without allocating per character.
fn wrap_text_popup_lines<'a, I>(lines: I, max_chars: usize) -> Vec<TextPopupLine>
where
    I: IntoIterator<Item = TextPopupSourceLine<'a>>,
{
    if max_chars == 0 {
        return Vec::new();
    }
    let mut wrapped = Vec::new();
    for line in lines {
        // Preserve blank lines from the source popup so documentation spacing
        // survives wrapping and clipping unchanged.
        if line.text.is_empty() {
            wrapped.push(TextPopupLine {
                text: String::new(),
                source_line_index: Some(line.source_line_index),
                start_char: 0,
            });
            continue;
        }
        let char_count = line.text.chars().count();
        let mut start_char = 0;
        while start_char < char_count {
            // Record the starting character offset for each wrapped fragment so
            // render-time highlighting can map back into the original line.
            wrapped.push(TextPopupLine {
                text: slice_display_width(line.text, start_char, max_chars).to_string(),
                source_line_index: Some(line.source_line_index),
                start_char,
            });
            start_char = start_char.saturating_add(max_chars);
        }
    }
    wrapped
}

/// Return the capped number of generic text-popup body lines that fit in `available_rows`.
fn text_popup_line_capacity(available_rows: usize) -> usize {
    if available_rows < POPUP_MIN_HEIGHT {
        return 0;
    }
    available_rows
        .saturating_sub(POPUP_BORDER_INSET)
        .min(TEXT_POPUP_MAX_HEIGHT.saturating_sub(POPUP_BORDER_INSET))
}

/// Return the total boxed height for a generic text-popup body of `line_count` rows.
fn text_popup_box_height(line_count: usize) -> usize {
    line_count + POPUP_BORDER_INSET
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

/// Return the `max_chars`-wide visible window of `input` starting at `start_char`.
fn slice_display_width(input: &str, start_char: usize, max_chars: usize) -> &str {
    let start = input
        .char_indices()
        .nth(start_char)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(input.len());
    let end = input[start..]
        .char_indices()
        .nth(max_chars)
        .map(|(byte_idx, _)| start + byte_idx)
        .unwrap_or(input.len());
    &input[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::{CompletionPopup, CompletionPopupEntry};
    use crate::dialogs::{HoverPopup, PickerPopup};
    use crate::lsp::{LspDiagnostic, LspDiagnosticSeverity, LspFileDiagnostics};
    use crate::mode::Mode;
    use crate::text_buffer::TextBuffer;
    use std::path::PathBuf;
    use termion::event::Key;

    /// Build one editor with many named buffers for picker-layout tests.
    fn create_buffer_switch_test_editor(
        buffer_count: usize,
        terminal_height: usize,
    ) -> EditorState {
        let mut editor = EditorState::new(terminal_height);
        editor.set_startup_path("/tmp/buffer_00.rs");
        for index in 1..buffer_count {
            editor
                .open_buffer(format!("/tmp/buffer_{index:02}.rs"))
                .expect("open named buffer");
        }
        editor
    }

    /// Build one compact completion popup for render-layout tests.
    fn create_completion_popup(labels: &[&str], selected_index: Option<usize>) -> CompletionPopup {
        let reserved_inner_width = labels
            .iter()
            .map(|label| label.chars().count() + 2)
            .max()
            .unwrap_or(1);
        CompletionPopup {
            anchor_char_idx: 0,
            entries: labels
                .iter()
                .enumerate()
                .map(|(index, label)| CompletionPopupEntry {
                    label: (*label).to_string(),
                    detail: None,
                    selected: selected_index == Some(index),
                })
                .collect(),
            reserved_entry_count: labels.len(),
            reserved_inner_width,
            placement_entry_count: labels.len(),
            placement_inner_width: reserved_inner_width,
        }
    }

    /// Apply one single-line diagnostic to `editor` for render tests.
    fn apply_render_test_diagnostic(
        editor: &mut EditorState,
        path: &str,
        start: usize,
        end: usize,
    ) {
        apply_render_test_diagnostics(
            editor,
            path,
            vec![(
                0,
                start,
                end,
                LspDiagnosticSeverity::Error,
                "render diagnostic",
            )],
        );
    }

    /// Apply the supplied diagnostics to `editor` for render tests.
    fn apply_render_test_diagnostics(
        editor: &mut EditorState,
        path: &str,
        diagnostics: Vec<(usize, usize, usize, LspDiagnosticSeverity, &str)>,
    ) {
        editor.set_startup_path(path);
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            PathBuf::from(path),
            Some(0),
            diagnostics
                .into_iter()
                .map(|(line, start, end, severity, message)| LspDiagnostic {
                    range: crate::lsp::protocol::LspRange {
                        start: crate::lsp::protocol::LspPosition {
                            line,
                            character: start,
                        },
                        end: crate::lsp::protocol::LspPosition {
                            line,
                            character: end,
                        },
                    },
                    severity,
                    message: message.to_string(),
                    source: None,
                    code: None,
                })
                .collect(),
        ));
    }

    /// Strip ANSI control sequences so assertions can inspect visible text only.
    fn strip_terminal_escapes(input: &str) -> String {
        let mut stripped = String::new();
        let mut chars = input.chars();
        while let Some(ch) = chars.next() {
            if ch != '\u{1b}' {
                stripped.push(ch);
                continue;
            }
            match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for next in chars.by_ref() {
                        if next == '\u{7}' {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        stripped
    }

    #[test]
    fn test_terminal_size_clamps_zero() {
        assert_eq!(
            TerminalSize::from_termion((0, 0)),
            TerminalSize {
                width: 1,
                height: 4
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
                height: 4
            }
        );
    }

    #[test]
    fn test_render_decision_full_for_sequence_popup_change() {
        let mut before = EditorState::new(24);
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        after.set_startup_path("a.txt");
        after.set_mode(Mode::Normal);
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
        before.buffer_mut().insert(0, "x");
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        after.buffer_mut().insert(0, "x");
        after.set_startup_path("a.txt");
        after.set_mode(Mode::command_with_text("q"));
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
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("hello");
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("hello");
        after.set_startup_path("a.txt");
        after.handle_key(termion::event::Key::Char('g'));
        after.handle_key(termion::event::Key::Char('g'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::None);
    }

    #[test]
    fn test_render_decision_full_when_read_only_indicator_changes() {
        let file = test_utils::TempFile::with_suffix(".txt").expect("create temp file");
        std::fs::write(file.path(), "hello").expect("seed temp file");

        let mut before = EditorState::new(24);
        before.set_startup_path(file.path());

        let mut permissions = std::fs::metadata(file.path())
            .expect("stat temp file")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(file.path(), permissions).expect("mark temp file read-only");

        let mut after = EditorState::new(24);
        after.set_startup_path(file.path());

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_cursor_only_when_cursor_moves_on_same_line() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        after.set_startup_path("a.txt");
        after.handle_key(termion::event::Key::Char('l'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::CursorOnly);
    }

    #[test]
    fn test_render_decision_full_when_same_line_motion_must_clear_message_row() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        before.set_startup_path("a.txt");
        before.show_status_message("Resolving definition...");
        before.finish_message_render();

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        after.set_startup_path("a.txt");
        after.show_status_message("Resolving definition...");
        after.finish_message_render();
        after.handle_key(termion::event::Key::Char('l'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    /// Clearing a rendered status overlay should force a full redraw.
    fn test_render_decision_full_when_clearing_status_overlay() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        before.set_startup_path("a.txt");
        before.show_status_message("Invalid regex:\nregex parse error:");
        before.finish_full_render();

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        after.set_startup_path("a.txt");
        after.show_status_message("Invalid regex:\nregex parse error:");
        after.finish_full_render();
        // Any follow-up key should dismiss the stale overlay and therefore force
        // the renderer down the full-frame path that repaints buffer content.
        after.handle_key(termion::event::Key::Char('l'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    /// Verify that stable vertical motion uses the targeted gutter redraw path.
    #[test]
    fn test_render_decision_vertical_cursor_when_cursor_moves_lines_without_other_changes() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("a\nb");
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("a\nb");
        after.set_startup_path("a.txt");

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
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("a\nb");
        before.set_startup_path("a.txt");
        before.apply_config(&crate::config::ConfigSettings {
            relative_line_numbers: Some(true),
            ..crate::config::ConfigSettings::default()
        });

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("a\nb");
        after.set_startup_path("a.txt");
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
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        before.set_startup_path("a.txt");
        before.handle_key(termion::event::Key::Char('v'));

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("ab");
        after.set_startup_path("a.txt");
        after.handle_key(termion::event::Key::Char('v'));
        after.handle_key(termion::event::Key::Char('l'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_statusline_diagnostic_counts_change() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("first\nsecond");
        before.set_startup_path("/tmp/status_counts.rs");

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("first\nsecond");
        after.set_startup_path("/tmp/status_counts.rs");
        apply_render_test_diagnostics(
            &mut after,
            "/tmp/status_counts.rs",
            vec![(1, 0, 6, LspDiagnosticSeverity::Warning, "warning")],
        );

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_same_line_motion_clears_pending_prefix() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("abca");
        before.set_startup_path("a.txt");
        before.handle_key(termion::event::Key::Char('f'));

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("abca");
        after.set_startup_path("a.txt");
        after.handle_key(termion::event::Key::Char('f'));
        after.handle_key(termion::event::Key::Char('a'));

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_buffer_switch_popup_cursor_stays_after_current_query_character() {
        let popup = PickerPopup {
            title: "Buffers".to_string(),
            query_label: " Filter: ".to_string(),
            query_suffix: String::new(),
            empty_message: "No matching buffers".to_string(),
            query: "alpha".to_string(),
            cursor_column: 5,
            entries: Vec::new(),
        };

        let rendered = layout_picker_popup(
            &popup,
            TerminalSize {
                width: 100,
                height: 30,
            },
        );

        let query_row = &rendered.lines[rendered.lines.len() - 2].text;
        let expected_prefix = format!("{POPUP_VERTICAL} Filter: alpha");
        assert!(query_row.starts_with(&expected_prefix));
        assert_eq!(rendered.cursor_x, 24);
        assert_eq!(rendered.cursor_y, 23);
    }

    #[test]
    fn test_buffer_switch_popup_cursor_uses_query_row_on_small_terminal() {
        let popup = PickerPopup {
            title: "Buffers".to_string(),
            query_label: " Filter: ".to_string(),
            query_suffix: String::new(),
            empty_message: "No matching buffers".to_string(),
            query: "abc".to_string(),
            cursor_column: 3,
            entries: vec![PickerPopupEntry {
                label: "src/main.rs".to_string(),
                selected: true,
                primary_marker: false,
                secondary_marker: false,
            }],
        };

        let rendered = layout_picker_popup(
            &popup,
            TerminalSize {
                width: 30,
                height: 7,
            },
        );

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered.lines[1].text, "│ Filter: abc        │");
        assert_eq!(rendered.cursor_x, 18);
        assert_eq!(rendered.cursor_y, 4);
    }

    #[test]
    fn test_picker_query_suffix_renders_on_right_side_of_query_row() {
        let popup = PickerPopup {
            title: "Files".to_string(),
            query_label: " Open: ".to_string(),
            query_suffix: "⠋".to_string(),
            empty_message: "No matching files".to_string(),
            query: "abc".to_string(),
            cursor_column: 3,
            entries: Vec::new(),
        };

        let rendered = layout_picker_popup(
            &popup,
            TerminalSize {
                width: 30,
                height: 7,
            },
        );

        assert_eq!(rendered.lines[1].text, "│ Open: abc         ⠋│");
    }

    #[test]
    fn test_buffer_switch_popup_formats_active_entry_with_marker() {
        let line = format_picker_entry(
            &PickerPopupEntry {
                label: "src/main.rs".to_string(),
                selected: false,
                primary_marker: true,
                secondary_marker: false,
            },
            24,
        );

        assert!(line.text.contains("%  src/main.rs"));
        assert!(line.active);
        assert!(!line.selected);
    }

    #[test]
    /// Top-right overlays should keep the buffer background and right edge alignment.
    fn test_render_top_right_overlays_right_align_with_buffer_background() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("let broken = value;");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        apply_render_test_diagnostics(
            &mut editor,
            "/tmp/overlay_diag.rs",
            vec![(0, 0, 10, LspDiagnosticSeverity::Error, "warning")],
        );
        let size = TerminalSize {
            width: 30,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
        let mut batch = tui::TerminalBatch::new();
        let background = termion::color::AnsiValue(
            editor
                .theme()
                .background_style()
                .bg
                .expect("background style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        render_top_right_overlays(&mut batch, &editor, size, layout);

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains(&format!("{}", termion::cursor::Goto(24, CONTENT_START_ROW))));
        assert!(output.contains(&background));
        assert!(output.contains("warning"));
    }

    #[test]
    /// Multi-line top-right overlays should share the viewport's right edge.
    fn test_render_top_right_overlays_align_to_viewport_right_edge() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("cannot find value `garbage` in this scope");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        apply_render_test_diagnostics(
            &mut editor,
            "/tmp/overlay_align_diag.rs",
            vec![(
                0,
                0,
                26,
                LspDiagnosticSeverity::Error,
                "cannot find value `garbage` in this scope\nnot found in this scope",
            )],
        );
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
        let expected_x = (1 + layout.gutter_total_width + layout.content_width
            - "not found in this scope".chars().count()) as u16;
        let mut batch = tui::TerminalBatch::new();

        render_top_right_overlays(&mut batch, &editor, size, layout);

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        assert!(output.contains(&format!(
            "{}",
            termion::cursor::Goto(expected_x, CONTENT_START_ROW + 1)
        )));
        assert!(output.contains("not found in this scope"));
    }

    #[test]
    /// Status overlays should stack above cursor diagnostics with a separator line.
    fn test_render_top_right_overlays_stack_status_above_diagnostic() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("let broken = value;");
        editor.show_status_message("Invalid regex:\nregex parse error:");
        apply_render_test_diagnostics(
            &mut editor,
            "/tmp/stacked_overlay_diag.rs",
            vec![(0, 0, 10, LspDiagnosticSeverity::Error, "diagnostic")],
        );
        let size = TerminalSize {
            width: 40,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, editor.render_buffer_line_count());
        let overlay_right_edge = 1 + layout.gutter_total_width + layout.content_width;
        let separator = "—".repeat("regex parse error:".chars().count());
        let separator_x = (overlay_right_edge - separator.chars().count()) as u16;
        let diagnostic_x = (overlay_right_edge - "diagnostic".chars().count()) as u16;
        let mut batch = tui::TerminalBatch::new();

        render_top_right_overlays(&mut batch, &editor, size, layout);

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        // The separator consumes one row, so the diagnostic block starts after
        // the two-line status overlay plus that extra divider line.
        assert!(output.contains("Invalid regex:"));
        assert!(output.contains("regex parse error:"));
        assert!(output.contains(&separator));
        assert!(output.contains(&format!(
            "{}",
            termion::cursor::Goto(separator_x, CONTENT_START_ROW + 2)
        )));
        assert!(output.contains(&format!(
            "{}",
            termion::cursor::Goto(diagnostic_x, CONTENT_START_ROW + 3)
        )));
        assert!(output.contains("diagnostic"));
    }

    #[test]
    fn test_render_status_line_shows_error_and_warning_counts() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("first\nsecond\nthird");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        apply_render_test_diagnostics(
            &mut editor,
            "/tmp/status_diag.rs",
            vec![
                (0, 0, 5, LspDiagnosticSeverity::Error, "error"),
                (1, 0, 6, LspDiagnosticSeverity::Warning, "warning"),
                (2, 0, 5, LspDiagnosticSeverity::Hint, "hint"),
            ],
        );
        let mut batch = tui::TerminalBatch::new();
        let error_color = termion::color::AnsiValue(
            editor
                .theme()
                .diagnostic_accent_style(LspDiagnosticSeverity::Error)
                .fg
                .expect("error accent should set a foreground")
                .ansi256_index(),
        )
        .fg_string();
        let warning_color = termion::color::AnsiValue(
            editor
                .theme()
                .diagnostic_accent_style(LspDiagnosticSeverity::Warning)
                .fg
                .expect("warning accent should set a foreground")
                .ansi256_index(),
        )
        .fg_string();

        render_status_line(
            &mut batch,
            &editor,
            TerminalSize {
                width: 80,
                height: 24,
            },
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        let visible = strip_terminal_escapes(output);
        assert!(visible.contains("NORMAL"));
        assert!(visible.contains("status_diag.rs"));
        assert!(visible.contains("status_diag.rs ● 1 ● 1"));
        assert!(visible.contains("1:1 "));
        assert!(output.contains(&error_color));
        assert!(output.contains(&warning_color));
    }

    #[test]
    fn test_render_status_line_shows_colored_read_only_indicator() {
        let file = test_utils::TempFile::with_suffix(".rs").expect("create temp file");
        std::fs::write(file.path(), "first\nsecond\nthird").expect("seed temp file");
        let mut permissions = std::fs::metadata(file.path())
            .expect("stat temp file")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(file.path(), permissions).expect("mark temp file read-only");

        let mut editor = EditorState::new(24);
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.load_file(file.path()).expect("load temp file");
        let mut batch = tui::TerminalBatch::new();
        let readonly_color = termion::color::AnsiValue(
            editor
                .theme()
                .statusline_readonly_style()
                .fg
                .expect("read-only marker should set a foreground")
                .ansi256_index(),
        )
        .fg_string();

        render_status_line(
            &mut batch,
            &editor,
            TerminalSize {
                width: 80,
                height: 24,
            },
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        let visible = strip_terminal_escapes(output);
        assert!(visible.contains(&format!(
            "{} 🔒",
            file.path().file_name().unwrap().to_str().unwrap()
        )));
        assert!(output.contains(&readonly_color));
    }

    #[test]
    fn test_write_message_line_highlights_swap_prompt() {
        let mut editor = EditorState::new(24);
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.set_swap_recovery_prompt_for_test(true);
        let mut batch = tui::TerminalBatch::new();
        let alert_style = editor.theme().message_line_swap_alert_style();
        let alert_fg = termion::color::AnsiValue(
            alert_style
                .fg
                .expect("swap alert should set a foreground")
                .ansi256_index(),
        )
        .fg_string();
        let alert_bg = termion::color::AnsiValue(
            alert_style
                .bg
                .expect("swap alert should set a background")
                .ansi256_index(),
        )
        .bg_string();

        write_message_line(
            &mut batch,
            &editor,
            TerminalSize {
                width: 80,
                height: 24,
            },
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        let visible = strip_terminal_escapes(output);
        assert!(visible.contains("swap prompt"));
        assert!(output.contains(&alert_fg));
        assert!(output.contains(&alert_bg));
    }

    #[test]
    fn test_render_status_line_keeps_mode_visible_during_macro_recording() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("first\nsecond");
        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        let mut batch = tui::TerminalBatch::new();

        render_status_line(
            &mut batch,
            &editor,
            TerminalSize {
                width: 80,
                height: 24,
            },
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        let visible = strip_terminal_escapes(output);
        assert!(visible.contains("NORMAL"));
        assert!(visible.contains("1:1 "));
        assert!(!visible.contains("recording @a"));
    }

    #[test]
    fn test_macro_recording_label_uses_message_row_slot() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("first\nsecond");
        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        let mut batch = tui::TerminalBatch::new();

        write_message_line(
            &mut batch,
            &editor,
            TerminalSize {
                width: 80,
                height: 24,
            },
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        let visible = strip_terminal_escapes(output);
        assert!(visible.contains("recording @a"));
    }

    #[test]
    fn test_macro_recording_label_overrides_transient_status_message() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("first\nsecond");
        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.show_status_message("Nothing to repeat");
        let mut batch = tui::TerminalBatch::new();

        write_message_line(
            &mut batch,
            &editor,
            TerminalSize {
                width: 80,
                height: 24,
            },
        );

        let output = std::str::from_utf8(batch.as_bytes()).expect("batch output should be UTF-8");
        let visible = strip_terminal_escapes(output);
        assert!(visible.contains("recording @a"));
        assert!(!visible.contains("Nothing to repeat"));
    }

    #[test]
    fn test_buffer_switch_page_down_keeps_previous_last_visible_entry_on_screen() {
        let mut editor = create_buffer_switch_test_editor(10, 12);
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('b'));

        let initial_popup = editor.picker_popup().expect("picker popup");
        let initial_layout = layout_picker_popup(
            &initial_popup,
            TerminalSize {
                width: 80,
                height: 12,
            },
        );
        assert!(
            initial_layout
                .lines
                .iter()
                .any(|line| line.text.contains("buffer_09.rs"))
        );

        editor.handle_key(Key::PageDown);

        let paged_popup = editor.picker_popup().expect("paged picker popup");
        assert!(paged_popup.entries.iter().any(|entry| entry.selected));
        let paged_layout = layout_picker_popup(
            &paged_popup,
            TerminalSize {
                width: 80,
                height: 12,
            },
        );
        let previous_last_visible = initial_layout
            .lines
            .iter()
            .rev()
            .find_map(|line| line.text.contains("buffer_").then_some(line.text.clone()))
            .expect("initial popup should show one buffer row");
        assert!(
            paged_layout
                .lines
                .iter()
                .any(|line| line.text == previous_last_visible),
            "page-down popup should keep the previous last visible entry on screen"
        );

        editor.handle_key(Key::PageUp);

        let unpaged_popup = editor.picker_popup().expect("unpged picker popup");
        assert!(unpaged_popup.entries.iter().any(|entry| entry.selected));
    }

    #[test]
    fn test_render_decision_full_when_relative_line_numbers_change() {
        let mut before = EditorState::new(24);
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("a\nb");
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("a\nb");
        after.set_startup_path("a.txt");
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
        *before.buffer_mut() =
            crate::text_buffer::TextBuffer::from_str("abcdefghijklmnopqrstuvwxyz");
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        *after.buffer_mut() =
            crate::text_buffer::TextBuffer::from_str("abcdefghijklmnopqrstuvwxyz");
        after.set_startup_path("a.txt");
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
    fn test_render_decision_full_when_lsp_progress_changes() {
        let mut before = EditorState::new(24);
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        after.set_startup_path("a.txt");
        after.set_lsp_progress_lines(vec!["Indexing (5%)".to_string()]);

        let decision = RenderSnapshot::decide(
            &RenderSnapshot::capture(&before),
            &RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, RenderDecision::Full);
    }

    #[test]
    fn test_render_decision_full_when_hover_popup_changes() {
        let mut before = EditorState::new(24);
        before.set_startup_path("a.txt");
        let mut after = EditorState::new(24);
        after.set_startup_path("a.txt");
        after.handle_key(Key::Char('K'));
        after.apply_hover_lookup_result(crate::lsp::HoverLookupResult {
            buffer_id: after.active_buffer_id(),
            lookup_token: 1,
            document_version: 0,
            outcome: crate::lsp::HoverLookupOutcome::Found("fn helper()".to_string()),
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
        *before.buffer_mut() = crate::text_buffer::TextBuffer::from_str("fn main() {}\n");
        before.set_startup_path("sample.rs");

        let mut after = EditorState::new(24);
        *after.buffer_mut() = crate::text_buffer::TextBuffer::from_str("fn main() {}\n");
        after.set_startup_path("sample.rs");
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
    fn test_completion_popup_layout_prefers_space_below_cursor() {
        let popup = create_completion_popup(&["alphabet", "alpha_num"], Some(0));
        let size = TerminalSize {
            width: 40,
            height: 12,
        };

        let layout =
            layout_completion_popup(&popup, size, 10, 4).expect("popup should fit below cursor");

        assert_eq!(layout.layout.start_y, 5);
        assert_eq!(layout.layout.height, 4);
    }

    #[test]
    /// Confirm cramped bottom-edge layouts prefer showing the popup above the cursor.
    fn test_completion_popup_layout_prefers_above_when_below_would_show_one_entry() {
        let popup = create_completion_popup(&["alpha0", "alpha1", "alpha2"], Some(0));
        let size = TerminalSize {
            width: 40,
            height: 11,
        };

        // This cursor position leaves room for one boxed entry below but more above.
        let layout =
            layout_completion_popup(&popup, size, 10, 6).expect("popup should fit above cursor");

        assert_eq!(layout.layout.start_y, 2);
        assert_eq!(layout.layout.height, 4);
    }

    #[test]
    /// Confirm a 3-entry bottom-edge popup also moves above when more space is available there.
    fn test_completion_popup_layout_prefers_above_when_below_would_show_three_entries() {
        let popup =
            create_completion_popup(&["alpha0", "alpha1", "alpha2", "alpha3", "alpha4"], Some(0));
        let size = TerminalSize {
            width: 40,
            height: 15,
        };

        // This cursor position leaves room for three entries below and four above.
        let layout =
            layout_completion_popup(&popup, size, 10, 8).expect("popup should fit above cursor");

        assert_eq!(layout.layout.start_y, 2);
        assert_eq!(layout.layout.height, 6);
    }

    #[test]
    fn test_completion_popup_layout_moves_above_when_below_has_no_room() {
        let popup = create_completion_popup(&["alphabet"], Some(0));
        let size = TerminalSize {
            width: 40,
            height: 8,
        };

        let layout =
            layout_completion_popup(&popup, size, 10, 5).expect("popup should fit above cursor");

        assert_eq!(layout.layout.start_y, 2);
        assert_eq!(layout.layout.height, 3);
    }

    #[test]
    fn test_completion_popup_layout_uses_variable_height_for_few_entries() {
        let popup = create_completion_popup(&["alphabet"], Some(0));
        let size = TerminalSize {
            width: 40,
            height: 12,
        };

        let layout =
            layout_completion_popup(&popup, size, 10, 4).expect("popup should fit below cursor");

        assert_eq!(layout.lines.len(), 3);
        assert_eq!(layout.layout.height, 3);
    }

    #[test]
    fn test_hover_popup_layout_wraps_long_lines() {
        let popup = HoverPopup::new(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnop",
        );
        let size = TerminalSize {
            width: 28,
            height: 12,
        };

        let layout =
            layout_hover_popup(&popup, size, 10, 4, size.content_height()).expect("hover popup");

        assert_eq!(layout.layout.start_y, 5);
        assert_eq!(layout.layout.width, 28);
        assert_eq!(layout.layout.height, 5);
        assert_eq!(layout.lines.len(), 5);
        assert_eq!(layout.lines[1].text, "│abcdefghijklmnopqrstuvwxyz│");
        assert_eq!(layout.lines[2].text, "│ABCDEFGHIJKLMNOPQRSTUVWXYZ│");
        assert_eq!(layout.lines[3].text, "│0123456789abcdefghijklmnop│");
    }

    #[test]
    /// Confirm large completion lists stop growing once they hit the popup height cap.
    fn test_completion_popup_layout_caps_height() {
        let labels = (0..20)
            .map(|index| format!("alpha{index}"))
            .collect::<Vec<_>>();
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        let popup = create_completion_popup(&label_refs, Some(0));
        let size = TerminalSize {
            width: 80,
            height: 30,
        };

        // The terminal has more than enough space, so the cap should be the limiting factor.
        let layout =
            layout_completion_popup(&popup, size, 10, 4).expect("popup should fit below cursor");

        assert_eq!(layout.layout.height, COMPLETION_POPUP_MAX_HEIGHT as u16);
    }

    #[test]
    /// Confirm the popup width stays fixed while the visible entry window scrolls.
    fn test_completion_popup_layout_keeps_width_stable_while_scrolling() {
        let labels = [
            "a0",
            "a1",
            "a2",
            "a3",
            "a4",
            "candidate_name_that_is_far_wider_than_the_rest_of_the_list",
        ];
        let initial_popup = create_completion_popup(&labels, Some(0));
        // The wide candidate stays off-screen in this first layout.
        let initial_layout = layout_completion_popup(
            &initial_popup,
            TerminalSize {
                width: 80,
                height: 9,
            },
            10,
            2,
        )
        .expect("initial popup should fit below cursor");
        let scrolled_popup = create_completion_popup(&labels, Some(5));
        // Scrolling to the wide candidate should not change the popup width.
        let scrolled_layout = layout_completion_popup(
            &scrolled_popup,
            TerminalSize {
                width: 80,
                height: 9,
            },
            10,
            2,
        )
        .expect("scrolled popup should fit below cursor");

        assert_eq!(initial_layout.layout.width, scrolled_layout.layout.width);
        assert_eq!(
            initial_layout.layout.width,
            COMPLETION_POPUP_MAX_WIDTH as u16
        );
    }

    #[test]
    /// Confirm async popup updates resize in place instead of moving the box origin.
    fn test_completion_popup_layout_resizes_async_results_without_moving_position() {
        let previous_popup = create_completion_popup(
            &[
                "candidate_name_that_is_far_wider_than_the_rest",
                "alpha1",
                "alpha2",
                "alpha3",
                "alpha4",
            ],
            Some(0),
        );
        let mut refreshed_popup = create_completion_popup(&["alpha"], Some(0));
        refreshed_popup.placement_entry_count = previous_popup.reserved_entry_count;
        refreshed_popup.placement_inner_width = previous_popup.reserved_inner_width;
        let size = TerminalSize {
            width: 24,
            height: 15,
        };

        let previous_layout = layout_completion_popup(&previous_popup, size, 22, 8)
            .expect("previous popup should fit");
        let refreshed_layout = layout_completion_popup(&refreshed_popup, size, 22, 8)
            .expect("refreshed popup should fit");

        assert_eq!(
            refreshed_layout.layout.start_x,
            previous_layout.layout.start_x
        );
        assert_eq!(
            refreshed_layout.layout.start_y,
            previous_layout.layout.start_y
        );
        assert!(refreshed_layout.layout.width < previous_layout.layout.width);
        assert!(refreshed_layout.layout.height < previous_layout.layout.height);
    }

    #[test]
    fn test_completion_popup_layout_scrolls_selected_entry_into_view() {
        let popup =
            create_completion_popup(&["alpha0", "alpha1", "alpha2", "alpha3", "alpha4"], Some(4));
        let size = TerminalSize {
            width: 40,
            height: 9,
        };

        let layout =
            layout_completion_popup(&popup, size, 10, 2).expect("popup should fit below cursor");

        assert!(layout.lines[1].text.contains("alpha2"));
        assert!(layout.lines[2].text.contains("alpha3"));
        assert!(layout.lines[3].text.contains("alpha4"));
        assert!(layout.lines[3].selected);
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
    fn test_trim_tab_path_label_preserves_file_name_and_initials() {
        assert_eq!(
            trim_tab_path_label("/src/tests/module.rs"),
            "/s/t/module.rs"
        );
    }

    #[test]
    fn test_trim_tab_path_label_preserves_special_buffer_labels() {
        assert_eq!(trim_tab_path_label("[No Name]"), "[No Name]");
    }

    #[test]
    fn test_render_layout_uses_minimum_gutter_digits() {
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, 9);
        assert_eq!(layout.gutter_digits, 3);
        assert_eq!(layout.gutter_total_width, 5);
        assert_eq!(layout.content_width, 75);
    }

    #[test]
    fn test_render_layout_expands_for_large_line_counts() {
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, 12_345);
        assert_eq!(layout.gutter_digits, 5);
        assert_eq!(layout.gutter_total_width, 7);
        assert_eq!(layout.content_width, 73);
    }

    #[test]
    fn test_trailing_cursor_cell_offset_reports_empty_cursor_cell() {
        let editor = EditorState::new(24);
        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: String::new(),
        };
        assert_eq!(trailing_cursor_cell_offset(&editor, &row, 10), Some(0));
    }

    #[test]
    fn test_trailing_cursor_cell_offset_ignores_real_content_cells() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("abc");
        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "abc".to_string(),
        };
        assert_eq!(trailing_cursor_cell_offset(&editor, &row, 10), None);
    }

    #[test]
    fn test_render_layout_clamps_content_width_to_zero() {
        let size = TerminalSize {
            width: 2,
            height: 24,
        };
        let layout = RenderLayout::from_size(size, 100);
        assert_eq!(layout.gutter_digits, 3);
        assert_eq!(layout.gutter_total_width, 5);
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
    fn test_cursor_color_uses_search_highlight_foreground_on_search_match() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("alpha beta");
        editor.handle_key(Key::Char('/'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('p'));
        editor.handle_key(Key::Char('h'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('\n'));
        editor.prepare_syntax_view(1);

        assert_eq!(
            cursor_color(&editor, tui::CursorShape::Block),
            editor.theme().search_match_style().fg
        );
    }

    #[test]
    fn test_render_row_content_visual_mode_uses_selection_background_for_cursor_cell() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("XYZ");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(termion::event::Key::Char('v'));
        editor.handle_key(termion::event::Key::Char('l'));
        let selection_bg = termion::color::AnsiValue(
            editor
                .theme()
                .selection_style()
                .bg
                .expect("selection style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "XYZ".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 10).into_owned();
        let underline_escape: &str = termion::style::Underline.as_ref();
        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains("XY"));
        assert!(
            rendered.contains(&selection_bg),
            "visual selections should still paint the configured selection background"
        );
        assert!(
            !rendered.contains(underline_escape),
            "visual selections should not underline the cursor cell anymore"
        );
    }

    #[test]
    fn test_render_row_content_visual_entry_selects_the_initial_cursor_cell() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("XYZ");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(termion::event::Key::Char('v'));
        let selection_bg = termion::color::AnsiValue(
            editor
                .theme()
                .selection_style()
                .bg
                .expect("selection style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "XYZ".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 10).into_owned();
        let underline_escape: &str = termion::style::Underline.as_ref();
        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains('X'));
        assert!(
            rendered.contains(&selection_bg),
            "visual entry should immediately paint the configured selection background"
        );
        assert!(
            !rendered.contains(underline_escape),
            "single-cell visual selections should not introduce underline styling"
        );
    }

    #[test]
    fn test_render_row_content_uses_undercurl_for_diagnostics() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("let broken = missing_name;");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        apply_render_test_diagnostic(&mut editor, "/tmp/render_diag.rs", 13, 25);

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "let broken = missing_name;".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 80).into_owned();
        assert!(rendered.contains("\u{1b}[4:3m"));
    }

    #[test]
    fn test_format_screen_row_gutter_marks_diagnostic_lines() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = TextBuffer::from_str("let broken = missing_name;");
        apply_render_test_diagnostic(&mut editor, "/tmp/render_diag.rs", 13, 25);
        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "let broken = missing_name;".to_string(),
        };

        let gutter = format_screen_row_gutter(&editor, &row, 2);
        assert_eq!(gutter.marker, '●');
        assert_eq!(gutter.number_text, " 1 ");
    }

    #[test]
    fn test_render_row_content_highlights_visible_matching_delimiters() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("(ab)");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.set_cursor(crate::cursor::Cursor::new(0, 0));
        editor.prepare_syntax_view(1);
        let bold_escape: &str = termion::style::Bold.as_ref();
        let reverse_escape = "\u{1b}[7m";

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "(ab)".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 10).into_owned();
        let bold_count = rendered.matches(bold_escape).count();

        assert!(
            rendered.contains(reverse_escape),
            "visible match target should enable reverse video"
        );
        assert!(
            bold_count >= 2,
            "both visible match endpoints should render in bold"
        );
    }

    #[test]
    fn test_render_row_content_highlights_search_preview_matches() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("alpha beta alpha");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(termion::event::Key::Char('/'));
        editor.handle_key(termion::event::Key::Char('a'));
        editor.handle_key(termion::event::Key::Char('l'));
        editor.handle_key(termion::event::Key::Char('p'));
        editor.handle_key(termion::event::Key::Char('h'));
        editor.handle_key(termion::event::Key::Char('a'));
        editor.prepare_syntax_view(1);
        let search_match_bg = termion::color::AnsiValue(
            editor
                .theme()
                .search_match_style()
                .bg
                .expect("search match style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "alpha beta alpha".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 20).into_owned();

        assert_eq!(editor.search_highlight_snapshot(), vec![(0, 5), (11, 16)]);
        assert!(
            rendered.matches(&search_match_bg).count() >= 2,
            "each search preview match should paint the configured highlight background"
        );
    }

    #[test]
    fn test_render_row_content_highlights_incomplete_substitute_pattern() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("alpha beta alpha");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(termion::event::Key::Char(':'));
        for ch in "s/alpha".chars() {
            editor.handle_key(termion::event::Key::Char(ch));
        }
        editor.prepare_syntax_view(1);
        let search_match_bg = termion::color::AnsiValue(
            editor
                .theme()
                .search_match_style()
                .bg
                .expect("search match style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "alpha beta alpha".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 20).into_owned();

        assert!(
            rendered.matches(&search_match_bg).count() >= 2,
            "typed substitute patterns should highlight matches before replacement is complete"
        );
    }

    #[test]
    fn test_render_row_content_keeps_syntax_during_substitute_preview() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() =
            crate::text_buffer::TextBuffer::from_str("fn foo() { let foo = 1; }\n");
        editor.set_startup_path("sample.rs");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(termion::event::Key::Char(':'));
        for ch in "%s/foo/bar".chars() {
            editor.handle_key(termion::event::Key::Char(ch));
        }
        editor.prepare_syntax_view(1);

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "fn bar() { let bar = 1; }".to_string(),
        };
        let plain = render_plain_row_content(&editor, &row.content, true).into_owned();
        let rendered = render_row_content(&editor, &row, 40).into_owned();

        assert_ne!(
            rendered, plain,
            "replacement preview should keep syntax-driven styling instead of falling back to plain text"
        );
    }

    #[test]
    fn test_render_row_content_highlights_replacement_preview_spans() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("foo foo\n");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(termion::event::Key::Char(':'));
        for ch in "s/foo/bar".chars() {
            editor.handle_key(termion::event::Key::Char(ch));
        }
        editor.prepare_syntax_view(1);
        let search_match_bg = termion::color::AnsiValue(
            editor
                .theme()
                .search_match_style()
                .bg
                .expect("search match style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "bar bar".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 20).into_owned();

        assert!(
            rendered.matches(&search_match_bg).count() >= 2,
            "previewed replacement text should stay temporarily highlighted until the command finishes"
        );
    }

    #[test]
    fn test_render_row_content_keeps_selected_match_target_bold_without_passive_background() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("(ab)");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.set_cursor(crate::cursor::Cursor::new(0, 3));
        editor.handle_key(termion::event::Key::Char('v'));
        editor.handle_key(termion::event::Key::Char('h'));
        editor.handle_key(termion::event::Key::Char('h'));
        editor.handle_key(termion::event::Key::Char('h'));
        editor.prepare_syntax_view(1);
        let selection_bg = termion::color::AnsiValue(
            editor
                .theme()
                .selection_style()
                .bg
                .expect("selection style should set a background")
                .ansi256_index(),
        )
        .bg_string();
        let bold_escape: &str = termion::style::Bold.as_ref();
        let reverse_escape = "\u{1b}[7m";

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "(ab)".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 10).into_owned();

        assert!(
            rendered.contains(&selection_bg),
            "visual selection should still paint the configured selection background"
        );
        assert!(
            !rendered.contains(reverse_escape),
            "selected match targets should not enable passive-match reverse video"
        );
        assert!(
            rendered.contains(bold_escape),
            "selected match targets should still render with bold emphasis"
        );
    }

    #[test]
    fn test_render_row_content_plain_text_uses_theme_background_style() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("    .viewport");
        editor.apply_config(&crate::config::ConfigSettings {
            theme: Some("catppuccin-latte".to_string()),
            ..crate::config::ConfigSettings::default()
        });
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "    .viewport".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 20).into_owned();
        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains(".viewport"));
    }

    #[test]
    fn test_row_background_style_uses_current_line_background_for_active_row() {
        let editor = EditorState::new(24);
        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "alpha".to_string(),
        };

        assert_eq!(
            row_background_style(&editor, &row).bg,
            editor.theme().current_line_style().bg
        );
    }

    #[test]
    fn test_render_row_content_current_line_uses_theme_current_line_background() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("alpha\nbeta");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.set_cursor(crate::cursor::Cursor::new(1, 0));
        let current_line_bg = termion::color::AnsiValue(
            editor
                .theme()
                .current_line_style()
                .bg
                .expect("current-line style should set a background")
                .ansi256_index(),
        )
        .bg_string();

        let row = ScreenRow {
            line_idx: Some(1),
            row_offset: 0,
            content: "beta".to_string(),
        };

        let rendered = render_row_content(&editor, &row, 20).into_owned();
        assert!(rendered.contains(&current_line_bg));
    }

    /// Verify that rendered rows keep syntax styling after many vertical scroll steps.
    #[test]
    fn test_render_row_content_keeps_multiline_comment_highlighting_after_scroll() {
        let mut source = String::from("/* open comment\n");

        // Match the integration test's long comment body so the viewport shifts
        // through several prepared syntax windows before reaching the target row.
        for _ in 0..199 {
            source.push_str("comment body\n");
        }
        source.push_str("*/\nlet value = 1;\n");
        let mut editor = EditorState::new(8);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str(&source);
        editor.set_startup_path("sample.rs");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.handle_key(Key::Ctrl('f'));
        for _ in 7..=47 {
            editor.handle_key(Key::Char('j'));
        }

        editor.prepare_syntax_view(6);
        let row = ScreenRow {
            line_idx: Some(44),
            row_offset: 0,
            content: "comment body".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 76).into_owned();

        assert!(
            rendered.contains("\u{1b}[38;5;249m"),
            "scrolled comment rows should keep comment coloring"
        );
    }

    /// Verify that cached visible spans still match a direct replay after repeated paging.
    #[test]
    fn test_main_rs_cached_comment_spans_match_direct_replay_after_ctrl_f() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/main_scroll_fixture.rs"
        ));
        editor.set_startup_path("main_scroll_fixture.rs");

        // Match the user's repro: opening `src/main.rs` and paging down four times.
        for _ in 0..4 {
            editor.handle_key(Key::Ctrl('f'));
        }
        editor.prepare_syntax_view(
            TerminalSize {
                width: 80,
                height: 24,
            }
            .content_height(),
        );

        let cached = editor.cached_syntax_spans_for_line(85);
        let direct = editor.compute_syntax_spans_for_line(85);
        assert_eq!(
            cached, direct,
            "cached spans for the visible comment line should match a direct replay"
        );
    }

    /// Verify that paging still renders built screen rows with comment styling.
    #[test]
    fn test_main_rs_built_screen_rows_keep_comment_style_after_ctrl_f() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/main_scroll_fixture.rs"
        ));
        editor.set_startup_path("main_scroll_fixture.rs");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let content_height = size.content_height();
        let _ = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(content_height);

        for _ in 0..4 {
            editor.handle_key(Key::Ctrl('f'));
            let _ = prepare_viewport_for_render(&mut editor, size);
            editor.prepare_syntax_view(content_height);
        }
        let layout = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(content_height);
        let screen_rows = build_screen_rows(&editor, content_height, layout.content_width);
        let row = screen_rows
            .iter()
            .find(|row| {
                row.line_idx.is_some_and(|line_idx| {
                    editor
                        .compute_syntax_spans_for_line(line_idx)
                        .iter()
                        .any(|span| span.class == crate::syntax::SyntaxClass::Comment)
                })
            })
            .expect("one visible syntax-comment row should remain after paging");
        let plain = render_plain_row_content(&editor, &row.content, false).into_owned();
        let rendered = render_row_content(&editor, row, layout.content_width).into_owned();

        assert!(
            rendered != plain,
            "paged comment row should render with syntax-specific styling"
        );
    }

    /// Verify that every visible cached line matches a direct replay after paging.
    #[test]
    fn test_main_rs_visible_cached_spans_match_direct_replay_after_four_ctrl_f() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/main_scroll_fixture.rs"
        ));
        editor.set_startup_path("main_scroll_fixture.rs");
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let content_height = size.content_height();

        // Mirror the real render loop so page-downs rebuild the same prepared
        // windows and viewport layout that the interactive editor uses.
        let _ = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(content_height);
        for _ in 0..4 {
            editor.handle_key(Key::Ctrl('f'));
            let _ = prepare_viewport_for_render(&mut editor, size);
            editor.prepare_syntax_view(content_height);
        }

        // Comparing every visible logical line catches stale checkpoint reuse
        // anywhere in the viewport instead of only on the known comment row.
        let layout = prepare_viewport_for_render(&mut editor, size);
        let screen_rows = build_screen_rows(&editor, content_height, layout.content_width);
        for line_idx in screen_rows.into_iter().filter_map(|row| row.line_idx) {
            let cached = editor.cached_syntax_spans_for_line(line_idx);
            let direct = editor.compute_syntax_spans_for_line(line_idx);
            assert_eq!(
                cached,
                direct,
                "visible line {} cached spans diverged from direct replay after four ctrl-f",
                line_idx + 1
            );
        }
    }

    /// Verify that scrolling past an inserted block-comment close keeps visible comment spans.
    #[test]
    fn test_inserted_block_comment_keeps_comment_spans_while_scrolling() {
        let mut editor = EditorState::new(8);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/editor_state_mod_scroll_fixture.rs"
        ));
        editor.set_startup_path("editor_state_mod_scroll_fixture.rs");
        let size = TerminalSize {
            width: 80,
            height: 8,
        };
        let content_height = size.content_height();

        // Prime the original buffer so the inserted delimiters follow the
        // incremental edit path that regressed during later scrolling.
        editor.handle_resize(size.width as usize, size.height as usize);
        let _ = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(content_height);

        // Insert the opening delimiter above the original first line.
        editor.handle_key(Key::Char('O'));
        editor.handle_key(Key::Char('/'));
        editor.handle_key(Key::Char('*'));
        editor.handle_key(Key::Esc);

        // Close the block comment on the blank line before the impl block.
        editor.set_cursor(crate::cursor::Cursor::new(163, 0));
        editor.handle_resize(size.width as usize, size.height as usize);
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('*'));
        editor.handle_key(Key::Char('/'));
        editor.handle_key(Key::Esc);

        // Scroll a few lines through the comment/code boundary and keep every
        // visible comment line aligned with a direct syntax replay.
        editor.set_cursor(crate::cursor::Cursor::new(155, 0));
        editor.handle_resize(size.width as usize, size.height as usize);
        for _ in 0..7 {
            editor.handle_key(Key::Char('j'));
            let _ = prepare_viewport_for_render(&mut editor, size);
            editor.prepare_syntax_view(content_height);
            let first_visible = editor.first_visible_line();
            let last_visible = first_visible + content_height.saturating_sub(1);

            for line_idx in first_visible..=last_visible.min(163) {
                let cached = editor.cached_syntax_spans_for_line(line_idx);
                let direct = editor.compute_syntax_spans_for_line(line_idx);
                assert_eq!(
                    cached,
                    direct,
                    "visible comment line {} should keep cached spans in sync",
                    line_idx + 1
                );
                assert!(
                    cached
                        .iter()
                        .any(|span| span.class == crate::syntax::SyntaxClass::Comment),
                    "visible comment line {} should keep comment highlighting",
                    line_idx + 1
                );
            }
        }
    }

    #[test]
    fn test_lsp_progress_overlay_lines_formats_borderless_overlay() {
        let lines = lsp_progress_overlay_lines(
            &[
                "Indexing: crate graph (5%)".to_string(),
                "Diagnostics: macros (73%)".to_string(),
                "rust-analyzer ⠋".to_string(),
            ],
            80,
            10,
        );

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "Indexing: crate graph (5%)");
        assert_eq!(lines[1], "Diagnostics: macros (73%)");
        assert_eq!(lines[2], "rust-analyzer ⠋");
    }
}
