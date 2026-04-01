//! Terminal rendering helpers and redraw decisions.

use crate::dialogs::{PickerPopup, PickerPopupEntry};
use crate::editor_state::{EditorState, SequenceDiscoveryPopup};
use crate::mode;
use crate::soft_wrap;
use crate::themes::ThemeStyle;
use crate::tui;
use std::borrow::Cow;
use std::io;

const MIN_GUTTER_DIGITS: usize = 3;
const GUTTER_SEPARATOR_WIDTH: usize = 1;
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
    let layout = RenderLayout::from_size(size, editor.buffer_line_count());
    editor.sync_viewport_width_for_render(layout.content_width.max(1));
    layout
}

/// Update editor viewport dimensions after a terminal resize.
pub(crate) fn resize_editor(editor: &mut EditorState, size: TerminalSize) {
    let layout = RenderLayout::from_size(size, editor.buffer_line_count());
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
        }
    }

    /// Return whether this mode paints the active cursor directly into content.
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
    buffer_lines: usize,
    buffer_chars: usize,
    syntax_generation: u64,
    theme_name: &'static str,
    visible_match: Option<(usize, usize, usize, usize)>,
    pending_prefix: Option<String>,
    input_prompt: Option<char>,
    input_line: Option<String>,
    input_cursor_col: Option<usize>,
    overwrite_prompt: Option<String>,
    quit_prompt: Option<String>,
    buffer_close_prompt: Option<String>,
    status_message: Option<String>,
    sequence_discovery_popup: Option<SequenceDiscoveryPopup>,
    picker_popup: Option<PickerPopup>,
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
            buffer_lines: editor.buffer_line_count(),
            buffer_chars: editor.buffer_char_count(),
            syntax_generation: editor.syntax_generation(),
            theme_name: editor.theme_name(),
            visible_match: editor.visible_match_snapshot(),
            pending_prefix: editor.pending_prefix_label(),
            input_prompt: editor.input_prompt(),
            input_line: editor.input_line().map(str::to_string),
            input_cursor_col: editor.input_cursor_column(),
            overwrite_prompt: editor.overwrite_prompt(),
            quit_prompt: editor.quit_prompt(),
            buffer_close_prompt: editor.buffer_close_prompt(),
            status_message: editor.status_message().map(str::to_string),
            sequence_discovery_popup: editor.sequence_discovery_popup(),
            picker_popup: editor.picker_popup(),
        }
    }

    /// Return the captured cursor line for targeted vertical redraw decisions.
    pub(crate) fn cursor_line(&self) -> usize {
        self.cursor_line
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
            && before.theme_name == after.theme_name
            && before.visible_match == after.visible_match
            && before.sequence_discovery_popup == after.sequence_discovery_popup
            && before.picker_popup == after.picker_popup;
        let message_changed = before.pending_prefix != after.pending_prefix
            || before.input_prompt != after.input_prompt
            || before.input_line != after.input_line
            || before.input_cursor_col != after.input_cursor_col
            || before.overwrite_prompt != after.overwrite_prompt
            || before.quit_prompt != after.quit_prompt
            || before.buffer_close_prompt != after.buffer_close_prompt
            || before.status_message != after.status_message;
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
            || before.mode != after.mode
            || before.file_name != after.file_name
            || before.modified != after.modified
            || before.buffer_lines != after.buffer_lines
            || before.buffer_chars != after.buffer_chars
            || before.syntax_generation != after.syntax_generation
            || before.theme_name != after.theme_name
            || before.visible_match != after.visible_match
            || before.sequence_discovery_popup != after.sequence_discovery_popup
            || before.picker_popup != after.picker_popup;

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
        if let Some(line) = editor.buffer().line_for_display(line_idx) {
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
        if let Some(line) = editor.buffer().line_for_display(line_idx) {
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
        editor.first_visible_column()
    }
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

    let selection_range = editor.selection_range();
    let syntax_spans = editor.syntax_spans_for_line(line_idx);
    if selection_range.is_none()
        && syntax_spans.is_empty()
        && !editor.line_has_visible_match(line_idx)
    {
        return render_plain_row_content(editor, &row.content);
    }

    let line_start = editor.buffer().line_to_char(line_idx);
    let row_start = screen_row_start_column(editor, row, content_width);
    let mut rendered = String::new();
    let mut active_style = None;
    let mut span_idx = 0;
    let theme = editor.theme();
    let color_capability = editor.color_capability();

    // Selection must layer on top of syntax colors without clobbering the
    // current syntax span when wrapping or scrolling clips a row.
    for (offset, ch) in row.content.chars().enumerate() {
        let char_idx = line_start + row_start + offset;
        let column = row_start + offset;
        let selected = selection_range.is_some_and(|(start, end)| (start..end).contains(&char_idx));
        let match_role = editor.visible_match_role(char_idx);
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
            match_role,
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
fn render_plain_row_content<'a>(editor: &EditorState, content: &'a str) -> Cow<'a, str> {
    if content.is_empty() {
        return Cow::Borrowed(content);
    }

    let mut rendered = String::with_capacity(content.len() + 32);
    let mut active_style = None;
    tui::push_styled_text(
        &mut rendered,
        &mut active_style,
        tui::CellStyle::default(),
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
    let line_len = editor.buffer().line_len(editor.cursor_line());
    // Convert the logical cursor into a visual row/column so rendering and
    // navigation share the same wrapped-layout interpretation.
    let cursor_visual = soft_wrap::visual_cursor(
        editor.cursor_column(),
        line_len,
        layout.content_width,
        editor.mode_uses_modal_bindings(),
        editor.cursor_line(),
    );
    let viewport_origin =
        soft_wrap::VisualPosition::new(editor.first_visible_line(), editor.first_visible_row());
    // The on-screen Y position is the number of wrapped rows between the
    // viewport origin and the cursor's wrapped row.
    let visual_row = soft_wrap::visual_rows_between(
        viewport_origin,
        cursor_visual.position,
        editor.buffer(),
        layout.content_width,
    );

    (
        // X is the gutter width plus the cursor's column inside its wrapped row.
        (layout.gutter_total_width + cursor_visual.column + 1) as u16,
        // Clamp to the last content row so the cursor never drops into the
        // status/message area even when the cursor sits just beyond the view.
        (visual_row.min(content_height.saturating_sub(1)) as u16) + CONTENT_START_ROW,
    )
}

/// Return the screen cursor position for non-wrapped rendering.
fn unwrapped_cursor_screen_position(editor: &EditorState, layout: RenderLayout) -> (u16, u16) {
    (
        // In unwrapped mode the horizontal position is just the logical column
        // relative to the leftmost visible buffer column.
        (layout.gutter_total_width
            + editor
                .cursor_column()
                .saturating_sub(editor.first_visible_column())
            + 1) as u16,
        // Each logical line maps to exactly one screen row in unwrapped mode.
        (editor
            .cursor_line()
            .saturating_sub(editor.first_visible_line()) as u16)
            + CONTENT_START_ROW,
    )
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
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    render_status_line(&mut batch, editor, size);
    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(theme.cursor_color(cursor_shape), color_capability);
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let content_height = size.content_height();
    let layout = RenderLayout::from_size(size, editor.buffer_line_count());
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    batch.goto(
        cursor_x.clamp(1, size.width),
        cursor_y.clamp(1, size.height),
    );
    term.write_batch(&batch)
}

/// Render the status line plus the gutters affected by a vertical cursor move.
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
    let layout = RenderLayout::from_size(size, editor.buffer_line_count());
    let cursor_shape = editor.cursor_shape();
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    editor.prepare_syntax_view(content_height);

    // Even this smaller multi-row update jumps through multiple gutter rows, so
    // hide the cursor while the batch is being applied to avoid visible stepping.
    if cursor_was_visible {
        batch.hide_cursor();
    }

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
    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(theme.cursor_color(cursor_shape), color_capability);
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
        if line_idx != previous_cursor_line && line_idx != editor.cursor_line() {
            continue;
        }

        // Only the old and new cursor gutters change on a stable vertical move,
        // so we can usually rewrite those cells without repainting the whole
        // content row. Empty cursor rows are the exception because they may
        // need one explicit space cell under the cursor to keep it visible.
        let y = CONTENT_START_ROW + row_index as u16;
        let gutter = format_screen_row_gutter(editor, screen_row, layout.gutter_digits);
        let gutter_style = theme.gutter_style(line_idx == editor.cursor_line());
        if screen_row.content.is_empty() {
            batch.clear_to_eol_styled_at(1, y, theme.background_style(), color_capability);
        }
        batch.write_styled_at(1, y, gutter_style, color_capability, &gutter);
        if screen_row.content.is_empty() {
            paint_trailing_cursor_cell(batch, editor, screen_row, layout, y);
        }
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
            editor.theme().background_style(),
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
    let theme = editor.theme();
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
        // Clear the row with the active theme background before repainting the
        // gutter and content. This preserves a fully themed backdrop without
        // streaming a full terminal-width run of spaces into every frame.
        batch.clear_to_eol_styled_at(1, y, theme.background_style(), color_capability);
        let gutter = format_screen_row_gutter(editor, screen_row, layout.gutter_digits);
        let content = render_row_content(editor, screen_row, layout.content_width);
        let gutter_width = gutter.chars().count() as u16;
        let gutter_style = if screen_row.line_idx.is_some() {
            theme.gutter_style(screen_row.line_idx == Some(editor.cursor_line()))
        } else {
            theme.eof_marker_style()
        };
        batch.write_styled_at(1, y, gutter_style, color_capability, &gutter);
        batch.write_at(1 + gutter_width, y, &content);
        paint_trailing_cursor_cell(&mut batch, editor, screen_row, layout, y);
    }

    render_status_line(&mut batch, editor, size);
    write_message_line(&mut batch, editor, size);
    let popup_layout = if let Some(popup) = editor.picker_popup() {
        Some(render_picker_popup(&mut batch, &popup, editor, size))
    } else {
        render_sequence_discovery_popup(&mut batch, editor, size)
    };

    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(theme.cursor_color(cursor_shape), color_capability);

    // Position cursor after all content so overlays can decide whether it must hide.
    let (cursor_x, cursor_y) = cursor_screen_position(editor, layout, content_height, size);
    let cursor_x = cursor_x.clamp(1, size.width);
    let cursor_y = cursor_y.clamp(1, size.height);
    let cursor_covered_by_popup = editor.picker_popup().is_none()
        && popup_layout.is_some_and(|popup| popup.covers(cursor_x, cursor_y));
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
        editor.cursor_line() + 1,
        editor.cursor_column() + 1
    );
    let modified = if editor.is_modified() { "[+] " } else { "" };
    let theme = editor.theme();
    let color_capability = editor.color_capability();
    let width = size.width as usize;
    let mode_segment = format!(" {} ", mode_str);
    let left_rest = format!(" {}{}", modified, editor.file_name());
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
        batch.set_cursor_color(editor.theme().cursor_color(cursor_shape), color_capability);
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
    batch.set_cursor_color(editor.theme().cursor_color(cursor_shape), color_capability);
    if *cursor_hidden_by_overlay {
        batch.show_cursor();
        *cursor_hidden_by_overlay = false;
    }
    let content_height = size.content_height();
    let layout = RenderLayout::from_size(size, editor.buffer_line_count());
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
    } else if let Some(prompt) = editor.buffer_close_prompt() {
        prompt
    } else if let (Some(prompt), Some(input)) = (editor.input_prompt(), editor.input_line()) {
        format!("{}{}", prompt, input)
    } else if let Some(msg) = editor.status_message() {
        msg.to_string()
    } else {
        String::new()
    };

    let pending_marker = editor.pending_prefix_label();
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

