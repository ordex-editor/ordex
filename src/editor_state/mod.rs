//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::config::ConfigSettings;
use crate::cursor::Cursor;
use crate::keybindings::{Action, ActionBinding, KeyBindings, KeyInput, SequenceMatch};
use crate::mode::{Mode, VisualKind};
use crate::navigation::{
    find_around_paren_span, find_inner_word_span, find_next_paragraph_line, find_next_word_start,
    find_prev_paragraph_line, find_prev_word_start, find_word_end,
};
use crate::soft_wrap;
use crate::syntax::{BufferEdit, HighlightSpan, SyntaxClass, SyntaxEngine};
use crate::text_buffer::TextBuffer;
use crate::themes;
use crate::tui;
use crate::viewport::Viewport;
use std::borrow::Cow;
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use termion::event::Key;

mod actions;
mod commands;
mod history;
mod matching;
mod view;
pub(crate) use matching::VisibleMatchRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindMotionKind {
    Find,
    Till,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FindMotion {
    kind: FindMotionKind,
    direction: FindDirection,
    count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastFind {
    motion: FindMotion,
    target: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastVisualSelection {
    anchor_char_idx: usize,
    cursor_char_idx: usize,
    kind: VisualKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOverwrite {
    target_path: PathBuf,
    update_file_path: bool,
    post_save_action: PostSaveAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingQuitConfirmation {
    post_save_action: PostSaveAction,
}

/// One buffer mutation stored inside an undoable transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HistoryEdit {
    /// Text inserted at `char_idx`.
    Insert { char_idx: usize, text: String },
    /// Text removed starting at `char_idx`.
    Remove { char_idx: usize, text: String },
}

/// One committed undo/redo step with cursor positions before and after the change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UndoTransaction {
    /// Cursor position before any edits in this transaction.
    before_cursor_char_idx: usize,
    /// Cursor position after all edits in this transaction.
    after_cursor_char_idx: usize,
    /// Ordered edit list describing the forward change.
    edits: Vec<HistoryEdit>,
}

/// In-progress undo transaction while an edit command is still being assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveUndoTransaction {
    /// Cursor position before the transaction started.
    before_cursor_char_idx: usize,
    /// Ordered edit list captured so far.
    edits: Vec<HistoryEdit>,
}

/// Direction for replaying one stored undo transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayDirection {
    Undo,
    Redo,
}

/// One normalized, exclusive selection range in buffer character coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionRange {
    /// First selected character index.
    start: usize,
    /// One-past-the-end selected character index.
    end: usize,
}

/// Distinguish characterwise and linewise unnamed-register contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YankKind {
    Character,
    Line,
}

/// Stored contents of the editor-owned unnamed paste buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct YankBuffer {
    text: String,
    kind: YankKind,
}

/// Direction for Vim-style before/after paste placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PastePosition {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverwriteBehavior {
    ConfirmIfDifferentPath,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostSaveAction {
    StayOpen,
    QuitOnSuccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredWrite {
    pub(crate) path: PathBuf,
    pub(crate) update_file_path: bool,
    pub(crate) quit_on_success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditorRequest {
    ReloadConfig,
    WriteBuffer(DeferredWrite),
}

/// Runtime editor settings that have built-in defaults and may be overridden by config.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSettings {
    scroll_margin: usize,
    horizontal_scroll_margin: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    sequence_discovery_popup: bool,
    theme_name: &'static str,
    color_capability: themes::ColorCapability,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            scroll_margin: Viewport::DEFAULT_SCROLL_MARGIN,
            horizontal_scroll_margin: Viewport::DEFAULT_HORIZONTAL_SCROLL_MARGIN,
            relative_line_numbers: false,
            soft_wrap: true,
            sequence_discovery_popup: true,
            theme_name: themes::DEFAULT_THEME_NAME,
            color_capability: themes::ColorCapability::Ansi256,
        }
    }
}

/// One line in the shortcut discovery popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequenceDiscoveryEntry {
    pub(crate) keys: String,
    pub(crate) action: String,
}

/// Popup view model for the currently pending multi-key sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequenceDiscoveryPopup {
    pub(crate) prefix: String,
    pub(crate) entries: Vec<SequenceDiscoveryEntry>,
}

/// Editor state holding all components for the editor session
pub(crate) struct EditorState {
    /// The text buffer containing file content
    buffer: TextBuffer,
    /// Current cursor position
    cursor: Cursor,
    /// Current editor mode
    mode: Mode,
    /// Anchor cursor recorded when entering visual mode.
    ///
    /// Visual selection is modeled as "anchor plus active cursor". The anchor
    /// stays fixed at the position where visual mode started, while `cursor`
    /// keeps moving with motions like `h`, `j`, `w`, or `fX`. The active
    /// selection range is derived from these two endpoints on demand so growing,
    /// shrinking, and switching between characterwise/linewise visual mode all
    /// share one consistent source of truth.
    visual_anchor: Option<Cursor>,
    /// Viewport for visible portion of document
    viewport: Viewport,
    /// Path to the file being edited
    file_path: PathBuf,
    /// Derived syntax-highlighting state for the current document.
    syntax: SyntaxEngine,
    /// Status message to display (cleared after one render)
    status_message: Option<String>,
    /// Runtime-rendered settings derived from config plus built-in defaults.
    settings: EditorSettings,
    /// Preferred wrapped-row column preserved across wrapped vertical motions.
    ///
    /// `Cursor::desired_column()` keeps a logical buffer column for line-based
    /// vertical movement, but soft-wrap navigation needs a different notion of
    /// "stay in the same column": the column inside the current wrapped screen
    /// row. When motion crosses through short lines or different wrap offsets,
    /// the logical column can change even though the visual goal should stay the
    /// same, so wrapped `j`/`k` keep this separate value.
    desired_visual_column: Option<usize>,
    /// Key bindings configuration
    keybindings: KeyBindings,
    /// Flag indicating the editor should quit
    should_quit: bool,
    /// Process exit status requested by the editor when quitting.
    quit_exit_code: i32,
    /// Last non-empty search pattern used by / search.
    last_search_pattern: Option<String>,
    /// Pending multi-key sequence in normal mode (e.g. 'g' waiting for continuation).
    pending_sequence: Vec<KeyInput>,
    /// Count prefix typed before a normal-mode command.
    pending_count: Option<usize>,
    /// Count prefix captured when entering a pending multi-key sequence.
    pending_sequence_count: Option<usize>,
    /// Motion-side count typed after an operator prefix like `d`/`c`.
    pending_sequence_motion_count: Option<usize>,
    /// Pending find/till motion waiting for a target character.
    pending_find: Option<FindMotion>,
    /// Last attempted character find/till motion used by ';' and ','.
    last_find: Option<LastFind>,
    /// Last visual selection that can be recreated via normal-mode `gv`.
    last_visual_selection: Option<LastVisualSelection>,
    /// Editor-owned unnamed register used by yank, delete, and paste actions.
    yank_buffer: Option<YankBuffer>,
    /// Pending overwrite confirmation for save commands targeting an existing file.
    pending_overwrite: Option<PendingOverwrite>,
    /// Pending quit confirmation for `:q` with unsaved changes.
    pending_quit_confirmation: Option<PendingQuitConfirmation>,
    /// `%`-matching cache and visible passive highlight state.
    matching: matching::MatchingState,
    /// Ignore trailing Escape bytes for a short window after input cursor movement.
    ignore_input_escape_cancel_until: Option<Instant>,
    /// One-shot request for work that must be deferred until after `handle_key`.
    ///
    /// `EditorState` owns editor-local state, but some commands need data or I/O
    /// owned by the outer application loop instead. `:reload-config` and
    /// command-driven file writes are the current examples: parsing commands and
    /// deciding when they should run belongs here, but resolving the active
    /// config path and touching the filesystem belongs in `app.rs`, where the
    /// CLI-derived config path and runtime resources are available. Keeping only
    /// a request token here prevents `EditorState` from taking on startup or
    /// process-level concerns, keeps input handling deterministic, and leaves
    /// the app loop as the single place that performs deferred side effects
    /// after one key has been fully processed.
    pending_request: Option<EditorRequest>,
    /// Undoable changes committed in the current editor session.
    undo_stack: Vec<UndoTransaction>,
    /// Changes that were undone and may still be replayed.
    redo_stack: Vec<UndoTransaction>,
    /// Transaction being assembled for the current edit command or insert session.
    active_undo: Option<ActiveUndoTransaction>,
    /// Undo-stack depth that corresponds to the last clean on-disk buffer state.
    ///
    /// When a save succeeds, the current `undo_stack.len()` becomes the new clean
    /// reference point. Undoing or redoing back to that same depth means the
    /// in-memory buffer text again matches what was last loaded from or written to
    /// disk during this editor session, so the modified flag can be cleared.
    saved_undo_depth: usize,
    /// Suppress history capture while undo/redo replays existing edits.
    replaying_history: bool,
}

/// Vertical direction for shared viewport and wrapped-row motion helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionDirection {
    Up,
    Down,
}

impl EditorState {
    const INPUT_ESCAPE_SUPPRESS_DURATION: Duration = Duration::from_millis(30);
    /// Maximum repeat count applied to repeat-style actions to keep execution bounded.
    const MAX_COUNT: usize = 999_999;
    const RESERVED_BOTTOM_ROWS: usize = 2;

    fn normalize_key(key: Key) -> Key {
        match key {
            Key::Char('\u{1b}') => Key::Esc,
            Key::Ctrl('[') => Key::Esc,
            other => other,
        }
    }

    /// Create a new editor state with an empty buffer
    pub(crate) fn new(terminal_height: usize) -> Self {
        let mut editor = Self {
            buffer: TextBuffer::new(),
            cursor: Cursor::new(0, 0),
            mode: Mode::Normal,
            visual_anchor: None,
            viewport: Viewport::new(terminal_height.saturating_sub(Self::RESERVED_BOTTOM_ROWS)),
            file_path: PathBuf::new(),
            syntax: SyntaxEngine::new(),
            status_message: None,
            settings: EditorSettings::default(),
            desired_visual_column: None,
            keybindings: KeyBindings::new(),
            should_quit: false,
            quit_exit_code: 0,
            last_search_pattern: None,
            pending_sequence: Vec::new(),
            pending_count: None,
            pending_sequence_count: None,
            pending_sequence_motion_count: None,
            pending_find: None,
            last_find: None,
            last_visual_selection: None,
            yank_buffer: None,
            pending_overwrite: None,
            pending_quit_confirmation: None,
            matching: matching::MatchingState::new(),
            ignore_input_escape_cancel_until: None,
            pending_request: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            active_undo: None,
            saved_undo_depth: 0,
            replaying_history: false,
        };
        editor.apply_runtime_settings();
        editor
    }

