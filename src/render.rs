//! Terminal rendering helpers and redraw decisions.

use crate::editor_state::{EditorState, SequenceDiscoveryPopup};
use crate::mode;
use crate::soft_wrap;
use crate::tui;
use std::borrow::Cow;
use std::io;

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
pub(crate) struct TerminalSize {
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl TerminalSize {
    /// Clamp raw terminal dimensions to a small usable editing area.
    pub(crate) fn from_termion((width, height): (u16, u16)) -> Self {
        Self {
            width: width.max(1),
            height: height.max(3),
        }
    }

    /// Return the number of rows available for buffer content.
    fn content_height(self) -> usize {
        self.height.saturating_sub(RESERVED_BOTTOM_ROWS) as usize
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
    status_message: Option<String>,
    sequence_discovery_popup: Option<SequenceDiscoveryPopup>,
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
            mode: RenderMode::capture(&editor.mode),
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
            status_message: editor.status_message().map(str::to_string),
            sequence_discovery_popup: editor.sequence_discovery_popup(),
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
            && before.sequence_discovery_popup == after.sequence_discovery_popup;
        let message_changed = before.pending_prefix != after.pending_prefix
            || before.input_prompt != after.input_prompt
            || before.input_line != after.input_line
            || before.input_cursor_col != after.input_cursor_col
            || before.overwrite_prompt != after.overwrite_prompt
            || before.quit_prompt != after.quit_prompt
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

    let line_start = editor.buffer.line_to_char(line_idx);
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
    let line_len = editor.buffer.line_len(editor.cursor_line());
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
                .cursor_column()
                .saturating_sub(editor.first_visible_column())
            + 1) as u16,
        // Each logical line maps to exactly one screen row in unwrapped mode.
        (editor
            .cursor_line()
            .saturating_sub(editor.first_visible_line())
            + 1) as u16,
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
        let y = (row_index + 1) as u16;
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
    let popup_layout = render_sequence_discovery_popup(&mut batch, editor, size);

    batch.set_cursor_shape(cursor_shape);
    batch.set_cursor_color(theme.cursor_color(cursor_shape), color_capability);

    // Position cursor after all content so overlays can decide whether it must hide.
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
    use std::path::PathBuf;
    use termion::event::Key;

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
        editor.buffer = crate::text_buffer::TextBuffer::from_str("abc");
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str("XYZ");
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str("XYZ");
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str("(ab)");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.cursor = crate::cursor::Cursor::new(0, 0);
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str("(ab)");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.cursor = crate::cursor::Cursor::new(0, 3);
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str("    .viewport");
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str(&source);
        editor.file_path = PathBuf::from("sample.rs");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.refresh_syntax();
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
        editor.buffer = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/main_scroll_fixture.rs"
        ));
        editor.file_path = PathBuf::from("main_scroll_fixture.rs");
        editor.refresh_syntax();

        // Match the user's repro: opening `src/main.rs` and paging down four times.
        for _ in 0..4 {
            editor.handle_key(Key::Ctrl('f'));
        }
        editor.prepare_syntax_view(22);

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
        editor.buffer = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/main_scroll_fixture.rs"
        ));
        editor.file_path = PathBuf::from("main_scroll_fixture.rs");
        editor.set_color_capability(crate::themes::ColorCapability::Ansi256);
        editor.refresh_syntax();
        let size = TerminalSize {
            width: 80,
            height: 24,
        };
        let _ = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(22);

        for _ in 0..4 {
            editor.handle_key(Key::Ctrl('f'));
            let _ = prepare_viewport_for_render(&mut editor, size);
            editor.prepare_syntax_view(22);
        }
        let layout = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(22);
        let screen_rows = build_screen_rows(&editor, 22, layout.content_width);
        let row = screen_rows
            .iter()
            .find(|row| {
                row.line_idx == Some(85)
                    && row
                        .content
                        .contains("// Gutter-width changes alter the effective content width")
            })
            .expect("comment row should be visible after paging");
        let rendered = render_row_content(&editor, row, layout.content_width).into_owned();

        assert!(
            rendered.contains("\u{1b}[38;5;249m"),
            "paged comment row should render with comment coloring"
        );
    }

    /// Verify that every visible cached line matches a direct replay after paging.
    #[test]
    fn test_main_rs_visible_cached_spans_match_direct_replay_after_four_ctrl_f() {
        let mut editor = EditorState::new(24);
        editor.buffer = crate::text_buffer::TextBuffer::from_str(include_str!(
            "../tests/fixtures/syntax/main_scroll_fixture.rs"
        ));
        editor.file_path = PathBuf::from("main_scroll_fixture.rs");
        editor.refresh_syntax();
        let size = TerminalSize {
            width: 80,
            height: 24,
        };

        // Mirror the real render loop so page-downs rebuild the same prepared
        // windows and viewport layout that the interactive editor uses.
        let _ = prepare_viewport_for_render(&mut editor, size);
        editor.prepare_syntax_view(22);
        for _ in 0..4 {
            editor.handle_key(Key::Ctrl('f'));
            let _ = prepare_viewport_for_render(&mut editor, size);
            editor.prepare_syntax_view(22);
        }

        // Comparing every visible logical line catches stale checkpoint reuse
        // anywhere in the viewport instead of only on the known comment row.
        let layout = prepare_viewport_for_render(&mut editor, size);
        let screen_rows = build_screen_rows(&editor, 22, layout.content_width);
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
