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
use crate::syntax::{BufferEdit, HighlightSpan, SyntaxEngine};
use crate::text_buffer::TextBuffer;
use crate::tui;
use crate::viewport::Viewport;
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use termion::event::Key;

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

/// One normalized, exclusive selection range in buffer character coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionRange {
    /// First selected character index.
    start: usize,
    /// One-past-the-end selected character index.
    end: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorRequest {
    ReloadConfig,
}

/// Runtime editor settings that have built-in defaults and may be overridden by config.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSettings {
    scroll_margin: usize,
    horizontal_scroll_margin: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    sequence_discovery_popup: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            scroll_margin: Viewport::DEFAULT_SCROLL_MARGIN,
            horizontal_scroll_margin: Viewport::DEFAULT_HORIZONTAL_SCROLL_MARGIN,
            relative_line_numbers: false,
            soft_wrap: true,
            sequence_discovery_popup: true,
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
    pub(crate) buffer: TextBuffer,
    /// Current cursor position
    pub(crate) cursor: Cursor,
    /// Current editor mode
    pub(crate) mode: Mode,
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
    pub(crate) viewport: Viewport,
    /// Path to the file being edited
    pub(crate) file_path: PathBuf,
    /// Derived syntax-highlighting state for the current document.
    syntax: SyntaxEngine,
    /// Status message to display (cleared after one render)
    pub(crate) status_message: Option<String>,
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
    pub(crate) should_quit: bool,
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
    /// Pending overwrite confirmation for save commands targeting an existing file.
    pending_overwrite: Option<PendingOverwrite>,
    /// Pending quit confirmation for `:q` with unsaved changes.
    pending_quit_confirmation: Option<PendingQuitConfirmation>,
    /// Ignore trailing Escape bytes for a short window after input cursor movement.
    ignore_input_escape_cancel_until: Option<Instant>,
    /// One-shot request for work that must be deferred until after `handle_key`.
    ///
    /// `EditorState` owns editor-local state, but some commands need data or I/O
    /// owned by the outer application loop instead. `:reload-config` is the
    /// current example: parsing the command belongs here, but resolving the
    /// active config path and reading files from disk belongs in `main.rs`,
    /// where the CLI-derived config path is available. Keeping only a request
    /// token here prevents `EditorState` from taking on startup/process
    /// concerns, keeps input handling deterministic, and leaves the main loop as
    /// the single place that performs process-level side effects after a key has
    /// been fully processed.
    pending_request: Option<EditorRequest>,
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
            last_search_pattern: None,
            pending_sequence: Vec::new(),
            pending_count: None,
            pending_sequence_count: None,
            pending_sequence_motion_count: None,
            pending_find: None,
            last_find: None,
            pending_overwrite: None,
            pending_quit_confirmation: None,
            ignore_input_escape_cancel_until: None,
            pending_request: None,
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
        self.settings = EditorSettings::default();
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

    /// Return whether relative line numbers are enabled for rendering.
    pub(crate) fn relative_line_numbers_enabled(&self) -> bool {
        self.settings.relative_line_numbers
    }

    /// Return whether soft wrapping is currently enabled.
    pub(crate) fn soft_wrap_enabled(&self) -> bool {
        self.settings.soft_wrap
    }

    /// Return whether the sequence-discovery popup is currently enabled.
    pub(crate) fn sequence_discovery_popup_enabled(&self) -> bool {
        self.settings.sequence_discovery_popup
    }

    /// Return the gutter number to show for one buffer line.
    ///
    /// When relative numbering is enabled, the cursor line stays absolute and all
    /// other buffer lines show their distance from the cursor.
    pub(crate) fn display_line_number(&self, line_idx: usize) -> usize {
        if !self.settings.relative_line_numbers || line_idx == self.cursor.line() {
            return line_idx + 1;
        }

        line_idx.abs_diff(self.cursor.line())
    }

    /// Take the next deferred request queued by command execution, if any.
    ///
    /// Requests are one-shot because they describe work for exactly one pass of
    /// the outer event loop after the triggering key sequence completes.
    pub(crate) fn take_pending_request(&mut self) -> Option<EditorRequest> {
        self.pending_request.take()
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
        Ok(())
    }

    /// Re-detect the active language and rebuild syntax state for the current buffer.
    pub(crate) fn refresh_syntax(&mut self) {
        let path = (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.as_path());
        self.syntax.open_document(path, &self.buffer);
    }

    /// Return the current syntax-generation counter.
    pub(crate) fn syntax_generation(&self) -> u64 {
        self.syntax.generation()
    }

    /// Borrow the syntax spans for one logical line.
    pub(crate) fn syntax_spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        self.syntax.spans_for_line(line_index)
    }

    /// Insert `text` at `char_idx` and notify the syntax engine about the edit.
    fn insert_buffer_text(&mut self, char_idx: usize, text: &str) {
        let start_line = self
            .buffer
            .char_to_line(char_idx.min(self.buffer.chars_count()));
        self.buffer.insert(char_idx, text);
        self.syntax.apply_edit(
            &self.buffer,
            BufferEdit {
                start_line,
                old_end_line: start_line,
                new_end_line: start_line + text.chars().filter(|&c| c == '\n' || c == '\r').count(),
            },
        );
    }

    /// Remove one character-index range and notify the syntax engine about the edit.
    fn remove_buffer_range(&mut self, start_char: usize, end_char: usize) {
        if start_char >= end_char {
            return;
        }
        let start_line = self.buffer.char_to_line(start_char);
        let old_end_line = self.removal_old_end_line(start_char, end_char);
        self.buffer.remove(start_char, end_char);
        self.syntax.apply_edit(
            &self.buffer,
            BufferEdit {
                start_line,
                old_end_line,
                new_end_line: start_line,
            },
        );
    }

    /// Return the last pre-edit line affected by a removal range.
    fn removal_old_end_line(&self, start_char: usize, end_char: usize) -> usize {
        let last_deleted_line = self.buffer.char_to_line(end_char.saturating_sub(1));
        let deleted_text = (start_char..end_char)
            .filter_map(|char_idx| self.buffer.char_at(char_idx))
            .collect::<String>();

        // Removing a line break merges the following logical line into the start
        // line, so the syntax cache splice must also include that following line.
        if deleted_text.chars().any(|ch| ch == '\n' || ch == '\r') {
            return (last_deleted_line + 1).min(self.buffer.lines_count().saturating_sub(1));
        }

        last_deleted_line
    }

    /// Update viewport dimensions after a terminal resize.
    pub(crate) fn handle_resize(&mut self, terminal_width: usize, terminal_height: usize) {
        self.viewport.set_width(terminal_width);
        self.viewport
            .set_height(terminal_height.saturating_sub(Self::RESERVED_BOTTOM_ROWS));
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Handle one normalized key input and route it through pending states and bindings.
    pub(crate) fn handle_key(&mut self, key: Key) {
        let key = Self::normalize_key(key);
        if matches!(self.mode, Mode::Command(_) | Mode::Search(_)) {
            if key == Key::Esc {
                if self
                    .ignore_input_escape_cancel_until
                    .is_some_and(|until| Instant::now() <= until)
                {
                    return;
                }
                self.ignore_input_escape_cancel_until = None;
            } else {
                self.ignore_input_escape_cancel_until = None;
            }
        } else {
            self.ignore_input_escape_cancel_until = None;
        }

        // Highest priority: overwrite confirmation must consume input first so
        // destructive write prompts cannot be bypassed by other pending states.
        if self.handle_pending_overwrite_key(key) {
            return;
        }

        // Next: quit confirmation prompt takes precedence over navigation/editing.
        if self.handle_pending_quit_key(key) {
            return;
        }

        // While waiting for find/till target, consume every key until resolved/cancelled.
        if self.handle_pending_find_key(key) {
            return;
        }

        // Then process multi-key normal-mode sequences (g*, diw/ciw/da().
        if self.handle_pending_sequence_key(key) {
            return;
        }

        // Finally, parse a fresh numeric count prefix if applicable.
        if self.handle_pending_count_key(key) {
            return;
        }

        if self.mode_uses_modal_bindings() {
            let key_input = KeyInput::from(key);
            if self
                .keybindings
                .starts_sequence_prefix(&self.mode, &key_input)
            {
                self.pending_sequence.clear();
                self.pending_sequence.push(key_input);
                // Preserve the already-typed outer count when transitioning into a
                // pending sequence. Reusing `pending_count` would lose this value
                // when inner motion digits are typed (e.g. `2d3iw`).
                self.pending_sequence_count = self.pending_count.take();
                self.pending_sequence_motion_count = None;
                return;
            }
        }

        // First check bindings map
        let binding = self.keybindings.get_binding(key, &self.mode).cloned();
        if let Some(actions) = binding.as_ref() {
            let count = self.pending_count.take();
            self.execute_actions_with_count(actions, count);
            return;
        }

        // Handle insertable characters for insert/command/search modes
        if let Some(c) = KeyBindings::is_insertable_char(key) {
            if self.mode_uses_modal_bindings() {
                // Unbound key in normal mode - ignore
                self.pending_count = None;
                return;
            }

            if self.mode == Mode::Insert {
                self.insert_char(c);
            } else {
                self.mode.append_char(c);
            }
        }

        if self.mode_uses_modal_bindings() {
            self.pending_count = None;
        }
    }

    /// Execute one or more actions with an optional Normal-mode count prefix.
    /// Execute a borrowed action binding, repeating whole multi-action sequences for counts.
    fn execute_actions_with_count(&mut self, actions: &ActionBinding, count: Option<usize>) {
        match actions {
            ActionBinding::Single(action) => {
                self.execute_action_with_count(*action, count);
            }
            ActionBinding::Multiple(actions) => {
                let repeats = count.map_or(1, |value| value.clamp(1, Self::MAX_COUNT));
                for _ in 0..repeats {
                    for action in actions.iter().copied() {
                        self.execute_action(action);
                    }
                }
            }
        }
    }

    /// Execute one action with an optional Normal-mode count prefix.
    ///
    /// Repeat-oriented actions use capped counts, while line-targeting `G`/`gg`
    /// use the raw parsed line number (no `MAX_COUNT` cap).
    fn execute_action_with_count(&mut self, action: Action, count: Option<usize>) {
        let Some(count) = count else {
            self.execute_action(action);
            return;
        };
        self.reset_wrapped_goal_if_needed(action);
        let raw_count = count.max(1);
        let count = raw_count.min(Self::MAX_COUNT);
        match action {
            Action::MoveLeft => {
                self.cursor.move_left_normal_by(count);
                self.finish_counted_normal_action();
            }
            Action::MoveRight => {
                self.cursor.move_right_normal_by(&self.buffer, count);
                self.finish_counted_normal_action();
            }
            Action::MoveUp => {
                self.move_up_for_current_wrap_mode_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveDown => {
                self.move_down_for_current_wrap_mode_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveWordForward => {
                self.move_word_forward_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveWordBackward => {
                self.move_word_backward_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveWordEnd => {
                self.move_word_end_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveParagraphForward => {
                self.move_paragraph_forward_count(count);
                self.finish_counted_normal_action();
            }
            Action::MoveParagraphBackward => {
                self.move_paragraph_backward_count(count);
                self.finish_counted_normal_action();
            }
            Action::DeleteCharAtCursor => {
                self.delete_char_at_cursor_count(count);
                self.finish_counted_normal_action();
            }
            Action::DeleteInnerWord => {
                self.delete_inner_word_count(count);
                self.finish_counted_normal_action();
            }
            Action::DeleteAroundParen => {
                self.delete_around_paren_count(count);
                self.finish_counted_normal_action();
            }
            Action::ChangeInnerWord => {
                self.change_inner_word_count(count);
                self.finish_counted_normal_action();
            }
            Action::PageUp => {
                self.viewport
                    .page_up_by(&mut self.cursor, &self.buffer, count);
            }
            Action::PageDown => {
                self.viewport
                    .page_down_by(&mut self.cursor, &self.buffer, count);
            }
            Action::HalfPageUp => {
                self.viewport
                    .half_page_up_by(&mut self.cursor, &self.buffer, count);
            }
            Action::HalfPageDown => {
                self.viewport
                    .half_page_down_by(&mut self.cursor, &self.buffer, count);
            }
            Action::SearchNext => self.repeat_search_count(true, count),
            Action::SearchPrevious => self.repeat_search_count(false, count),
            Action::MoveToLastLine | Action::MoveToFirstLine => {
                self.goto_line(raw_count);
                self.cursor.clamp_to_line_normal(&self.buffer);
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::FindForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Forward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::FindBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Backward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::TillForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Forward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::TillBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Backward,
                    count,
                });
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            Action::RepeatFindForward => self.repeat_find(false, count),
            Action::RepeatFindBackward => self.repeat_find(true, count),
            _ => {
                // Non-repeatable actions with a count execute once and clear the count.
                self.execute_action(action);
            }
        }
    }

    /// Normalize cursor and viewport once after count-aware normal-mode actions.
    fn finish_counted_normal_action(&mut self) {
        self.cursor.clamp_to_line_normal(&self.buffer);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Return whether the current mode uses normal-style motion and count handling.
    pub(crate) fn mode_uses_modal_bindings(&self) -> bool {
        self.mode.is_normal() || self.mode.is_visual()
    }

    /// Return whether an action should preserve wrapped-row column intent.
    fn preserves_wrapped_goal(action: Action) -> bool {
        matches!(action, Action::MoveUp | Action::MoveDown)
    }

    /// Clear wrapped-row column intent when a different action takes over.
    fn reset_wrapped_goal_if_needed(&mut self, action: Action) {
        if !self.soft_wrap_enabled() || !Self::preserves_wrapped_goal(action) {
            self.desired_visual_column = None;
        }
    }

    /// Execute one upward movement using the active wrap mode.
    fn move_up_for_current_wrap_mode(&mut self) {
        if self.soft_wrap_enabled() {
            self.move_up_wrapped();
        } else if self.mode_uses_modal_bindings() {
            self.cursor.move_up_normal(&self.buffer);
        } else {
            self.cursor.move_up(&self.buffer);
        }
    }

    /// Execute one downward movement using the active wrap mode.
    fn move_down_for_current_wrap_mode(&mut self) {
        if self.soft_wrap_enabled() {
            self.move_down_wrapped();
        } else if self.mode_uses_modal_bindings() {
            self.cursor.move_down_normal(&self.buffer);
        } else {
            self.cursor.move_down(&self.buffer);
        }
    }

    /// Execute an upward counted movement using the active wrap mode.
    fn move_up_for_current_wrap_mode_count(&mut self, count: usize) {
        if self.soft_wrap_enabled() {
            self.move_up_wrapped_count(count);
        } else {
            self.cursor.move_up_normal_by(&self.buffer, count);
        }
    }

    /// Execute a downward counted movement using the active wrap mode.
    fn move_down_for_current_wrap_mode_count(&mut self, count: usize) {
        if self.soft_wrap_enabled() {
            self.move_down_wrapped_count(count);
        } else {
            self.cursor.move_down_normal_by(&self.buffer, count);
        }
    }

    /// Execute one logical action without a count prefix.
    ///
    /// NOTE: when adding or changing action behavior, verify whether
    /// `execute_action_with_count` needs the same update for counted execution.
    fn execute_action(&mut self, action: Action) {
        self.reset_wrapped_goal_if_needed(action);
        match action {
            // Navigation
            Action::MoveLeft => {
                if self.mode_uses_modal_bindings() {
                    self.cursor.move_left_normal();
                } else {
                    self.cursor.move_left(&self.buffer);
                }
            }
            Action::MoveRight => {
                if self.mode_uses_modal_bindings() {
                    self.cursor.move_right_normal(&self.buffer);
                } else {
                    self.cursor.move_right(&self.buffer);
                }
            }
            Action::MoveUp => {
                self.move_up_for_current_wrap_mode();
            }
            Action::MoveDown => {
                self.move_down_for_current_wrap_mode();
            }
            Action::MoveWordForward => self.move_word_forward(),
            Action::MoveWordBackward => self.move_word_backward(),
            Action::MoveWordEnd => self.move_word_end(),
            Action::MoveParagraphForward => self.move_paragraph_forward(),
            Action::MoveParagraphBackward => self.move_paragraph_backward(),
            Action::MoveLineStart => self.cursor.move_to_line_start(),
            Action::MoveLineEnd => self.cursor.move_to_line_end(&self.buffer),
            Action::MovePastLineEnd => self.cursor.move_past_line_end(&self.buffer),
            Action::MoveFirstNonBlank => self.move_first_non_blank(),
            Action::MoveToFirstLine => self.move_to_first_line(),
            Action::MoveToLastLine => self.move_to_last_line(),
            Action::PageUp => self.viewport.page_up(&mut self.cursor, &self.buffer),
            Action::PageDown => self.viewport.page_down(&mut self.cursor, &self.buffer),
            Action::HalfPageUp => self.viewport.half_page_up(&mut self.cursor, &self.buffer),
            Action::HalfPageDown => self.viewport.half_page_down(&mut self.cursor, &self.buffer),
            Action::FindForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Forward,
                    count: 1,
                });
            }
            Action::FindBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Find,
                    direction: FindDirection::Backward,
                    count: 1,
                });
            }
            Action::TillForward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Forward,
                    count: 1,
                });
            }
            Action::TillBackward => {
                self.begin_find_motion(FindMotion {
                    kind: FindMotionKind::Till,
                    direction: FindDirection::Backward,
                    count: 1,
                });
            }
            Action::RepeatFindForward => self.repeat_find(false, 1),
            Action::RepeatFindBackward => self.repeat_find(true, 1),

            // Mode switching
            Action::EnterInsertMode => self.enter_insert_mode(),
            Action::EnterVisualMode => self.enter_visual_mode(VisualKind::Character),
            Action::EnterVisualLineMode => self.enter_visual_mode(VisualKind::Line),
            Action::InsertAfterCursor => self.insert_after_cursor(),
            Action::OpenLineBelow => self.open_line_below(),
            Action::OpenLineAbove => self.open_line_above(),
            Action::EnterCommandMode => self.mode = Mode::command_empty(),
            Action::EnterSearchMode => self.mode = Mode::search_empty(),
            Action::ExitToNormalMode => self.exit_visual_mode(),
            Action::SearchNext => self.repeat_search(true),
            Action::SearchPrevious => self.repeat_search(false),
            Action::SaveCurrentFile => self.request_save_current(
                OverwriteBehavior::ConfirmIfDifferentPath,
                PostSaveAction::StayOpen,
            ),
            Action::SaveCurrentFileAndQuit => self.request_save_current(
                OverwriteBehavior::ConfirmIfDifferentPath,
                PostSaveAction::QuitOnSuccess,
            ),

            // Insert mode
            Action::DeleteCharBackward => self.delete_char_backward(),
            Action::DeleteCharForward => self.delete_char_forward(),
            Action::DeleteCharAtCursor => self.delete_char_at_cursor(),
            Action::DeleteWordBackward => self.delete_word_backward(),
            Action::DeleteToLineStart => self.delete_to_line_start(),
            Action::InsertNewline => self.insert_newline(),
            Action::DeleteSelection => self.delete_visual_selection(false),
            Action::ChangeSelection => self.delete_visual_selection(true),
            Action::ChangeInnerWord => self.change_inner_word(),
            Action::DeleteInnerWord => self.delete_inner_word(),
            Action::DeleteAroundParen => self.delete_around_paren(),

            // Command/Search mode
            Action::ExecuteCommand => self.execute_command(),
            Action::CancelCommand => self.mode = Mode::Normal,
            Action::DeleteInputChar => self.delete_input_char(),
            Action::DeleteInputCharForward => self.delete_input_char_forward(),
            Action::DeleteInputWordBackward => self.delete_input_word_backward(),
            Action::DeleteInputToStart => self.delete_input_to_start(),
            Action::DeleteInputToEnd => self.delete_input_to_end(),
            Action::MoveInputStart => self.move_input_start(),
            Action::MoveInputEnd => self.move_input_end(),
            Action::MoveInputLeft => self.move_input_left(),
            Action::MoveInputRight => self.move_input_right(),
            Action::MoveInputWordLeft => self.move_input_word_left(),
            Action::MoveInputWordRight => self.move_input_word_right(),
        }

        // In normal mode, cursor must stay on a real character for non-empty lines.
        if self.mode_uses_modal_bindings() {
            self.cursor.clamp_to_line_normal(&self.buffer);
        } else {
            self.pending_sequence.clear();
            self.pending_sequence_count = None;
            self.pending_sequence_motion_count = None;
            self.pending_find = None;
        }

        // Ensure cursor is visible after any action
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Move the cursor by wrapped screen rows instead of buffer lines.
    fn move_wrapped_rows(&mut self, count: usize, forward: bool) {
        let width = self.viewport.width().max(1);
        let normal_mode = self.mode_uses_modal_bindings();
        let line_len = self.buffer.line_len(self.cursor.line());
        let current_visual = soft_wrap::visual_cursor(
            self.cursor.column(),
            line_len,
            width,
            normal_mode,
            self.cursor.line(),
        );
        let desired_visual_column = self.desired_visual_column.unwrap_or(current_visual.column);
        let mut target_position = current_visual.position;

        // Wrapped-row movement is bounded by the requested count and shares the
        // same stepping primitives as wrapped rendering and viewport scrolling.
        if forward {
            target_position =
                soft_wrap::advance_visual_position(target_position, &self.buffer, width, count);
        } else {
            target_position =
                soft_wrap::retreat_visual_position(target_position, &self.buffer, width, count);
        }

        let target_len = self.buffer.line_len(target_position.line);
        let target_column = soft_wrap::buffer_column_for_visual_column(
            target_position.row,
            desired_visual_column,
            target_len,
            width,
            normal_mode,
        );
        self.cursor = Cursor::new(target_position.line, target_column);
        self.desired_visual_column = Some(desired_visual_column);
    }

    /// Move up by one wrapped screen row.
    fn move_up_wrapped(&mut self) {
        self.move_wrapped_rows(1, false);
    }

    /// Move down by one wrapped screen row.
    fn move_down_wrapped(&mut self) {
        self.move_wrapped_rows(1, true);
    }

    /// Move up by `count` wrapped screen rows.
    fn move_up_wrapped_count(&mut self, count: usize) {
        self.move_wrapped_rows(count, false);
    }

    /// Move down by `count` wrapped screen rows.
    fn move_down_wrapped_count(&mut self, count: usize) {
        self.move_wrapped_rows(count, true);
    }

    fn move_word_forward(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_next_word_start(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `w`-style motion repeatedly while avoiding per-step viewport work.
    fn move_word_forward_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_forward();
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    fn move_word_backward(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_prev_word_start(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `b`-style motion repeatedly while avoiding per-step viewport work.
    fn move_word_backward_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_backward();
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    fn move_word_end(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let new_idx = find_word_end(&self.buffer, char_idx);
        self.cursor = Cursor::from_char_index(&self.buffer, new_idx);
    }

    /// Apply `e`-style motion repeatedly while avoiding per-step viewport work.
    fn move_word_end_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.move_word_end();
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    fn move_paragraph_forward(&mut self) {
        let target_line = find_next_paragraph_line(&self.buffer, self.cursor.line());
        self.cursor = Cursor::new(target_line, self.cursor.desired_column());
    }

    /// Apply `}` paragraph motion repeatedly while preserving desired column.
    fn move_paragraph_forward_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.line();
            self.move_paragraph_forward();
            if self.cursor.line() == before {
                break;
            }
        }
    }

    fn move_paragraph_backward(&mut self) {
        let target_line = find_prev_paragraph_line(&self.buffer, self.cursor.line());
        self.cursor = Cursor::new(target_line, self.cursor.desired_column());
    }

    /// Apply `{` paragraph motion repeatedly while preserving desired column.
    fn move_paragraph_backward_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.cursor.line();
            self.move_paragraph_backward();
            if self.cursor.line() == before {
                break;
            }
        }
    }

    fn move_first_non_blank(&mut self) {
        if let Some(line) = self.buffer.line(self.cursor.line()) {
            let mut col = 0;
            for c in line.chars() {
                if !c.is_whitespace() {
                    break;
                }
                col += 1;
            }
            self.cursor.set_column(col);
        }
    }

    fn move_to_last_line(&mut self) {
        let last_line = self.buffer.lines_count().saturating_sub(1);
        self.cursor = Cursor::new(last_line, 0);
    }

    fn move_to_first_line(&mut self) {
        self.cursor = Cursor::new(0, self.cursor.desired_column());
    }

    /// Enter visual mode or toggle/switch between the supported visual variants.
    fn enter_visual_mode(&mut self, kind: VisualKind) {
        match self.mode {
            Mode::Visual(current) if current == kind => self.exit_visual_mode(),
            Mode::Visual(_) => self.mode = Mode::Visual(kind),
            _ => {
                self.visual_anchor = Some(self.cursor.clone());
                self.mode = Mode::Visual(kind);
            }
        }
    }

    /// Leave visual mode and clear any active selection anchor.
    fn exit_visual_mode(&mut self) {
        self.visual_anchor = None;
        self.mode = Mode::Normal;
    }

    /// Clear any active visual selection and switch into insert mode.
    fn enter_insert_mode(&mut self) {
        self.visual_anchor = None;
        self.mode = Mode::Insert;
    }

    fn begin_find_motion(&mut self, motion: FindMotion) {
        self.pending_sequence.clear();
        self.pending_sequence_count = None;
        self.pending_sequence_motion_count = None;
        self.pending_find = Some(motion);
    }

    /// Consume one key while a find/till motion is pending.
    ///
    /// Returns `true` when this function consumed the key.
    fn handle_pending_find_key(&mut self, key: Key) -> bool {
        let Some(motion) = self.pending_find else {
            return false;
        };
        if !self.mode_uses_modal_bindings() {
            self.pending_find = None;
            return false;
        }

        if matches!(key, Key::Esc) {
            self.pending_find = None;
            return true;
        }

        if let Some(target) = KeyBindings::is_insertable_char(key) {
            self.pending_find = None;
            self.apply_find_motion(motion, target, true);
            self.finish_counted_normal_action();
        }

        // While waiting for find target, consume all keys to avoid accidental mode switches.
        true
    }

    /// Consume one key while a multi-key normal-mode sequence is pending.
    ///
    /// Returns `true` when this function consumed the key.
    fn handle_pending_sequence_key(&mut self, key: Key) -> bool {
        if !self.mode_uses_modal_bindings() || self.pending_sequence.is_empty() {
            return false;
        }

        if matches!(key, Key::Esc) {
            self.pending_sequence.clear();
            self.pending_sequence_count = None;
            self.pending_sequence_motion_count = None;
            return true;
        }

        if self.pending_sequence_allows_motion_count()
            && let Some(digit) = Self::key_count_digit(key)
            && let Some(next) = Self::append_count_digit(self.pending_sequence_motion_count, digit)
        {
            self.pending_sequence_motion_count = Some(next);
            return true;
        }

        self.pending_sequence.push(KeyInput::from(key));
        match self
            .keybindings
            .match_sequence(&self.mode, &self.pending_sequence)
        {
            SequenceMatch::Exact(actions) => {
                self.pending_sequence.clear();
                let count = self.take_sequence_count();
                self.execute_actions_with_count(&actions, count);
            }
            SequenceMatch::Prefix => {}
            SequenceMatch::NoMatch => {
                self.pending_sequence.clear();
                self.pending_sequence_count = None;
                self.pending_sequence_motion_count = None;
            }
        }
        true
    }

    /// Capture normal-mode count prefixes before resolving actions.
    ///
    /// Returns `true` when the key was consumed as part of count parsing.
    fn handle_pending_count_key(&mut self, key: Key) -> bool {
        // Count prefixes are only meaningful in plain Normal-mode dispatch.
        if !self.mode_uses_modal_bindings()
            || !self.pending_sequence.is_empty()
            || self.pending_find.is_some()
        {
            return false;
        }
        // Esc cancels a partially typed numeric prefix.
        if matches!(key, Key::Esc) && self.pending_count.is_some() {
            self.pending_count = None;
            return true;
        }

        let Some(digit) = Self::key_count_digit(key) else {
            return false;
        };
        let Some(next) = Self::append_count_digit(self.pending_count, digit) else {
            return false;
        };
        // Keep the parsed count pending until an action consumes it.
        self.pending_count = Some(next);
        true
    }

    /// Extract a numeric digit eligible for count parsing from key input.
    fn key_count_digit(key: Key) -> Option<char> {
        match key {
            Key::Char(c) if c.is_ascii_digit() => Some(c),
            _ => None,
        }
    }

    /// Append one count digit with Vim-like leading-zero rules and count capping.
    fn append_count_digit(current: Option<usize>, digit: char) -> Option<usize> {
        if !digit.is_ascii_digit() {
            return None;
        }
        if digit == '0' && current.is_none() {
            return None;
        }
        let digit_value = (digit as u8 - b'0') as usize;
        let next = current
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit_value);
        Some(next)
    }

    /// Whether the pending key prefix supports an in-sequence motion count.
    fn pending_sequence_allows_motion_count(&self) -> bool {
        matches!(
            self.pending_sequence.as_slice(),
            [KeyInput::Char('d')] | [KeyInput::Char('c')]
        )
    }

    /// Merge outer and motion counts for operator+motion flows using multiplication.
    fn take_sequence_count(&mut self) -> Option<usize> {
        let outer = self.pending_sequence_count.take();
        let inner = self.pending_sequence_motion_count.take();
        match (outer, inner) {
            (None, None) => None,
            (Some(o), None) => Some(o),
            (None, Some(i)) => Some(i),
            (Some(o), Some(i)) => Some(o.saturating_mul(i).min(Self::MAX_COUNT)),
        }
    }

    /// Apply an `f/F/t/T` motion with all-or-nothing counted target resolution.
    fn apply_find_motion(&mut self, motion: FindMotion, target: char, update_last_find: bool) {
        if update_last_find {
            self.last_find = Some(LastFind { motion, target });
        }

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let mut search_from = cursor_idx;
        let mut target_idx = None;
        for _ in 0..motion.count {
            let Some(idx) = self.find_char_on_current_line(search_from, motion.direction, target)
            else {
                return;
            };
            target_idx = Some(idx);
            search_from = idx;
        }
        let Some(target_idx) = target_idx else {
            return;
        };

        let destination = match (motion.kind, motion.direction) {
            (FindMotionKind::Find, _) => target_idx,
            (FindMotionKind::Till, FindDirection::Forward) => target_idx.saturating_sub(1),
            (FindMotionKind::Till, FindDirection::Backward) => target_idx.saturating_add(1),
        };

        self.cursor = Cursor::from_char_index(&self.buffer, destination);
    }

    /// Return the current visual selection as an exclusive character-index range.
    pub(crate) fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.visual_anchor.as_ref()?;
        let kind = match self.mode {
            Mode::Visual(kind) => kind,
            _ => return None,
        };

        match kind {
            // Characterwise visual mode uses inclusive cursor endpoints, so the
            // selection extends one char beyond the furthest endpoint.
            VisualKind::Character => {
                let anchor_idx = anchor.to_char_index(&self.buffer);
                let cursor_idx = self.cursor.to_char_index(&self.buffer);
                let start = anchor_idx.min(cursor_idx);
                let end = anchor_idx.max(cursor_idx).saturating_add(1);
                Some((start, end.min(self.buffer.chars_count())))
            }
            // Linewise mode expands to full logical-line boundaries so edits and
            // highlighting stay consistent regardless of cursor columns.
            VisualKind::Line => {
                let start_line = anchor.line().min(self.cursor.line());
                let end_line = anchor.line().max(self.cursor.line());
                let start = self.buffer.line_to_char(start_line);
                let end = if end_line + 1 < self.buffer.lines_count() {
                    self.buffer.line_to_char(end_line + 1)
                } else {
                    self.buffer.chars_count()
                };
                Some((start, end))
            }
        }
    }

    /// Return the normalized selection range and visual kind together.
    fn normalized_selection(&self) -> Option<(SelectionRange, VisualKind)> {
        let kind = match self.mode {
            Mode::Visual(kind) => kind,
            _ => return None,
        };
        let (start, end) = self.selection_range()?;
        Some((SelectionRange { start, end }, kind))
    }

    /// Repeat the last find motion up to `count` times, stopping at first no-op.
    fn repeat_find(&mut self, reverse_direction: bool, count: usize) {
        let Some(last) = self.last_find else {
            return;
        };

        let direction = if reverse_direction {
            match last.motion.direction {
                FindDirection::Forward => FindDirection::Backward,
                FindDirection::Backward => FindDirection::Forward,
            }
        } else {
            last.motion.direction
        };

        let motion = FindMotion {
            kind: last.motion.kind,
            direction,
            count: 1,
        };
        for _ in 0..count {
            let before = self.cursor.clone();
            self.apply_find_motion(motion, last.target, false);
            if self.cursor == before {
                break;
            }
        }
    }

    /// Find the next matching target index on the current line in the given direction.
    fn find_char_on_current_line(
        &self,
        cursor_idx: usize,
        direction: FindDirection,
        target: char,
    ) -> Option<usize> {
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let line_len = self.buffer.line_len(self.cursor.line());
        let line_end_exclusive = line_start + line_len;

        match direction {
            FindDirection::Forward => ((cursor_idx.saturating_add(1)).min(line_end_exclusive)
                ..line_end_exclusive)
                .find(|&idx| self.buffer.char_at(idx) == Some(target)),
            FindDirection::Backward => {
                if cursor_idx <= line_start {
                    return None;
                }
                (line_start..cursor_idx)
                    .rev()
                    .find(|&idx| self.buffer.char_at(idx) == Some(target))
            }
        }
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
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.insert_buffer_text(line_end, "\n");
        self.cursor = Cursor::new(line + 1, 0);
        self.enter_insert_mode();
    }

    fn insert_after_cursor(&mut self) {
        let line_len = self.buffer.line_len(self.cursor.line());
        if line_len > 0 {
            self.cursor.move_right(&self.buffer);
        }
        self.enter_insert_mode();
    }

    /// Open a new line above the cursor and enter insert mode.
    fn open_line_above(&mut self) {
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
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let line_len = self.buffer.line_len(self.cursor.line());
        if line_len > 0 {
            self.remove_buffer_range(char_idx, char_idx + 1);
        }
    }

    /// Delete up to `count` characters from the cursor to line end for counted `x`.
    fn delete_char_at_cursor_count(&mut self, count: usize) {
        let line_start = self.buffer.line_to_char(self.cursor.line());
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let line_len = self.buffer.line_len(self.cursor.line());
        if line_len == 0 {
            return;
        }
        let line_end = line_start + line_len;
        let end = char_idx.saturating_add(count).min(line_end);
        self.remove_buffer_range(char_idx, end);
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
        if !self.mode.is_normal() {
            return;
        }

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let Some((start, end)) = find_inner_word_span(&self.buffer, cursor_idx) else {
            return;
        };

        if start >= end {
            return;
        }

        self.remove_buffer_range(start, end);
        self.cursor = Cursor::from_char_index(&self.buffer, start);
    }

    /// Repeat `diw` semantics up to `count` times and stop at the first no-op.
    fn delete_inner_word_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.buffer.chars_count();
            self.delete_inner_word();
            if self.buffer.chars_count() == before {
                break;
            }
        }
    }

    fn change_inner_word(&mut self) {
        if !self.mode.is_normal() {
            return;
        }

        let before = self.buffer.chars_count();
        self.delete_inner_word();
        if self.buffer.chars_count() < before {
            self.enter_insert_mode();
        }
    }

    /// Repeat `ciw` deletions up to `count` times, then enter insert if anything changed.
    fn change_inner_word_count(&mut self, count: usize) {
        let before_total = self.buffer.chars_count();
        self.delete_inner_word_count(count);
        if self.buffer.chars_count() < before_total {
            self.enter_insert_mode();
        }
    }

    fn delete_around_paren(&mut self) {
        if !self.mode.is_normal() {
            return;
        }

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let Some((start, end)) = find_around_paren_span(&self.buffer, cursor_idx) else {
            return;
        };

        if start >= end {
            return;
        }

        self.remove_buffer_range(start, end);
        self.cursor = Cursor::from_char_index(&self.buffer, start);
    }

    /// Repeat `da(` semantics up to `count` times and stop at the first no-op.
    fn delete_around_paren_count(&mut self, count: usize) {
        for _ in 0..count {
            let before = self.buffer.chars_count();
            self.delete_around_paren();
            if self.buffer.chars_count() == before {
                break;
            }
        }
    }

    /// Delete the active visual selection and optionally enter insert mode.
    fn delete_visual_selection(&mut self, enter_insert: bool) {
        let Some((selection, kind)) = self.normalized_selection() else {
            return;
        };

        if selection.end > selection.start {
            self.remove_buffer_range(selection.start, selection.end);
        }

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

        if enter_insert {
            self.enter_insert_mode();
        } else {
            self.exit_visual_mode();
        }
    }

    /// Execute the current command/search input and apply side effects.
    ///
    /// Command mode supports save/quit commands and numeric go-to-line input.
    fn execute_command(&mut self) {
        if let Some(pattern) = self.mode.take_search_input() {
            self.execute_search(&pattern);
            return;
        }

        if let Some(command) = self.mode.take_command_input() {
            let trimmed = command.trim();

            // Check for line number (go-to line)
            if let Ok(line_num) = trimmed.parse::<usize>() {
                self.goto_line(line_num);
                return;
            }

            // Parse command and arguments
            let (cmd, arg) = match trimmed.split_once(' ') {
                Some((c, a)) => (c, Some(a.trim())),
                None => (trimmed, None),
            };

            match (cmd, arg) {
                ("q", None) => {
                    if self.buffer.is_modified() {
                        self.pending_quit_confirmation = Some(PendingQuitConfirmation {
                            post_save_action: PostSaveAction::QuitOnSuccess,
                        });
                        self.status_message = None;
                    } else {
                        self.should_quit = true;
                    }
                }
                ("q!", None) => {
                    self.should_quit = true;
                }
                ("w", None) => {
                    self.request_save_current(
                        OverwriteBehavior::ConfirmIfDifferentPath,
                        PostSaveAction::StayOpen,
                    );
                }
                ("w!", None) => {
                    self.request_save_current(OverwriteBehavior::Force, PostSaveAction::StayOpen);
                }
                ("w", Some(filename)) | ("write", Some(filename)) => {
                    self.request_save_as(filename, OverwriteBehavior::ConfirmIfDifferentPath);
                }
                ("w!", Some(filename)) => {
                    self.request_save_as(filename, OverwriteBehavior::Force);
                }
                ("wq", None) => {
                    self.request_save_current(
                        OverwriteBehavior::ConfirmIfDifferentPath,
                        PostSaveAction::QuitOnSuccess,
                    );
                }
                ("wq!", None) => {
                    self.request_save_current(
                        OverwriteBehavior::Force,
                        PostSaveAction::QuitOnSuccess,
                    );
                }
                ("reload-config", None) => {
                    self.pending_request = Some(EditorRequest::ReloadConfig);
                }
                _ => {
                    self.status_message = Some(format!("Unknown command: {}", trimmed));
                }
            }
        }
    }

    fn execute_search(&mut self, pattern: &str) {
        if pattern.is_empty() {
            self.status_message = Some("Pattern not found".to_string());
            return;
        }

        self.last_search_pattern = Some(pattern.to_string());

        // Search from current position.
        let start_idx = self.cursor.to_char_index(&self.buffer);
        if let Some(found_idx) = self.buffer.find(pattern, start_idx) {
            self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        } else {
            // Wrap around to beginning
            if let Some(found_idx) = self.buffer.find(pattern, 0) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                self.viewport
                    .ensure_cursor_visible(&self.cursor, &self.buffer);
                self.status_message = Some("Search wrapped to beginning".to_string());
            } else {
                self.status_message = Some("Pattern not found".to_string());
            }
        }
    }

    fn repeat_search(&mut self, forward: bool) {
        let Some(pattern) = self.last_search_pattern.clone() else {
            self.status_message = Some("No previous search".to_string());
            return;
        };

        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let total_chars = self.buffer.chars_count();

        if forward {
            let start_idx = cursor_idx.saturating_add(1);
            if let Some(found_idx) = self.buffer.find(&pattern, start_idx) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                return;
            }

            if let Some(found_idx) = self.buffer.find(&pattern, 0) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                self.status_message = Some("Search wrapped to beginning".to_string());
            } else {
                self.status_message = Some("Pattern not found".to_string());
            }
        } else {
            if let Some(found_idx) = self.buffer.find_backward(&pattern, cursor_idx) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                return;
            }

            if let Some(found_idx) = self.buffer.find_backward(&pattern, total_chars) {
                self.cursor = Cursor::from_char_index(&self.buffer, found_idx);
                self.status_message = Some("Search wrapped to end".to_string());
            } else {
                self.status_message = Some("Pattern not found".to_string());
            }
        }
    }

    /// Repeat search motion `count` times while preserving existing wrap/error behavior.
    fn repeat_search_count(&mut self, forward: bool, count: usize) {
        for _ in 0..count {
            let before = self.cursor.to_char_index(&self.buffer);
            self.repeat_search(forward);
            if self.cursor.to_char_index(&self.buffer) == before {
                break;
            }
        }
    }

    fn goto_line(&mut self, line_num: usize) {
        let total_lines = self.buffer.lines_count();
        let target_line = if line_num == 0 {
            0
        } else if line_num > total_lines {
            self.status_message = Some(format!(
                "Line {} out of range, moved to last line",
                line_num
            ));
            total_lines.saturating_sub(1)
        } else {
            line_num - 1 // Convert to 0-indexed
        };

        self.cursor = Cursor::new(target_line, 0);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Request a save to the current file path.
    ///
    /// This centralizes `:w` and `:wq` behavior while keeping overwrite and
    /// post-save handling explicit at the callsite.
    fn request_save_current(
        &mut self,
        overwrite_behavior: OverwriteBehavior,
        post_save_action: PostSaveAction,
    ) {
        if self.file_path.as_os_str().is_empty() {
            self.status_message = Some("No file name".to_string());
            return;
        }

        self.request_save(
            self.file_path.clone(),
            false,
            overwrite_behavior,
            post_save_action,
        );
    }

    /// Request a save to a user-supplied path (`:w <path>` / `:write <path>`).
    fn request_save_as(&mut self, filename: &str, overwrite_behavior: OverwriteBehavior) {
        if filename.is_empty() {
            self.status_message = Some("No file name".to_string());
            return;
        }

        self.request_save(
            PathBuf::from(filename),
            true,
            overwrite_behavior,
            PostSaveAction::StayOpen,
        );
    }

    /// Shared save request pipeline for all write commands.
    ///
    /// It decides whether to queue an overwrite prompt or perform the write
    /// immediately, and applies the post-save action only on successful writes.
    fn request_save(
        &mut self,
        target_path: PathBuf,
        update_file_path: bool,
        overwrite_behavior: OverwriteBehavior,
        post_save_action: PostSaveAction,
    ) {
        if target_path.as_os_str().is_empty() {
            self.status_message = Some("No file name".to_string());
            return;
        }

        let needs_overwrite_confirmation = overwrite_behavior
            == OverwriteBehavior::ConfirmIfDifferentPath
            && target_path.exists()
            && self.file_path != target_path;

        if needs_overwrite_confirmation {
            self.pending_overwrite = Some(PendingOverwrite {
                target_path,
                update_file_path,
                post_save_action,
            });
            self.status_message = None;
            return;
        }

        let save_ok = self.save_to_path(target_path, update_file_path);
        if save_ok && post_save_action == PostSaveAction::QuitOnSuccess {
            self.should_quit = true;
        }
    }

    /// Execute the actual write-to-disk operation and update editor state.
    ///
    /// Returns `true` when write succeeded and state was updated, otherwise
    /// sets an error status message and returns `false`.
    fn save_to_path(&mut self, path: PathBuf, update_file_path: bool) -> bool {
        match File::create(&path) {
            Ok(mut file) => match self.buffer.write_to(&mut file) {
                Ok(()) => {
                    if update_file_path {
                        self.file_path = path.clone();
                        self.refresh_syntax();
                    }
                    self.buffer.clear_modified();
                    self.status_message = Some(format!("\"{}\" written", path.display()));
                    true
                }
                Err(e) => {
                    self.status_message = Some(format!("Error writing file: {}", e));
                    false
                }
            },
            Err(e) => {
                self.status_message = Some(format!("Error creating file: {}", e));
                false
            }
        }
    }

    /// Consume one key while an overwrite prompt is pending.
    ///
    /// `y`/`Y` confirms and executes the deferred write; any other key cancels.
    /// Returns `true` when this function consumed the key.
    fn handle_pending_overwrite_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_overwrite.take() else {
            return false;
        };

        let confirmed = key == Key::Char('y') || key == Key::Char('Y');
        if confirmed {
            let save_ok = self.save_to_path(pending.target_path, pending.update_file_path);
            if save_ok && pending.post_save_action == PostSaveAction::QuitOnSuccess {
                self.should_quit = true;
            }
        } else {
            self.status_message = Some("Write cancelled".to_string());
        }

        true
    }

    /// Get the current mode name for display
    pub(crate) fn mode_name(&self) -> &str {
        self.mode.mode_label()
    }

    /// Return the terminal cursor shape for the active editor mode.
    pub(crate) fn cursor_shape(&self) -> tui::CursorShape {
        if self.mode.uses_beam_cursor() {
            return tui::CursorShape::Beam;
        }

        tui::CursorShape::Block
    }

    /// Get the command/search input string for display
    pub(crate) fn input_line(&self) -> Option<&str> {
        self.mode
            .command_string()
            .or_else(|| self.mode.search_string())
    }

    /// Get the prompt character for command/search mode
    pub(crate) fn input_prompt(&self) -> Option<char> {
        match &self.mode {
            Mode::Command(_) => Some(':'),
            Mode::Search(_) => Some('/'),
            _ => None,
        }
    }

    pub(crate) fn input_cursor_column(&self) -> Option<usize> {
        self.mode.input_cursor().map(|cursor| cursor + 1)
    }

    pub(crate) fn overwrite_prompt(&self) -> Option<String> {
        self.pending_overwrite
            .as_ref()
            .map(|pending| format!("Overwrite \"{}\"? [y/N]", pending.target_path.display()))
    }

    pub(crate) fn quit_prompt(&self) -> Option<String> {
        if self.pending_quit_confirmation.is_none() {
            return None;
        }

        let file_name = self
            .file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("[No Name]");
        Some(format!(
            "Save changes to \"{}\"? [y]es/[n]o/[c]ancel",
            file_name
        ))
    }

    /// Get a short pending multi-key prefix label for UI display.
    pub(crate) fn pending_prefix_label(&self) -> Option<String> {
        if !self.mode_uses_modal_bindings() {
            return None;
        }

        if let Some(motion) = self.pending_find {
            let mut label = String::new();
            if motion.count > 1 {
                label.push_str(&motion.count.to_string());
            }
            let suffix = match (motion.kind, motion.direction) {
                (FindMotionKind::Find, FindDirection::Forward) => "f",
                (FindMotionKind::Find, FindDirection::Backward) => "F",
                (FindMotionKind::Till, FindDirection::Forward) => "t",
                (FindMotionKind::Till, FindDirection::Backward) => "T",
            };
            label.push_str(suffix);
            return Some(label);
        }

        if !self.pending_sequence.is_empty() {
            let mut label = String::new();
            if let Some(count) = self.pending_sequence_count {
                label.push_str(&count.to_string());
            }
            for key in &self.pending_sequence {
                label.push_str(&key.label());
            }
            if let Some(motion_count) = self.pending_sequence_motion_count {
                label.push_str(&motion_count.to_string());
            }
            return Some(label);
        }

        if let Some(count) = self.pending_count {
            return Some(count.to_string());
        }
        None
    }

    /// Build the discovery-popup model for the current pending multi-key sequence.
    pub(crate) fn sequence_discovery_popup(&self) -> Option<SequenceDiscoveryPopup> {
        if !self.sequence_discovery_popup_enabled()
            || !self.mode_uses_modal_bindings()
            || self.pending_sequence.is_empty()
        {
            return None;
        }

        let prefix = self.pending_prefix_label()?;
        let entries = self
            .keybindings
            .continuations_for_prefix(&self.mode, &self.pending_sequence)
            .into_iter()
            .map(|continuation| SequenceDiscoveryEntry {
                keys: continuation.keys_label(),
                action: continuation.action_label(),
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return None;
        }

        Some(SequenceDiscoveryPopup { prefix, entries })
    }

    /// Consume one key while a quit confirmation prompt is pending.
    ///
    /// `y`/`Y` saves and quits on success, `n`/`N` quits without saving, and
    /// any other key cancels quit.
    /// Returns `true` when this function consumed the key.
    fn handle_pending_quit_key(&mut self, key: Key) -> bool {
        let Some(pending) = self.pending_quit_confirmation.take() else {
            return false;
        };

        match key {
            Key::Char('y') | Key::Char('Y') => {
                self.request_save_current(
                    OverwriteBehavior::ConfirmIfDifferentPath,
                    pending.post_save_action,
                );
            }
            Key::Char('n') | Key::Char('N') => {
                self.should_quit = true;
            }
            _ => {
                self.status_message = Some("Quit cancelled".to_string());
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_editor_with_content(content: &str) -> EditorState {
        let mut editor = EditorState::new(24);
        editor.buffer = TextBuffer::from_str(content);
        editor
    }

    fn unique_temp_path(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos));
        path.to_string_lossy().to_string()
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

        editor.handle_key(Key::Esc);
        assert!(matches!(editor.mode, Mode::Normal));
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
    fn test_remove_newline_shrinks_syntax_cache_with_merged_lines() {
        let mut editor = create_editor_with_content("let alpha = 1;\nlet beta = 2;");
        editor.file_path = PathBuf::from("sample.rs");
        editor.refresh_syntax();

        let newline_idx = editor.buffer.line_to_char(0) + editor.buffer.line_len(0);
        editor.remove_buffer_range(newline_idx, newline_idx + 1);

        assert_eq!(editor.buffer.lines_count(), 1);
        assert_eq!(editor.syntax.document_state().line_states.len(), 1);
        assert_eq!(editor.syntax.document_state().spans_by_line.len(), 1);
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
        let target = unique_temp_path("ordex_test_save_as");
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::command_with_text(format!("w {}", target));

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.file_path.to_string_lossy(), target);
        assert!(!editor.buffer.is_modified());
        assert!(editor.status_message.as_ref().unwrap().contains("written"));

        // Cleanup
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_save_file_as_with_write_command() {
        let target = unique_temp_path("ordex_test_write");
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::command_with_text(format!("write {}", target));

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.file_path.to_string_lossy(), target);
        assert!(!editor.buffer.is_modified());

        // Cleanup
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_save_file_as_updates_file_path() {
        let target = unique_temp_path("ordex_new_file");
        let mut editor = create_editor_with_content("new file content");
        assert!(editor.file_path.as_os_str().is_empty());

        editor.mode = Mode::command_with_text(format!("w {}", target));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.file_path.to_string_lossy(), target);

        // Cleanup
        let _ = fs::remove_file(target);
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
        let target = unique_temp_path("ordex_confirm_write");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.mode = Mode::command_with_text("w");
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.overwrite_prompt(), None);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(
            editor
                .status_message
                .as_deref()
                .unwrap()
                .contains("written")
        );

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_space_w_writes_current_file() {
        let target = unique_temp_path("ordex_space_w");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('w'));

        assert!(!editor.should_quit);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_space_q_without_filename_does_not_quit() {
        let mut editor = create_editor_with_content("new");
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('q'));

        assert!(!editor.should_quit);
        assert_eq!(editor.status_message, Some("No file name".to_string()));
    }

    #[test]
    fn test_w_save_as_existing_file_cancel_keeps_target_unchanged() {
        let source = unique_temp_path("ordex_save_as_source");
        let target = unique_temp_path("ordex_confirm_cancel");
        fs::write(&source, "source_old").unwrap();
        fs::write(&target, "target_old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&source);
        editor.mode = Mode::command_with_text(format!("w {}", target));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.overwrite_prompt(),
            Some(format!("Overwrite \"{}\"? [y/N]", target))
        );
        editor.handle_key(Key::Esc);

        assert_eq!(fs::read_to_string(&target).unwrap(), "target_old");
        assert_eq!(editor.status_message, Some("Write cancelled".to_string()));
        assert_eq!(editor.file_path.to_string_lossy(), source);

        let _ = fs::remove_file(source);
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_w_bang_bypasses_confirmation_for_existing_file() {
        let target = unique_temp_path("ordex_force_write");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.mode = Mode::command_with_text("w!");
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.overwrite_prompt(), None);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_wq_current_file_writes_and_quits_without_confirmation() {
        let target = unique_temp_path("ordex_wq_cancel");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.mode = Mode::command_with_text("wq");
        editor.handle_key(Key::Char('\n'));

        assert!(editor.should_quit);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_space_q_writes_current_file_and_quits() {
        let target = unique_temp_path("ordex_space_q");
        fs::write(&target, "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = PathBuf::from(&target);
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('q'));

        assert!(editor.should_quit);
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        let _ = fs::remove_file(target);
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