    /// Apply resolved configuration settings to the current editor state.
    pub(crate) fn apply_config(&mut self, settings: &ConfigSettings) {
        if let Some(margin) = settings.scroll_margin {
            self.settings.scroll_margin = margin;
        }

        if let Some(margin) = settings.horizontal_scroll_margin {
            self.settings.horizontal_scroll_margin = margin;
        }

        if let Some(enabled) = settings.relative_line_numbers {
            self.settings.relative_line_numbers = enabled;
        }

        if let Some(enabled) = settings.soft_wrap {
            self.settings.soft_wrap = enabled;
        }

        if let Some(enabled) = settings.sequence_discovery_popup {
            self.settings.sequence_discovery_popup = enabled;
        }

        if let Some(theme_name) = settings.theme.as_deref()
            && let Some(theme) = themes::find(theme_name)
        {
            self.settings.theme_name = theme.name;
        }

        self.apply_runtime_settings();

        for binding in &settings.key_bindings {
            self.keybindings.set_binding_action_binding(
                binding.mode,
                binding.key.clone(),
                binding.actions.clone(),
            );
        }
        for binding in &settings.sequence_bindings {
            self.keybindings.set_sequence_binding_action_binding(
                binding.mode,
                binding.keys.clone(),
                binding.actions.clone(),
            );
        }
    }

    /// Replace all runtime-configurable state with a fresh config snapshot.
    ///
    /// Reloads must reset back to built-in defaults first so removed settings and
    /// key bindings stop taking effect immediately.
    pub(crate) fn replace_config(&mut self, settings: &ConfigSettings) {
        let color_capability = self.settings.color_capability;
        // Config reload should reset only config-derived settings. Terminal color
        // capability is detected from the environment and must survive reloads.
        self.settings = EditorSettings {
            color_capability,
            ..EditorSettings::default()
        };
        self.desired_visual_column = None;
        self.keybindings = KeyBindings::new();
        self.apply_config(settings);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.refresh_syntax();
    }

    /// Synchronize runtime settings onto subsystems that store the active values.
    fn apply_runtime_settings(&mut self) {
        self.viewport.set_scroll_margin(self.settings.scroll_margin);
        self.viewport.set_soft_wrap(self.settings.soft_wrap);
        self.viewport
            .set_horizontal_scroll_margin(self.settings.horizontal_scroll_margin);
    }

    /// Load a file into the editor using chunked reading for efficiency
    pub(crate) fn load_file(&mut self, path: &str) -> std::io::Result<()> {
        let file = File::open(path)?;
        self.buffer = TextBuffer::from_reader(file)?;
        self.file_path = PathBuf::from(path);
        self.cursor = Cursor::new(0, 0);
        self.desired_visual_column = None;
        self.viewport.set_first_visible_line(0);
        self.refresh_syntax();
        self.reset_history();
        Ok(())
    }

    /// Insert `text` at `char_idx` and notify the syntax engine about the edit.
    fn insert_buffer_text(&mut self, char_idx: usize, text: &str) {
        self.ensure_insert_history_transaction();
        if !self.replaying_history {
            self.record_history_insert(char_idx, text);
        }
        let start_line = self
            .buffer
            .char_to_line(char_idx.min(self.buffer.chars_count()));
        self.buffer.insert(char_idx, text);
        self.syntax.apply_edit(BufferEdit {
            start_line,
            old_end_line: start_line,
            new_end_line: start_line + text.chars().filter(|&c| c == '\n' || c == '\r').count(),
        });
        self.clear_match_state();
    }

    /// Remove one character-index range and notify the syntax engine about the edit.
    fn remove_buffer_range(&mut self, start_char: usize, end_char: usize) {
        if start_char >= end_char {
            return;
        }
        self.ensure_insert_history_transaction();
        if !self.replaying_history {
            self.record_history_remove(start_char, self.buffer.slice_string(start_char, end_char));
        }
        let start_line = self.buffer.char_to_line(start_char);
        let old_end_line = self.removal_old_end_line(start_char, end_char);
        self.buffer.remove(start_char, end_char);
        self.syntax.apply_edit(BufferEdit {
            start_line,
            old_end_line,
            new_end_line: start_line,
        });
        self.clear_match_state();
    }

    /// Return the last pre-edit line affected by a removal range.
    fn removal_old_end_line(&self, start_char: usize, end_char: usize) -> usize {
        let last_deleted_line = self.buffer.char_to_line(end_char.saturating_sub(1));

        // Removing a line break merges the following logical line into the start
        // line, so the syntax cache splice must also include that following line.
        if (start_char..end_char).any(|char_idx| {
            self.buffer
                .char_at(char_idx)
                .is_some_and(|ch| ch == '\n' || ch == '\r')
        }) {
            return (last_deleted_line + 1).min(self.buffer.lines_count().saturating_sub(1));
        }

        last_deleted_line
    }

    /// Return whether the given character is any supported logical line break.
    fn is_line_break(ch: char) -> bool {
        matches!(ch, '\n' | '\r')
    }

    /// Return whether the provided text already ends with a line break.
    fn text_ends_with_line_break(text: &str) -> bool {
        text.chars().last().is_some_and(Self::is_line_break)
    }

    /// Convert one visual selection kind into the matching unnamed-register shape.
    fn yank_kind_for_visual(kind: VisualKind) -> YankKind {
        match kind {
            VisualKind::Character => YankKind::Character,
            VisualKind::Line => YankKind::Line,
        }
    }

    /// Copy one buffer range into the unnamed register with the requested shape.
    fn store_yank_range(&mut self, selection: SelectionRange, kind: YankKind) {
        self.yank_buffer = Some(YankBuffer {
            text: self.buffer.slice_string(selection.start, selection.end),
            kind,
        });
    }

    /// Delete one buffer range after first copying it into the unnamed register.
    fn delete_range_into_yank_buffer(&mut self, selection: SelectionRange, kind: YankKind) {
        self.store_yank_range(selection, kind);
        if selection.end > selection.start {
            self.remove_buffer_range(selection.start, selection.end);
        }
    }

    /// Return the current linewise selection range for `yy`-style commands.
    fn current_line_range(&self, count: usize) -> SelectionRange {
        let start_line = self.cursor.line();
        let end_line_exclusive = start_line.saturating_add(count.max(1));
        let bounded_end_line = end_line_exclusive.min(self.buffer.lines_count());
        let start = self.buffer.line_to_char(start_line);
        let end = if bounded_end_line < self.buffer.lines_count() {
            self.buffer.line_to_char(bounded_end_line)
        } else {
            self.buffer.chars_count()
        };
        SelectionRange { start, end }
    }