/// One rendered picker popup row with its selected-state styling hint.
#[derive(Clone)]
struct PickerPopupLine {
    text: String,
    selected: bool,
    active: bool,
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
    let active = if entry.active { '%' } else { ' ' };
    let modified = if entry.modified { '+' } else { ' ' };
    PickerPopupLine {
        text: format_popup_line(
            &format!(" {active}{modified} {} ", entry.label),
            inner_width,
        ),
        selected: entry.selected,
        active: entry.active,
    }
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
    use crate::dialogs::PickerPopup;
    use crate::mode::Mode;
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
                .open_buffer(&format!("/tmp/buffer_{index:02}.rs"))
                .expect("open named buffer");
        }
        editor
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
                active: false,
                modified: false,
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
                active: true,
                modified: false,
            },
            24,
        );

        assert!(line.text.contains("%  src/main.rs"));
        assert!(line.active);
        assert!(!line.selected);
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
                .any(|line| line.text.contains("buffer_00.rs"))
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
    fn test_render_row_content_highlights_visible_matching_delimiters() {
        let mut editor = EditorState::new(24);
        *editor.buffer_mut() = crate::text_buffer::TextBuffer::from_str("(ab)");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.set_cursor(crate::cursor::Cursor::new(0, 0));
        editor.prepare_syntax_view(1);
        let passive_match_bg = termion::color::AnsiValue(
            editor
                .theme()
                .passive_match_style()
                .bg
                .expect("passive match style should set a background")
                .ansi256_index(),
        )
        .bg_string();
        let bold_escape: &str = termion::style::Bold.as_ref();

        let row = ScreenRow {
            line_idx: Some(0),
            row_offset: 0,
            content: "(ab)".to_string(),
        };
        let rendered = render_row_content(&editor, &row, 10).into_owned();
        let bold_count = rendered.matches(bold_escape).count();

        assert!(
            rendered.contains(&passive_match_bg),
            "visible match target should paint the passive match background"
        );
        assert!(
            bold_count >= 2,
            "both visible match endpoints should render in bold"
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
        let passive_match_bg = termion::color::AnsiValue(
            editor
                .theme()
                .passive_match_style()
                .bg
                .expect("passive match style should set a background")
                .ansi256_index(),
        )
        .bg_string();
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
            !rendered.contains(&passive_match_bg),
            "selected match targets should not add the passive match background"
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
        let plain = render_plain_row_content(&editor, &row.content).into_owned();
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
}