    /// Build the text inserted by one linewise paste, adding any leading or
    /// trailing newline needed to place the payload before a line or after EOF.
    fn linewise_insertion_text<'a>(
        &self,
        text: &'a str,
        insert_before_existing_line: bool,
    ) -> Cow<'a, str> {
        let mut insertion = String::new();

        // Appending a linewise payload at EOF needs a separator when the current
        // buffer does not already end with a logical line break.
        if !insert_before_existing_line
            && self.buffer.chars_count() > 0
            && self
                .buffer
                .char_at(self.buffer.chars_count() - 1)
                .is_some_and(|ch| !Self::is_line_break(ch))
        {
            insertion.push('\n');
        }

        if insertion.is_empty() && !insert_before_existing_line {
            if text.is_empty() {
                return Cow::Borrowed("\n");
            }
            return Cow::Borrowed(text);
        }
        insertion.push_str(text);
        if insert_before_existing_line && !Self::text_ends_with_line_break(&insertion) {
            insertion.push('\n');
        }
        if insertion.is_empty() {
            insertion.push('\n');
        }
        Cow::Owned(insertion)
    }

    /// Paste one captured payload according to Vim-style before/after semantics.
    fn paste_payload(&mut self, payload: &YankBuffer, position: PastePosition) {
        match payload.kind {
            YankKind::Character => self.paste_characterwise(&payload.text, position),
            YankKind::Line => self.paste_linewise(&payload.text, position),
        }
    }

    /// Paste one characterwise payload before or after the cursor.
    fn paste_characterwise(&mut self, text: &str, position: PastePosition) {
        if text.is_empty() {
            return;
        }
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let insert_idx = match position {
            PastePosition::After if self.buffer.line_len(self.cursor.line()) > 0 => char_idx + 1,
            PastePosition::Before | PastePosition::After => char_idx,
        };
        self.insert_buffer_text(insert_idx, text);
        let last_inserted = insert_idx + text.chars().count().saturating_sub(1);
        self.cursor = Cursor::from_char_index(&self.buffer, last_inserted);
    }

    /// Paste one linewise payload above or below the current line.
    fn paste_linewise(&mut self, text: &str, position: PastePosition) {
        let target_line = match position {
            PastePosition::After => self.cursor.line().saturating_add(1),
            PastePosition::Before => self.cursor.line(),
        };
        let insert_before_existing_line = target_line < self.buffer.lines_count();

        // Linewise pastes preserve whole-line structure, so synthesize only the
        // separator that is missing at the chosen insertion boundary.
        let insertion = self.linewise_insertion_text(text, insert_before_existing_line);
        let insert_idx = if insert_before_existing_line {
            self.buffer.line_to_char(target_line)
        } else {
            self.buffer.chars_count()
        };
        self.insert_buffer_text(insert_idx, &insertion);
        self.cursor = Cursor::new(
            target_line.min(self.buffer.lines_count().saturating_sub(1)),
            0,
        );
    }

    /// Yank the current visual selection into the unnamed register and leave Visual mode.
    fn yank_visual_selection(&mut self) {
        let Some((selection, kind)) = self.normalized_selection() else {
            return;
        };
        self.store_yank_range(selection, Self::yank_kind_for_visual(kind));
        self.exit_visual_mode();
    }

    /// Yank the current line, and optionally following lines, into the unnamed register.
    fn yank_current_line_count(&mut self, count: usize) {
        let selection = self.current_line_range(count);
        self.store_yank_range(selection, YankKind::Line);
    }

    /// Yank the current line into the unnamed register.
    fn yank_current_line(&mut self) {
        self.yank_current_line_count(1);
    }

    /// Paste the unnamed register before or after the cursor according to Vim-style rules.
    fn paste_from_yank_buffer(&mut self, position: PastePosition) {
        self.with_history_transaction(|editor| {
            let Some(payload) = editor.yank_buffer.take() else {
                editor.status_message = Some("Nothing to paste".to_string());
                return;
            };
            editor.paste_payload(&payload, position);
            editor.yank_buffer = Some(payload);
        });
    }

    /// Repeat one paste action up to `count` times and stop after the first no-op.
    fn paste_from_yank_buffer_count(&mut self, position: PastePosition, count: usize) {
        self.with_history_transaction(|editor| {
            let Some(payload) = editor.yank_buffer.take() else {
                editor.status_message = Some("Nothing to paste".to_string());
                return;
            };
            for _ in 0..count {
                let before = editor.buffer.chars_count();
                editor.paste_payload(&payload, position);
                if editor.buffer.chars_count() == before {
                    break;
                }
            }
            editor.yank_buffer = Some(payload);
        });
    }

    fn insert_char(&mut self, c: char) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.insert_buffer_text(char_idx, &c.to_string());
        self.cursor.move_right(&self.buffer);
    }

    /// Insert one newline at the cursor and keep syntax state in sync.
    fn insert_newline(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.insert_buffer_text(char_idx, "\n");
        self.cursor.move_down(&self.buffer);
        self.cursor.set_column(0);
    }

    /// Open a new line below the cursor and enter insert mode.
    fn open_line_below(&mut self) {
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.insert_buffer_text(line_end, "\n");
        self.cursor = Cursor::new(line + 1, 0);
        self.enter_insert_mode();
    }

    fn insert_after_cursor(&mut self) {
        self.begin_history_transaction();
        let line_len = self.buffer.line_len(self.cursor.line());
        if line_len > 0 {
            self.cursor.move_right(&self.buffer);
        }
        self.enter_insert_mode();
    }

    /// Open a new line above the cursor and enter insert mode.
    fn open_line_above(&mut self) {
        self.begin_history_transaction();
        let line = self.cursor.line();
        let line_start = self.buffer.line_to_char(line);
        self.insert_buffer_text(line_start, "\n");
        self.cursor = Cursor::new(line, 0);
        self.enter_insert_mode();
    }

    /// Delete one character backward in insert mode.
    fn delete_char_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx > 0 {
            self.cursor.move_left(&self.buffer);
            self.remove_buffer_range(char_idx - 1, char_idx);
        }
    }

    /// Delete one character forward in insert mode.
    fn delete_char_forward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx < self.buffer.chars_count() {
            self.remove_buffer_range(char_idx, char_idx + 1);
        }
    }

    /// Delete the character under the cursor in normal mode.
    fn delete_char_at_cursor(&mut self) {
        self.delete_char_at_cursor_count(1);
    }

    /// Delete up to `count` characters from the cursor to line end for counted `x`.
    fn delete_char_at_cursor_count(&mut self, count: usize) {
        self.with_history_transaction(|editor| {
            let line_start = editor.buffer.line_to_char(editor.cursor.line());
            let char_idx = editor.cursor.to_char_index(&editor.buffer);
            let line_len = editor.buffer.line_len(editor.cursor.line());
            if line_len == 0 {
                return;
            }
            let line_end = line_start + line_len;
            let end = char_idx.saturating_add(count).min(line_end);
            editor.delete_range_into_yank_buffer(
                SelectionRange {
                    start: char_idx,
                    end,
                },
                YankKind::Character,
            );
        });
    }

    /// Delete one word backward in insert mode.
    fn delete_word_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx == 0 {
            return;
        }

        let word_start = find_prev_word_start(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, word_start);
        self.remove_buffer_range(word_start, char_idx);
    }

    /// Delete from the cursor back to the start of the line in insert mode.
    fn delete_to_line_start(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let line = self.cursor.line();
        let col = self.cursor.column();
        if col == 0 {
            return;
        }

        // Get the start of the current line in char index
        let line_start = self.buffer.line_to_char(line);
        let char_idx = self.cursor.to_char_index(&self.buffer);

        self.cursor.set_column(0);
        self.remove_buffer_range(line_start, char_idx);
    }

    fn delete_input_char(&mut self) {
        self.mode.pop_char();
    }

    fn delete_input_char_forward(&mut self) {
        self.mode.delete_input_char_forward();
    }

    fn delete_input_word_backward(&mut self) {
        self.mode.delete_input_word_backward();
    }

    fn delete_input_to_start(&mut self) {
        self.mode.delete_input_to_start();
    }

    fn delete_input_to_end(&mut self) {
        self.mode.delete_input_to_end();
    }

    fn move_input_start(&mut self) {
        self.mode.move_input_start();
        self.ignore_input_escape_cancel_until =
            Some(Instant::now() + Self::INPUT_ESCAPE_SUPPRESS_DURATION);
    }

    fn move_input_end(&mut self) {
        self.mode.move_input_end();
        self.ignore_input_escape_cancel_until =
            Some(Instant::now() + Self::INPUT_ESCAPE_SUPPRESS_DURATION);
    }

    fn move_input_left(&mut self) {
        self.mode.move_input_left();
        self.ignore_input_escape_cancel_until =
            Some(Instant::now() + Self::INPUT_ESCAPE_SUPPRESS_DURATION);
    }

    fn move_input_right(&mut self) {
        self.mode.move_input_right();
        self.ignore_input_escape_cancel_until =
            Some(Instant::now() + Self::INPUT_ESCAPE_SUPPRESS_DURATION);
    }

    fn move_input_word_left(&mut self) {
        self.mode.move_input_word_left();
        self.ignore_input_escape_cancel_until =
            Some(Instant::now() + Self::INPUT_ESCAPE_SUPPRESS_DURATION);
    }

    fn move_input_word_right(&mut self) {
        self.mode.move_input_word_right();
        self.ignore_input_escape_cancel_until =
            Some(Instant::now() + Self::INPUT_ESCAPE_SUPPRESS_DURATION);
    }

    fn delete_inner_word(&mut self) {
        self.with_history_transaction(|editor| {
            if !editor.mode.is_normal() {
                return;
            }

            let cursor_idx = editor.cursor.to_char_index(&editor.buffer);
            let Some((start, end)) = find_inner_word_span(&editor.buffer, cursor_idx) else {
                return;
            };

            if start >= end {
                return;
            }

            editor
                .delete_range_into_yank_buffer(SelectionRange { start, end }, YankKind::Character);
            editor.cursor = Cursor::from_char_index(&editor.buffer, start);
        });
    }

    /// Repeat `diw` semantics up to `count` times and stop at the first no-op.
    fn delete_inner_word_count(&mut self, count: usize) {
        self.with_history_transaction(|editor| {
            for _ in 0..count {
                let before = editor.buffer.chars_count();
                editor.delete_inner_word();
                if editor.buffer.chars_count() == before {
                    break;
                }
            }
        });
    }

    fn change_inner_word(&mut self) {
        if !self.mode.is_normal() {
            return;
        }

        let before = self.buffer.chars_count();
        self.begin_history_transaction();
        self.delete_inner_word();
        if self.buffer.chars_count() < before {
            self.enter_insert_mode();
        } else {
            self.finish_history_transaction();
        }
    }

    /// Repeat `ciw` deletions up to `count` times, then enter insert if anything changed.
    fn change_inner_word_count(&mut self, count: usize) {
        let before_total = self.buffer.chars_count();
        self.begin_history_transaction();
        self.delete_inner_word_count(count);
        if self.buffer.chars_count() < before_total {
            self.enter_insert_mode();
        } else {
            self.finish_history_transaction();
        }
    }

    fn delete_around_paren(&mut self) {
        self.with_history_transaction(|editor| {
            if !editor.mode.is_normal() {
                return;
            }

            let cursor_idx = editor.cursor.to_char_index(&editor.buffer);
            let Some((start, end)) = find_around_paren_span(&editor.buffer, cursor_idx) else {
                return;
            };

            if start >= end {
                return;
            }

            editor
                .delete_range_into_yank_buffer(SelectionRange { start, end }, YankKind::Character);
            editor.cursor = Cursor::from_char_index(&editor.buffer, start);
        });
    }

    /// Repeat `da(` semantics up to `count` times and stop at the first no-op.
    fn delete_around_paren_count(&mut self, count: usize) {
        self.with_history_transaction(|editor| {
            for _ in 0..count {
                let before = editor.buffer.chars_count();
                editor.delete_around_paren();
                if editor.buffer.chars_count() == before {
                    break;
                }
            }
        });
    }

    /// Delete the active visual selection and optionally enter insert mode.
    fn delete_visual_selection(&mut self, enter_insert: bool) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some((selection, kind)) = self.normalized_selection() else {
            return;
        };

        self.begin_history_transaction();
        self.delete_range_into_yank_buffer(selection, Self::yank_kind_for_visual(kind));

        // Characterwise deletion resumes at the removed span, while linewise
        // deletion snaps to column 0 on the first affected line.
        self.cursor = match kind {
            VisualKind::Character => {
                let target = selection.start.min(self.buffer.chars_count());
                Cursor::from_char_index(&self.buffer, target)
            }
            VisualKind::Line => {
                let target = selection.start.min(self.buffer.chars_count());
                Cursor::new(self.buffer.char_to_line(target), 0)
            }
        };

        self.last_visual_selection = Some(saved_selection);
        if enter_insert {
            self.clear_visual_mode(Mode::Insert);
        } else {
            self.clear_visual_mode(Mode::Normal);
            self.finish_history_transaction();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use std::fs;
    use test_utils::TempFile;

    fn create_editor_with_content(content: &str) -> EditorState {
        let mut editor = EditorState::new(24);
        editor.buffer = TextBuffer::from_str(content);
        editor
    }

    /// Handle one key and execute any deferred write requests for unit tests.
    #[cfg(test)]
    fn handle_key_and_flush_requests(editor: &mut EditorState, key: Key) {
        editor.handle_key(key);
        flush_pending_requests(editor);
    }

    /// Execute queued write requests with the same filesystem boundary as the app.
    #[cfg(test)]
    fn flush_pending_requests(editor: &mut EditorState) {
        while let Some(request) = editor.take_pending_request() {
            match request {
                EditorRequest::ReloadConfig => {
                    panic!("unit tests should assert reload requests directly")
                }
                EditorRequest::WriteBuffer(write) => app::execute_deferred_write(editor, write),
            }
        }
    }

    /// Build one editor with syntax detection enabled for `path`.
    fn create_syntax_editor(content: &str, path: &str) -> EditorState {
        let mut editor = create_editor_with_content(content);
        editor.file_path = PathBuf::from(path);
        editor.refresh_syntax();
        editor
    }

    #[test]
    fn test_hjkl_navigation() {
        let mut editor = create_editor_with_content("hello\nworld\ntest");

        // Move right
        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.cursor.column(), 1);

        // Move down
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 1);

        // Move left
        editor.handle_key(Key::Char('h'));
        assert_eq!(editor.cursor.column(), 0);

        // Move up
        editor.handle_key(Key::Char('k'));
        assert_eq!(editor.cursor.line(), 0);
    }

    #[test]
    fn test_word_navigation() {
        let mut editor = create_editor_with_content("hello world test");

        // Move to next word
        editor.handle_key(Key::Char('w'));
        assert_eq!(editor.cursor.column(), 6); // 'w' of world

        // Move to next word again
        editor.handle_key(Key::Char('w'));
        assert_eq!(editor.cursor.column(), 12); // 't' of test

        // Move back
        editor.handle_key(Key::Char('b'));
        assert_eq!(editor.cursor.column(), 6); // 'w' of world
    }

    #[test]
    fn test_enter_insert_mode() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('i'));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_exit_insert_mode() {
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Esc);
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_user_repro_sequence_with_ctrl_left_bracket_escape_variant() {
        let mut editor = create_editor_with_content("One line");

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));
        editor.handle_key(Key::Char('C'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Ctrl('['));

        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_alt_key_in_insert_mode_is_noop() {
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Alt('h'));

        assert!(matches!(editor.mode, Mode::Insert));
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_open_line_below_enters_insert_mode() {
        let mut editor = create_editor_with_content("line1\nline2");
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "line1\n\nline2");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_above_enters_insert_mode() {
        let mut editor = create_editor_with_content("line1\nline2");
        editor.cursor = Cursor::new(1, 3);

        editor.handle_key(Key::Char('O'));

        assert_eq!(editor.buffer.to_string(), "line1\n\nline2");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_insert_character() {
        let mut editor = create_editor_with_content("hllo");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('e'));
        assert_eq!(editor.buffer.to_string(), "hello");
    }

    #[test]
    fn test_undo_groups_insert_session_until_escape() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Char('Y'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "XYhello");
        editor.handle_key(Key::Char('u'));
        assert_eq!(editor.buffer.to_string(), "hello");
        assert_eq!(editor.cursor, Cursor::new(0, 0));
    }

    #[test]
    fn test_redo_replays_last_undone_change() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('u'));
        editor.handle_key(Key::Ctrl('r'));

        assert_eq!(editor.buffer.to_string(), "Xhello");
        assert_eq!(editor.cursor, Cursor::new(0, 0));
    }

    #[test]
    fn test_undo_and_redo_track_saved_state_across_writes() {
        let file = TempFile::new().expect("create temp file");
        let mut editor = create_editor_with_content("hello");
        editor.file_path = file.path().to_path_buf();

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Esc);
        assert!(editor.buffer.is_modified());

        editor.request_save_current(
            OverwriteBehavior::ConfirmIfDifferentPath,
            PostSaveAction::StayOpen,
        );
        flush_pending_requests(&mut editor);
        assert!(!editor.buffer.is_modified());

        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('Y'));
        editor.handle_key(Key::Esc);
        assert!(editor.buffer.is_modified());

        editor.handle_key(Key::Char('u'));
        assert_eq!(editor.buffer.to_string(), "Xhello");
        assert!(!editor.buffer.is_modified());

        editor.handle_key(Key::Ctrl('r'));
        assert_eq!(editor.buffer.to_string(), "XYhello");
        assert!(editor.buffer.is_modified());
    }

    #[test]
    fn test_undo_open_line_below_removes_auto_inserted_newline() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Esc);
        assert_eq!(editor.buffer.to_string(), "line1\n\nline2");

        editor.handle_key(Key::Char('u'));
        assert_eq!(editor.buffer.to_string(), "line1\nline2");
    }

    #[test]
    fn test_undo_visual_delete_restores_removed_text() {
        let mut editor = create_editor_with_content("abcd\n");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('d'));
        assert_eq!(editor.buffer.to_string(), "cd\n");

        editor.handle_key(Key::Char('u'));
        assert_eq!(editor.buffer.to_string(), "abcd\n");
    }

    #[test]
    fn test_command_mode_supports_undo_and_redo_commands() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Esc);

        editor.handle_key(Key::Char(':'));
        for ch in "undo".chars() {
            editor.handle_key(Key::Char(ch));
        }
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.buffer.to_string(), "hello");

        editor.handle_key(Key::Char(':'));
        for ch in "redo".chars() {
            editor.handle_key(Key::Char(ch));
        }
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.buffer.to_string(), "Xhello");
    }

    #[test]
    fn test_remove_newline_shrinks_syntax_cache_with_merged_lines() {
        let mut editor = create_editor_with_content("let alpha = 1;\nlet beta = 2;");
        editor.file_path = PathBuf::from("sample.rs");
        editor.refresh_syntax();

        let newline_idx = editor.buffer.line_to_char(0) + editor.buffer.line_len(0);
        editor.remove_buffer_range(newline_idx, newline_idx + 1);
        editor.prepare_syntax_view(1);

        assert_eq!(editor.buffer.lines_count(), 1);
        assert!(editor.syntax.document_state().checkpoint_count() >= 1);
        assert_eq!(editor.syntax.document_state().span_window_line_count(), 1);
        assert!(
            editor.syntax_spans_for_line(0).iter().any(|span| {
                span.class == crate::syntax::SyntaxClass::Keyword
                    || span.class == crate::syntax::SyntaxClass::Number
            }),
            "merged line should still retain syntax spans"
        );
        assert!(
            editor.syntax_spans_for_line(1).is_empty(),
            "stale spans for the removed line must be dropped"
        );
    }

    #[test]
    fn test_command_mode() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char(':'));
        assert!(matches!(editor.mode, Mode::Command(_)));

        editor.handle_key(Key::Char('q'));
        if let Mode::Command(ref input) = editor.mode {
            assert_eq!(input.text(), "q");
        }

        editor.handle_key(Key::Char('\n'));
        assert!(editor.should_quit);
    }

    #[test]
    fn test_command_input_ctrl_a_ctrl_e_inserts_at_cursor() {
        let mut editor = create_editor_with_content("hello");
        editor.handle_key(Key::Char(':'));
        for c in "wq".chars() {
            editor.handle_key(Key::Char(c));
        }
        editor.handle_key(Key::Ctrl('a'));
        editor.handle_key(Key::Char('!'));
        editor.handle_key(Key::Ctrl('e'));
        editor.handle_key(Key::Char('?'));

        assert_eq!(editor.input_line(), Some("!wq?"));
        assert_eq!(editor.input_cursor_column(), Some(5));
    }

    #[test]
    fn test_command_input_ctrl_w_uses_keyword_word_boundaries() {
        let mut editor = create_editor_with_content("hello");
        editor.handle_key(Key::Char(':'));
        for c in "foo_bar -baz".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.input_line(), Some("foo_bar -"));

        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.input_line(), Some("foo_bar "));
    }

    #[test]
    fn test_command_escape_cancels_after_short_pause_from_input_movement() {
        let mut editor = create_editor_with_content("hello");
        editor.handle_key(Key::Char(':'));
        for c in "write".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Left);
        std::thread::sleep(std::time::Duration::from_millis(120));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_search_input_alt_word_motions_and_ctrl_d() {
        let mut editor = create_editor_with_content("alpha beta gamma");
        editor.handle_key(Key::Char('/'));
        for c in "alpha beta".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Alt('b'));
        editor.handle_key(Key::Alt('b'));
        editor.handle_key(Key::Char('X'));
        assert_eq!(editor.input_line(), Some("Xalpha beta"));

        editor.handle_key(Key::Ctrl('d'));
        assert_eq!(editor.input_line(), Some("Xlpha beta"));
    }

    #[test]
    fn test_goto_line() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4\nline5");

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.cursor.line(), 2); // 0-indexed
    }

    #[test]
    fn test_search() {
        let mut editor = create_editor_with_content("hello world\nfoo bar");

        editor.handle_key(Key::Char('/'));
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_search_next_and_previous() {
        let mut editor = create_editor_with_content("target\nx\ntarget\n");

        editor.handle_key(Key::Char('/'));
        for c in "target\n".chars() {
            editor.handle_key(Key::Char(c));
        }
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('n'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('N'));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_percent_jumps_to_matching_bracket_under_cursor() {
        let mut editor = create_editor_with_content("(alpha)");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('%'));
        assert_eq!(editor.cursor.column(), 6);

        editor.handle_key(Key::Char('%'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_percent_uses_next_delimiter_on_current_line() {
        let mut editor = create_editor_with_content("foo bar(baz)");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), 11);
    }

    #[test]
    fn test_percent_matches_angle_brackets_with_nesting() {
        let mut editor = create_editor_with_content("<<>>");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('%'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char('%'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_percent_ignores_brackets_inside_strings_when_in_code() {
        let content = "let value = call(\"(\", a);";
        let open_column = content.find("call(").unwrap() + 4;
        let close_column = content.rfind(')').unwrap();
        let mut editor = create_syntax_editor(content, "sample.rs");
        editor.cursor = Cursor::new(0, open_column);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), close_column);
    }

    #[test]
    fn test_percent_falls_back_to_plaintext_matching_inside_string() {
        let content = "let text = \"[a(b)c]\";";
        let open_column = content.find('(').unwrap();
        let close_column = content.rfind(')').unwrap();
        let mut editor = create_syntax_editor(content, "sample.rs");
        editor.cursor = Cursor::new(0, open_column);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), close_column);
    }

    #[test]
    fn test_percent_falls_back_to_plaintext_matching_inside_line_comment() {
        let content = "// [a(b)c]\nvalue";
        let open_column = content.find('(').unwrap();
        let close_column = content.find(')').unwrap();
        let mut editor = create_syntax_editor(content, "sample.rs");
        editor.cursor = Cursor::new(0, open_column);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), close_column);
    }

    #[test]
    fn test_percent_matches_nested_block_comment_delimiters() {
        let mut editor = create_syntax_editor("/+ outer /+ inner +/ outer +/", "sample.d");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), 27);
    }

    #[test]
    fn test_percent_uses_next_block_comment_delimiter_on_current_line() {
        let mut editor = create_syntax_editor("value /* comment */ tail", "sample.rs");
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), 17);
    }

    #[test]
    fn test_percent_uses_next_closing_block_comment_delimiter_inside_comment() {
        let mut editor = create_syntax_editor("value /* comment */ tail", "sample.rs");
        editor.cursor = Cursor::new(0, 15);

        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.column(), 6);
    }

    #[test]
    fn test_counted_percent_uses_percentage_motion() {
        let mut editor = create_editor_with_content("1\n2\n3\n4\n5\n6\n7\n8\n9\n10");

        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('0'));
        editor.handle_key(Key::Char('0'));
        editor.handle_key(Key::Char('%'));

        assert_eq!(editor.cursor.line(), 9);
    }

    #[test]
    fn test_percent_caches_matches_and_clears_them_after_edits() {
        let mut editor = create_editor_with_content("(a)");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('%'));
        assert_eq!(editor.matching.match_cache.len(), 2);

        editor.mode = Mode::Insert;
        editor.handle_key(Key::Char('x'));
        assert!(editor.matching.match_cache.is_empty());
    }

    #[test]
    fn test_search_repeat_without_previous_search() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('n'));
        assert_eq!(
            editor.status_message,
            Some("No previous search".to_string())
        );
    }

    #[test]
    fn test_handle_resize_keeps_cursor_visible() {
        let mut editor = create_editor_with_content("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
        editor.cursor = Cursor::new(9, 0);

        editor.handle_resize(80, 4);

        assert!(
            editor
                .viewport
                .visible_range()
                .contains(&editor.cursor.line())
        );
    }

    #[test]
    fn test_boundary_protection_left() {
        let mut editor = create_editor_with_content("hello");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('h'));
        assert_eq!(editor.cursor.column(), 0); // Should not go negative
    }

    #[test]
    fn test_boundary_protection_up() {
        let mut editor = create_editor_with_content("hello\nworld");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('k'));
        assert_eq!(editor.cursor.line(), 0); // Should not go negative
    }

    #[test]
    fn test_boundary_protection_right_in_normal_mode() {
        let mut editor = create_editor_with_content("ab");
        editor.cursor = Cursor::new(0, 1); // Last character

        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.cursor.column(), 1); // Should not go past end in normal mode
    }

    #[test]
    fn test_exit_insert_mode_clamps_from_past_line_end() {
        let mut editor = create_editor_with_content("ab");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 2); // Insert-mode valid position (past end)

        editor.handle_key(Key::Esc);
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 1); // Last character in normal mode
    }

    #[test]
    fn test_input_line_returns_str_slice() {
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::command_with_text("test");

        let input = editor.input_line();
        assert_eq!(input, Some("test"));
    }

    #[test]
    fn test_move_line_start() {
        let mut editor = create_editor_with_content("hello world");
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('0'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_move_line_end() {
        let mut editor = create_editor_with_content("hello world");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('$'));
        assert_eq!(editor.cursor.column(), 10); // 'd' is at index 10
    }

    #[test]
    fn test_move_first_non_blank() {
        let mut editor = create_editor_with_content("   hello world");
        editor.cursor = Cursor::new(0, 10);

        editor.handle_key(Key::Char('^'));
        assert_eq!(editor.cursor.column(), 3); // 'h' is at index 3
    }

    #[test]
    fn test_move_to_last_line() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('G'));
        assert_eq!(editor.cursor.line(), 3); // Last line (0-indexed)
    }

    #[test]
    fn test_move_word_end() {
        let mut editor = create_editor_with_content("hello world test");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('e'));
        assert_eq!(editor.cursor.column(), 4); // 'o' of hello

        editor.handle_key(Key::Char('e'));
        assert_eq!(editor.cursor.column(), 10); // 'd' of world
    }

    #[test]
    fn test_move_next_paragraph() {
        let mut editor = create_editor_with_content("p1 line\nstill p1\n\np2 line\n");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('}'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_move_previous_paragraph() {
        let mut editor = create_editor_with_content("p1 line\n\np2 line\nstill p2\n");
        editor.cursor = Cursor::new(3, 0);

        editor.handle_key(Key::Char('{'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_save_file_as_with_w_command() {
        let target = TempFile::with_suffix("_save_as").unwrap();
        target.remove_now().unwrap();
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::command_with_text(format!("w {}", target.path().display()));

        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(editor.file_path, target.path());
        assert!(!editor.buffer.is_modified());
        assert!(editor.status_message.as_ref().unwrap().contains("written"));
    }

    #[test]
    fn test_save_file_as_with_write_command() {
        let target = TempFile::with_suffix("_write").unwrap();
        target.remove_now().unwrap();
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::command_with_text(format!("write {}", target.path().display()));

        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(editor.file_path, target.path());
        assert!(!editor.buffer.is_modified());
    }

    #[test]
    fn test_save_file_as_updates_file_path() {
        let target = TempFile::with_suffix("_new_file").unwrap();
        target.remove_now().unwrap();
        let mut editor = create_editor_with_content("new file content");
        assert!(editor.file_path.as_os_str().is_empty());

        editor.mode = Mode::command_with_text(format!("w {}", target.path().display()));
        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(editor.file_path, target.path());
    }

    #[test]
    fn test_save_without_filename_shows_error() {
        let mut editor = create_editor_with_content("some content");
        assert!(editor.file_path.as_os_str().is_empty());

        // Try to save without filename
        editor.mode = Mode::command_with_text("w");
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_w_current_file_writes_without_confirmation() {
        let target = TempFile::with_suffix("_confirm_write").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = target.path().to_path_buf();
        editor.mode = Mode::command_with_text("w");
        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(editor.overwrite_prompt(), None);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new");
        assert!(
            editor
                .status_message
                .as_deref()
                .unwrap()
                .contains("written")
        );
    }

    #[test]
    fn test_space_w_writes_current_file() {
        let target = TempFile::with_suffix("_space_w").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = target.path().to_path_buf();
        editor.handle_key(Key::Char(' '));
        handle_key_and_flush_requests(&mut editor, Key::Char('w'));

        assert!(!editor.should_quit);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new");
    }

    #[test]
    fn test_update_unmodified_buffer_is_noop_without_file_write() {
        let target = TempFile::with_suffix("_update_clean").unwrap();
        fs::write(target.path(), "old").unwrap();

        let metadata = fs::metadata(target.path()).unwrap();
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(target.path(), permissions).unwrap();

        let mut editor = create_editor_with_content("old");
        editor.file_path = target.path().to_path_buf();
        editor.mode = Mode::command_with_text("update");
        editor.handle_key(Key::Char('\n'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, None);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "old");

        let mut permissions = fs::metadata(target.path()).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(target.path(), permissions).unwrap();
    }

    #[test]
    fn test_update_modified_buffer_writes_current_file() {
        let target = TempFile::with_suffix("_update").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("old");
        editor.file_path = target.path().to_path_buf();
        // Dirty the buffer so `:update` must actually persist the new content.
        editor.buffer.insert(3, "!");
        editor.mode = Mode::command_with_text("update");
        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert!(!editor.should_quit);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "old!");
    }

    #[test]
    fn test_space_q_unmodified_unnamed_buffer_quits() {
        let mut editor = create_editor_with_content("new");
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('q'));

        assert!(editor.should_quit);
    }

    #[test]
    fn test_space_q_modified_unnamed_buffer_does_not_quit() {
        let mut editor = create_editor_with_content("new");
        editor.buffer.insert(3, "!");
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('q'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_w_save_as_existing_file_cancel_keeps_target_unchanged() {
        let source = TempFile::with_suffix("_save_as_source").unwrap();
        let target = TempFile::with_suffix("_confirm_cancel").unwrap();
        fs::write(source.path(), "source_old").unwrap();
        fs::write(target.path(), "target_old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = source.path().to_path_buf();
        editor.mode = Mode::command_with_text(format!("w {}", target.path().display()));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.overwrite_prompt(),
            Some(format!("Overwrite \"{}\"? [y/N]", target.path().display()))
        );
        editor.handle_key(Key::Esc);

        assert_eq!(fs::read_to_string(target.path()).unwrap(), "target_old");
        assert_eq!(editor.status_message, Some("Write cancelled".to_string()));
        assert_eq!(editor.file_path, source.path());
    }

    #[test]
    fn test_w_bang_bypasses_confirmation_for_existing_file() {
        let target = TempFile::with_suffix("_force_write").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = target.path().to_path_buf();
        editor.mode = Mode::command_with_text("w!");
        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(editor.overwrite_prompt(), None);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new");
    }

    #[test]
    fn test_wq_current_file_writes_and_quits_without_confirmation() {
        let target = TempFile::with_suffix("_wq").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = target.path().to_path_buf();
        editor.mode = Mode::command_with_text("wq");
        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new");
    }

    #[test]
    fn test_space_q_modified_buffer_writes_current_file_and_quits() {
        let target = TempFile::with_suffix("_space_q").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("old");
        editor.file_path = target.path().to_path_buf();
        // Dirty the buffer so update-and-quit must persist the in-memory edit.
        editor.buffer.insert(3, "!");
        editor.handle_key(Key::Char(' '));
        handle_key_and_flush_requests(&mut editor, Key::Char('q'));

        assert!(editor.should_quit);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "old!");
    }

    #[test]
    fn test_space_q_unmodified_named_buffer_quits_without_writing() {
        let target = TempFile::with_suffix("_space_q_clean").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("old");
        editor.file_path = target.path().to_path_buf();
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('q'));

        assert!(editor.should_quit);
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "old");
    }

    #[test]
    fn test_wq_force_no_file_name_does_not_quit() {
        let mut editor = create_editor_with_content("new");
        editor.mode = Mode::command_with_text("wq!");
        editor.handle_key(Key::Char('\n'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_q_modified_buffer_does_not_quit_immediately() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));

        assert!(!editor.should_quit);
    }

    #[test]
    fn test_q_bang_quits_with_unsaved_changes() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q!");
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(editor.quit_exit_code(), 0);
    }

    #[test]
    fn test_cquit_quits_with_error_exit_code() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("cquit");
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(editor.quit_exit_code(), 1);
    }

    #[test]
    fn test_q_modified_buffer_shows_quit_prompt_with_base_name() {
        let mut editor = create_editor_with_content("abc");
        editor.file_path = PathBuf::from("/tmp/ordex_test_name.txt");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.quit_prompt(),
            Some("Save changes to \"ordex_test_name.txt\"? [y]es/[n]o/[c]ancel".to_string())
        );
        assert!(!editor.should_quit);
    }

    #[test]
    fn test_q_modified_buffer_n_quits_without_saving() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('n'));

        assert!(editor.should_quit);
    }

    #[test]
    fn test_q_modified_buffer_c_cancels_quit() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('c'));

        assert!(!editor.should_quit);
        assert_eq!(editor.quit_prompt(), None);
        assert_eq!(editor.status_message, Some("Quit cancelled".to_string()));
    }

    #[test]
    fn test_q_modified_buffer_other_key_cancels_quit() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Esc);

        assert!(!editor.should_quit);
        assert_eq!(editor.quit_prompt(), None);
        assert_eq!(editor.status_message, Some("Quit cancelled".to_string()));
    }

    #[test]
    fn test_q_unmodified_buffer_quits_directly() {
        let mut editor = create_editor_with_content("abc");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(editor.quit_prompt(), None);
    }

    #[test]
    fn test_q_unnamed_buffer_y_shows_no_file_name_and_does_not_quit() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("q");
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('y'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_find_forward_and_backward_on_current_line() {
        let mut editor = create_editor_with_content("abca");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char('F'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_till_forward_and_backward() {
        let mut editor = create_editor_with_content("abcde");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('t'));
        editor.handle_key(Key::Char('d'));
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char('T'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_till_adjacent_target_stays_in_place() {
        let mut editor = create_editor_with_content("abc");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('t'));
        editor.handle_key(Key::Char('b'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_find_does_not_cross_line_boundaries() {
        let mut editor = create_editor_with_content("abc\nxa");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    fn test_repeat_find_semicolon_and_comma() {
        let mut editor = create_editor_with_content("abca");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);

        // ';' repeats original find direction (forward), not the temporary ',' opposite direction.
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    fn test_repeat_find_without_previous_motion_is_silent() {
        let mut editor = create_editor_with_content("abc");
        editor.status_message = None;
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.status_message, None);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    fn test_failed_repeat_attempt_does_not_change_base_repeat_direction() {
        let mut editor = create_editor_with_content("cxxc");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('c'));
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Char('0'));
        assert_eq!(editor.cursor.column(), 0);

        // Opposite direction repeat fails at line start.
        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);

        // ';' keeps the original forward direction and should jump to the next match.
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    fn test_failed_find_then_semicolon_on_line_with_match_moves_cursor() {
        let mut editor = create_editor_with_content("bbbb\naxxa");
        editor.cursor = Cursor::new(0, 0);

        // Fail to find 'a' on first line, but keep last-find state.
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor, Cursor::new(0, 0));

        // Move to a line where the same motion has a match and repeat it.
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor, Cursor::new(1, 0));

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor, Cursor::new(1, 3));
    }

    #[test]
    fn test_semicolon_repeatedly_moves_in_base_direction() {
        let mut editor = create_editor_with_content("abacada");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 4);

        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 6);

        // No further match, so repeated ';' stays put.
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 6);
    }

    #[test]
    fn test_comma_repeatedly_moves_in_opposite_direction() {
        let mut editor = create_editor_with_content("abacada");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char(';'));
        editor.handle_key(Key::Char(';'));
        assert_eq!(editor.cursor.column(), 6);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 4);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);

        // No further match in opposite direction, so repeated ',' stays put.
        editor.handle_key(Key::Char(','));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_find_pending_indicator_and_escape_cancel() {
        let mut editor = create_editor_with_content("abc");

        editor.handle_key(Key::Char('f'));
        assert_eq!(editor.pending_prefix_label(), Some("f".to_string()));

        editor.handle_key(Key::Esc);
        assert_eq!(editor.pending_prefix_label(), None);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_pending_find_consumes_non_printable_input() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('f'));
        assert_eq!(editor.pending_prefix_label(), Some("f".to_string()));

        // Ctrl+F is normally page-down, but should be consumed while waiting for find target.
        editor.handle_key(Key::Ctrl('f'));
        assert_eq!(editor.pending_prefix_label(), Some("f".to_string()));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_g_starts_pending_sequence() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('g'));

        assert_eq!(editor.pending_prefix_label(), Some("g".to_string()));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_sequence_discovery_popup_shows_built_in_g_continuations() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('g'));

        assert_eq!(
            editor.sequence_discovery_popup(),
            Some(SequenceDiscoveryPopup {
                prefix: "g".to_string(),
                entries: vec![
                    SequenceDiscoveryEntry {
                        keys: "g".to_string(),
                        action: "Move to first line".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "$".to_string(),
                        action: "Move line end".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "0".to_string(),
                        action: "Move line start".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "v".to_string(),
                        action: "Recreate last selection".to_string(),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_sequence_discovery_popup_shows_built_in_z_continuations() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('z'));

        assert_eq!(
            editor.sequence_discovery_popup(),
            Some(SequenceDiscoveryPopup {
                prefix: "z".to_string(),
                entries: vec![
                    SequenceDiscoveryEntry {
                        keys: "t".to_string(),
                        action: "Align viewport top".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "z".to_string(),
                        action: "Align viewport center".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "b".to_string(),
                        action: "Align viewport bottom".to_string(),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_sequence_discovery_popup_keeps_count_in_prefix() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('d'));

        assert_eq!(
            editor.sequence_discovery_popup(),
            Some(SequenceDiscoveryPopup {
                prefix: "2d".to_string(),
                entries: vec![
                    SequenceDiscoveryEntry {
                        keys: "iw".to_string(),
                        action: "Delete inner word".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "a(".to_string(),
                        action: "Delete around paren".to_string(),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_sequence_discovery_popup_uses_configured_sequences() {
        let mut editor = create_editor_with_content("ab\ncd\nef");
        editor.apply_config(&ConfigSettings {
            sequence_bindings: vec![
                crate::config::ConfiguredSequenceBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    keys: vec![KeyInput::Char('z'), KeyInput::Char('u')],
                    actions: ActionBinding::Multiple(vec![Action::MoveDown, Action::MoveRight]),
                    source: "test".to_string(),
                },
                crate::config::ConfiguredSequenceBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    keys: vec![KeyInput::Char('z'), KeyInput::Char('q')],
                    actions: ActionBinding::single(Action::SaveCurrentFile),
                    source: "test".to_string(),
                },
            ],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(
            editor.sequence_discovery_popup(),
            Some(SequenceDiscoveryPopup {
                prefix: "z".to_string(),
                entries: vec![
                    SequenceDiscoveryEntry {
                        keys: "t".to_string(),
                        action: "Align viewport top".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "z".to_string(),
                        action: "Align viewport center".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "b".to_string(),
                        action: "Align viewport bottom".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "u".to_string(),
                        action: "Move down -> Move right".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "q".to_string(),
                        action: "Save current file".to_string(),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_gg_moves_to_first_line_and_keeps_column() {
        let mut editor = create_editor_with_content("abcdef\nxy");
        editor.cursor = Cursor::new(1, 1);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_g_dollar_moves_to_current_line_end() {
        let mut editor = create_editor_with_content("abcde");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('$'));

        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_g_zero_moves_to_current_line_start() {
        let mut editor = create_editor_with_content("abcde");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('0'));

        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_gi_consumes_both_and_does_not_enter_insert_mode() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('i'));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_g_colon_consumes_both_and_does_not_enter_command_mode() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char(':'));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_g_slash_consumes_both_and_does_not_enter_search_mode() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('/'));

        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_zt_aligns_viewport_top_without_moving_cursor() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\n",
        );
        editor.viewport.set_scroll_margin(1);
        editor.handle_resize(80, 10);
        editor.cursor = Cursor::new(5, 2);

        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('t'));

        assert_eq!(editor.cursor.line(), 5);
        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(editor.viewport.first_visible_line(), 4);
    }

    #[test]
    fn test_zz_aligns_viewport_center_without_moving_cursor() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\n",
        );
        editor.viewport.set_scroll_margin(1);
        editor.handle_resize(80, 10);
        editor.cursor = Cursor::new(5, 2);

        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 5);
        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(editor.viewport.first_visible_line(), 1);
    }

    #[test]
    fn test_zb_aligns_viewport_bottom_without_moving_cursor() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\n",
        );
        editor.viewport.set_scroll_margin(1);
        editor.handle_resize(80, 10);
        editor.cursor = Cursor::new(5, 2);

        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('b'));

        assert_eq!(editor.cursor.line(), 5);
        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(editor.viewport.first_visible_line(), 0);
    }

    #[test]
    fn test_ctrl_e_respects_scroll_margin_by_nudging_cursor_down() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\nline 09\nline 10\nline 11\nline 12\nline 13\nline 14\nline 15\nline 16\n",
        );
        editor.viewport.set_soft_wrap(false);
        editor.viewport.set_scroll_margin(1);
        editor.viewport.set_height(8);
        editor.viewport.set_first_visible_line(9);
        editor.cursor = Cursor::new(10, 2);

        editor.handle_key(Key::Ctrl('e'));

        assert_eq!(editor.viewport.first_visible_line(), 10);
        assert_eq!(editor.cursor.line(), 11);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_ctrl_y_respects_scroll_margin_by_nudging_cursor_up() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\nline 09\nline 10\nline 11\nline 12\nline 13\nline 14\nline 15\nline 16\nline 17\nline 18\n",
        );
        editor.viewport.set_soft_wrap(false);
        editor.viewport.set_scroll_margin(1);
        editor.viewport.set_height(8);
        editor.viewport.set_first_visible_line(10);
        editor.cursor = Cursor::new(16, 2);

        editor.handle_key(Key::Ctrl('y'));

        assert_eq!(editor.viewport.first_visible_line(), 9);
        assert_eq!(editor.cursor.line(), 15);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_configured_single_key_binding_beats_built_in_z_prefix() {
        let mut editor = create_editor_with_content("ab\n");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                actions: ActionBinding::single(Action::MoveRight),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.column(), 1);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_diw_deletes_inner_word_and_stays_normal() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), " beta");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_ciw_deletes_inner_word_and_enters_insert() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 7);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), "alpha ");
        assert_eq!(editor.cursor.column(), 6);
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn test_user_repro_sequence_with_escape_char_variant() {
        let mut editor = create_editor_with_content("One line");

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));
        editor.handle_key(Key::Char('C'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('\u{1b}'));

        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_da_paren_deletes_smallest_surrounding_pair() {
        let mut editor = create_editor_with_content("x(a(b)c)y");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('('));

        assert_eq!(editor.buffer.to_string(), "x(ac)y");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_da_paren_without_match_is_silent_noop() {
        let mut editor = create_editor_with_content("abc def");
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('('));

        assert_eq!(editor.buffer.to_string(), "abc def");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    fn test_escape_clears_pending_sequence() {
        let mut editor = create_editor_with_content("abcde");

        editor.handle_key(Key::Char('g'));
        assert_eq!(editor.pending_prefix_label(), Some("g".to_string()));

        editor.handle_key(Key::Esc);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_count_pending_indicator_is_not_capped() {
        let mut editor = create_editor_with_content("abcde");

        for c in "1000000".chars() {
            editor.handle_key(Key::Char(c));
        }

        assert_eq!(editor.pending_prefix_label(), Some("1000000".to_string()));
    }

    #[test]
    fn test_count_zero_rule_and_counted_h_motion() {
        let mut editor = create_editor_with_content("abcdef");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('0'));
        assert_eq!(editor.cursor.column(), 0);

        editor.cursor = Cursor::new(0, 4);
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('0'));
        editor.handle_key(Key::Char('h'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_counted_g_and_gg_go_to_line_number() {
        let mut editor = create_editor_with_content("l1\nl2\nl3\nl4\nl5");

        editor.handle_key(Key::Char('4'));
        editor.handle_key(Key::Char('G'));
        assert_eq!(editor.cursor.line(), 3);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));
        assert_eq!(editor.cursor.line(), 1);
    }

    #[test]
    fn test_counted_g_and_gg_do_not_use_repeat_cap() {
        let mut editor = create_editor_with_content("l1\nl2");

        for c in "1000000".chars() {
            editor.handle_key(Key::Char(c));
        }
        editor.handle_key(Key::Char('G'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(
            editor.status_message,
            Some("Line 1000000 out of range, moved to last line".to_string())
        );

        for c in "1000001".chars() {
            editor.handle_key(Key::Char(c));
        }
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(
            editor.status_message,
            Some("Line 1000001 out of range, moved to last line".to_string())
        );
    }

    #[test]
    fn test_counted_find_all_or_nothing() {
        let mut editor = create_editor_with_content("abacada");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('f'));
        assert_eq!(editor.pending_prefix_label(), Some("3f".to_string()));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 6);

        editor.cursor = Cursor::new(0, 0);
        editor.handle_key(Key::Char('4'));
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_count_before_insert_action_executes_once() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('3'));
        assert_eq!(editor.pending_prefix_label(), Some("3".to_string()));
        editor.handle_key(Key::Char('i'));

        assert!(editor.mode.is_insert());
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_escape_after_i_lands_on_previous_character() {
        let mut editor = create_editor_with_content("helo");
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "hello");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_escape_after_a_lands_on_previous_character() {
        let mut editor = create_editor_with_content("helo");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "hello");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_uppercase_i_inserts_at_first_non_blank() {
        let mut editor = create_editor_with_content("  abc");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('I'));
        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "  xabc");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_uppercase_a_appends_at_end_of_line() {
        let mut editor = create_editor_with_content("abc");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('A'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('e'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "abcde");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_multi_action_binding_executes_actions_in_order() {
        let mut editor = create_editor_with_content("ab\ncd\nef");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                actions: ActionBinding::Multiple(vec![Action::MoveDown, Action::MoveRight]),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_multi_action_binding_repeats_whole_sequence_for_counts() {
        let mut editor = create_editor_with_content("ab\ncd\nef\ngh");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                actions: ActionBinding::Multiple(vec![Action::MoveDown, Action::MoveRight]),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 3);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_multi_action_sequence_binding_executes_actions_in_order() {
        let mut editor = create_editor_with_content("ab\ncd\nef");
        editor.apply_config(&ConfigSettings {
            sequence_bindings: vec![crate::config::ConfiguredSequenceBinding {
                mode: crate::keybindings::ModeContext::Normal,
                keys: vec![KeyInput::Char('z'), KeyInput::Char('u')],
                actions: ActionBinding::Multiple(vec![Action::MoveDown, Action::MoveRight]),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('u'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_replace_config_resets_removed_bindings_to_defaults() {
        let mut editor = create_editor_with_content("ab\ncd");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                actions: ActionBinding::single(Action::MoveRight),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));
        assert_eq!(editor.cursor.column(), 1);

        editor.cursor = Cursor::new(0, 0);
        editor.replace_config(&ConfigSettings::default());
        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_apply_config_enables_relative_line_numbers() {
        let mut editor = create_editor_with_content("a\nb\nc");
        editor.cursor = Cursor::new(1, 0);

        editor.apply_config(&ConfigSettings {
            relative_line_numbers: Some(true),
            ..ConfigSettings::default()
        });

        assert!(editor.relative_line_numbers_enabled());
        assert_eq!(editor.display_line_number(0), 1);
        assert_eq!(editor.display_line_number(1), 2);
        assert_eq!(editor.display_line_number(2), 1);
    }

    #[test]
    fn test_apply_config_can_disable_soft_wrap() {
        let mut editor = create_editor_with_content("abcdefghijklmnopqrstuvwxyz");

        editor.apply_config(&ConfigSettings {
            soft_wrap: Some(false),
            ..ConfigSettings::default()
        });
        editor.cursor = Cursor::new(0, 20);
        editor.handle_resize(8, 8);

        assert!(!editor.soft_wrap_enabled());
        assert!(editor.viewport.first_visible_column() > 0);
    }

    #[test]
    fn test_apply_config_can_disable_sequence_discovery_popup() {
        let mut editor = create_editor_with_content("alpha\nbeta");

        editor.apply_config(&ConfigSettings {
            sequence_discovery_popup: Some(false),
            ..ConfigSettings::default()
        });
        editor.handle_key(Key::Char('g'));

        assert!(!editor.sequence_discovery_popup_enabled());
        assert_eq!(editor.sequence_discovery_popup(), None);
        assert_eq!(editor.pending_prefix_label(), Some("g".to_string()));
    }

    #[test]
    fn test_apply_config_can_switch_theme() {
        let mut editor = create_editor_with_content("alpha");

        editor.apply_config(&ConfigSettings {
            theme: Some("nord".to_string()),
            ..ConfigSettings::default()
        });

        assert_eq!(editor.theme_name(), "nord");
    }

    #[test]
    fn test_replace_config_resets_relative_line_numbers_to_default() {
        let mut editor = create_editor_with_content("a\nb");
        editor.apply_config(&ConfigSettings {
            relative_line_numbers: Some(true),
            ..ConfigSettings::default()
        });

        editor.replace_config(&ConfigSettings::default());

        assert!(!editor.relative_line_numbers_enabled());
        assert_eq!(editor.display_line_number(1), 2);
    }

    #[test]
    fn test_replace_config_resets_soft_wrap_to_default() {
        let mut editor = create_editor_with_content("abcdefghijklmnopqrstuvwxyz");
        editor.apply_config(&ConfigSettings {
            soft_wrap: Some(false),
            ..ConfigSettings::default()
        });

        editor.replace_config(&ConfigSettings::default());

        assert!(editor.soft_wrap_enabled());
    }

    #[test]
    fn test_replace_config_resets_sequence_discovery_popup_to_default() {
        let mut editor = create_editor_with_content("alpha\nbeta");
        editor.apply_config(&ConfigSettings {
            sequence_discovery_popup: Some(false),
            ..ConfigSettings::default()
        });

        editor.replace_config(&ConfigSettings::default());
        editor.handle_key(Key::Char('g'));

        assert!(editor.sequence_discovery_popup_enabled());
        assert!(editor.sequence_discovery_popup().is_some());
    }

    #[test]
    fn test_replace_config_resets_theme_to_default() {
        let mut editor = create_editor_with_content("alpha");
        editor.apply_config(&ConfigSettings {
            theme: Some("nord".to_string()),
            ..ConfigSettings::default()
        });

        editor.replace_config(&ConfigSettings::default());

        assert_eq!(editor.theme_name(), themes::DEFAULT_THEME_NAME);
    }

    #[test]
    fn test_replace_config_preserves_color_capability() {
        let mut editor = create_editor_with_content("alpha");
        editor.set_color_capability(themes::ColorCapability::TrueColor);

        editor.replace_config(&ConfigSettings::default());

        assert_eq!(
            editor.color_capability(),
            themes::ColorCapability::TrueColor
        );
    }

    #[test]
    fn test_move_down_uses_wrapped_rows_when_soft_wrap_enabled() {
        let mut editor = create_editor_with_content("abcdefghij\nzz");
        editor.handle_resize(4, 8);
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('j'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 5);
    }

    #[test]
    fn test_move_down_wraps_to_next_buffer_line() {
        let mut editor = create_editor_with_content("abcdef\nghij");
        editor.handle_resize(4, 8);
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('j'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_wrapped_vertical_motion_preserves_desired_visual_column() {
        let mut editor = create_editor_with_content("abcdefgh\nx\nabcdefgh");
        editor.handle_resize(4, 8);
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 7);

        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    fn test_move_down_keeps_buffer_line_semantics_when_soft_wrap_disabled() {
        let mut editor = create_editor_with_content("abcdefghij\nzz");
        editor.apply_config(&ConfigSettings {
            soft_wrap: Some(false),
            ..ConfigSettings::default()
        });
        editor.handle_resize(4, 8);
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('j'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_reload_config_command_queues_request() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('r'));
        editor.handle_key(Key::Char('e'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('-'));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('n'));
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.take_pending_request(),
            Some(EditorRequest::ReloadConfig)
        );
        assert_eq!(editor.take_pending_request(), None);
    }

    /// Queue a deferred write request when `:w` can proceed without confirmation.
    #[test]
    fn test_write_command_queues_deferred_request() {
        let target = TempFile::with_suffix("_queued_write").unwrap();
        target.remove_now().unwrap();
        let mut editor = create_editor_with_content("hello");
        editor.mode = Mode::command_with_text(format!("w {}", target.path().display()));

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.take_pending_request(),
            Some(EditorRequest::WriteBuffer(DeferredWrite {
                path: target.path().to_path_buf(),
                update_file_path: true,
                quit_on_success: false,
            }))
        );
    }

    #[test]
    fn test_operator_motion_count_multiplication_for_diw() {
        let mut editor = create_editor_with_content("one two three four five");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        let content = editor.buffer.to_string();
        assert!(!content.contains("one"));
        assert!(!content.contains("two"));
        assert!(!content.contains("three"));
        assert!(!content.contains("four"));
        assert!(content.contains("five"));
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_counted_vertical_motions_use_single_prefix() {
        let mut editor = create_editor_with_content("l1\nl2\nl3\nl4\nl5\nl6");
        editor.handle_key(Key::Char('4'));
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 4);

        editor.handle_key(Key::Char('9'));
        editor.handle_key(Key::Char('k'));
        assert_eq!(editor.cursor.line(), 0);
    }

    #[test]
    fn test_counted_right_motion_saturates_line_end() {
        let mut editor = create_editor_with_content("abcdef");
        editor.handle_key(Key::Char('9'));
        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.cursor.column(), 5);
    }

    #[test]
    fn test_counted_word_motions() {
        let mut editor = create_editor_with_content("one two three four");
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('w'));
        assert_eq!(editor.cursor.column(), 14);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('b'));
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_counted_x_deletes_multiple_chars() {
        let mut editor = create_editor_with_content("abcdef");
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('x'));
        assert_eq!(editor.buffer.to_string(), "def");
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_counted_search_next() {
        let mut editor = create_editor_with_content("target\nx\ntarget\ny\ntarget\nz\ntarget");
        editor.handle_key(Key::Char('/'));
        for c in "target\n".chars() {
            editor.handle_key(Key::Char(c));
        }
        assert_eq!(editor.cursor.line(), 0);

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('n'));
        assert_eq!(editor.cursor.line(), 6);
    }

    #[test]
    fn test_counted_page_down_and_up() {
        let lines = (1..=200).map(|i| format!("line{}", i)).collect::<Vec<_>>();
        let mut editor = create_editor_with_content(&lines.join("\n"));

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Ctrl('f'));
        assert!(editor.cursor.line() >= 40);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Ctrl('b'));
        assert_eq!(editor.cursor.line(), 0);
    }

    #[test]
    fn test_operator_count_without_motion_count_for_diw() {
        let mut editor = create_editor_with_content("one two three four");
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), "   four");
    }

    #[test]
    fn test_motion_count_without_outer_count_for_diw() {
        let mut editor = create_editor_with_content("one two three");
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), "  three");
    }

    #[test]
    fn test_pending_indicator_shows_operator_motion_count() {
        let mut editor = create_editor_with_content("one two");
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('3'));
        assert_eq!(editor.pending_prefix_label(), Some("2d3".to_string()));
    }

    #[test]
    fn test_escape_clears_pending_count() {
        let mut editor = create_editor_with_content("abc");
        editor.handle_key(Key::Char('4'));
        assert_eq!(editor.pending_prefix_label(), Some("4".to_string()));
        editor.handle_key(Key::Esc);
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_count_before_command_mode_executes_once() {
        let mut editor = create_editor_with_content("abc");
        editor.handle_key(Key::Char('5'));
        editor.handle_key(Key::Char(':'));
        assert!(matches!(editor.mode, Mode::Command(_)));
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    /// Regression test for Vim-style visual `o` endpoint swaps.
    fn test_visual_o_swaps_active_selection_end() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('l'));

        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(editor.selection_range(), Some((0, 3)));

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.selection_range(), Some((0, 3)));

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(editor.selection_range(), Some((0, 3)));
    }

    #[test]
    /// Regression test for visual-line `o` preserving the selected lines.
    fn test_visual_line_o_swaps_endpoints_without_changing_lines() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.selection_range(), Some((0, 8)));

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.selection_range(), Some((0, 8)));
    }

    #[test]
    /// Regression test for `gv` recreating the most recent characterwise selection.
    fn test_gv_recreates_last_characterwise_selection() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Esc);

        assert!(editor.mode.is_normal());
        assert_eq!(editor.selection_range(), None);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('v'));

        assert_eq!(editor.mode, Mode::Visual(VisualKind::Character));
        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(editor.selection_range(), Some((0, 3)));
    }

    #[test]
    /// Regression test for `gv` recreating the most recent linewise selection.
    fn test_gv_recreates_last_linewise_selection() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Esc);

        assert!(editor.mode.is_normal());
        assert_eq!(editor.selection_range(), None);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('v'));

        assert_eq!(editor.mode, Mode::Visual(VisualKind::Line));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.selection_range(), Some((0, 8)));
    }

    #[test]
    /// Regression test for `gv` staying a no-op before any visual selection exists.
    fn test_gv_without_prior_selection_is_no_op() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('v'));

        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.selection_range(), None);
    }

    #[test]
    fn test_visual_character_mode_tracks_inclusive_selection() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        assert!(matches!(editor.mode, Mode::Visual(VisualKind::Character)));
        assert_eq!(editor.selection_range(), Some((0, 1)));

        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.selection_range(), Some((0, 2)));
    }

    #[test]
    fn test_visual_counted_motion_extends_selection() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('l'));

        assert_eq!(editor.selection_range(), Some((0, 3)));
    }

    #[test]
    fn test_visual_delete_selection_returns_to_normal() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('d'));

        assert_eq!(editor.buffer.to_string(), "cd");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.selection_range(), None);
    }

    #[test]
    fn test_visual_change_selection_enters_insert_mode() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "cd");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.selection_range(), None);
    }

    #[test]
    /// Regression test for Visual `y` leaving Visual mode and preserving the selection text.
    fn test_visual_yank_selection_pastes_after_cursor() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('y'));

        assert!(editor.mode.is_normal());
        assert_eq!(editor.selection_range(), None);

        editor.handle_key(Key::Char('p'));

        assert_eq!(editor.buffer.to_string(), "ababcd");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    /// Regression test for `yy` storing a linewise payload that `p` pastes below.
    fn test_yy_then_p_pastes_line_below_cursor() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('p'));

        assert_eq!(editor.buffer.to_string(), "one\none\ntwo\nthree");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression test for `P` inserting a last-line yank above the current line.
    fn test_linewise_paste_before_handles_last_line_yank() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('k'));
        editor.handle_key(Key::Char('P'));

        assert_eq!(editor.buffer.to_string(), "one\nthree\ntwo\nthree");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression test for delete actions populating the unnamed register for `P`.
    fn test_x_then_paste_before_restores_deleted_character() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Char('P'));

        assert_eq!(editor.buffer.to_string(), "abcd");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression test for linewise visual deletes feeding the same paste buffer.
    fn test_visual_line_delete_then_paste_before_restores_lines() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('P'));

        assert_eq!(editor.buffer.to_string(), "one\ntwo\nthree");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression test for counted `yy` storing multiple lines in one payload.
    fn test_counted_yy_yanks_multiple_lines() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('p'));

        assert_eq!(editor.buffer.to_string(), "one\none\ntwo\ntwo\nthree");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression test for surfacing an explicit status when paste is unavailable.
    fn test_paste_without_yank_sets_status_message() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('p'));

        assert_eq!(editor.status_message.as_deref(), Some("Nothing to paste"));
        assert_eq!(editor.buffer.to_string(), "abcd");
    }

    #[test]
    /// Regression test for reprocessing a key that breaks the pending `yy` prefix.
    fn test_unmatched_y_prefix_reprocesses_following_key() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char(':'));

        assert!(editor.mode.is_command());
    }

    #[test]
    fn test_visual_line_delete_removes_full_lines() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('d'));

        assert_eq!(editor.buffer.to_string(), "three");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_a_inserts_after_cursor() {
        let mut editor = create_editor_with_content("hello");

        // Cursor starts at column 0; 'a' should move to column 1 and enter insert mode
        editor.handle_key(Key::Char('a'));
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_a_on_empty_line() {
        let mut editor = create_editor_with_content("");

        editor.handle_key(Key::Char('a'));
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_x_deletes_char_at_cursor() {
        let mut editor = create_editor_with_content("hello");

        // Delete 'h' at cursor
        editor.handle_key(Key::Char('x'));
        assert_eq!(editor.buffer.to_string(), "ello");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_x_on_empty_line() {
        let mut editor = create_editor_with_content("");

        // Should be a no-op on empty line
        editor.handle_key(Key::Char('x'));
        assert_eq!(editor.buffer.to_string(), "");
        assert!(editor.mode.is_normal());
    }
}
