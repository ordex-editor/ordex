//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::config::ConfigSettings;
use crate::cursor::Cursor;
use crate::keybindings::{Action, ActionBinding, KeyBindings, KeyInput, SequenceMatch};
use crate::mode::Mode;
use crate::navigation::{
    find_around_paren_span, find_inner_word_span, find_next_paragraph_line, find_next_word_start,
    find_prev_paragraph_line, find_prev_word_start, find_word_end,
};
use crate::text_buffer::TextBuffer;
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

/// Editor state holding all components for the editor session
pub(crate) struct EditorState {
    /// The text buffer containing file content
    pub(crate) buffer: TextBuffer,
    /// Current cursor position
    pub(crate) cursor: Cursor,
    /// Current editor mode
    pub(crate) mode: Mode,
    /// Viewport for visible portion of document
    pub(crate) viewport: Viewport,
    /// Path to the file being edited
    pub(crate) file_path: PathBuf,
    /// Status message to display (cleared after one render)
    pub(crate) status_message: Option<String>,
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
}

impl EditorState {
    const INPUT_ESCAPE_SUPPRESS_DURATION: Duration = Duration::from_millis(30);
    /// Maximum repeat count applied to repeat-style actions to keep execution bounded.
    const MAX_COUNT: usize = 999_999;

    fn normalize_key(key: Key) -> Key {
        match key {
            Key::Char('\u{1b}') => Key::Esc,
            Key::Ctrl('[') => Key::Esc,
            other => other,
        }
    }

    /// Create a new editor state with an empty buffer
    pub(crate) fn new(terminal_height: usize) -> Self {
        Self {
            buffer: TextBuffer::new(),
            cursor: Cursor::new(0, 0),
            mode: Mode::Normal,
            viewport: Viewport::new(terminal_height.saturating_sub(2)), // Reserve 2 lines for status bar
            file_path: PathBuf::new(),
            status_message: None,
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
        }
    }

    /// Apply resolved configuration settings to the current editor state.
    pub(crate) fn apply_config(&mut self, settings: &ConfigSettings) {
        if let Some(margin) = settings.scroll_margin {
            self.viewport.set_scroll_margin(margin);
        }

        if let Some(margin) = settings.horizontal_scroll_margin {
            self.viewport.set_horizontal_scroll_margin(margin);
        }

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

    /// Load a file into the editor using chunked reading for efficiency
    pub(crate) fn load_file(&mut self, path: &str) -> std::io::Result<()> {
        let file = File::open(path)?;
        self.buffer = TextBuffer::from_reader(file)?;
        self.file_path = PathBuf::from(path);
        self.cursor = Cursor::new(0, 0);
        self.viewport.set_first_visible_line(0);
        Ok(())
    }

    /// Update viewport dimensions after a terminal resize.
    pub(crate) fn handle_resize(&mut self, terminal_width: usize, terminal_height: usize) {
        self.viewport.set_width(terminal_width);
        self.viewport.set_height(terminal_height.saturating_sub(2));
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

        if self.mode.is_normal() {
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
            if self.mode.is_normal() {
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

        if self.mode.is_normal() {
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
                self.cursor.move_up_normal_by(&self.buffer, count);
                self.finish_counted_normal_action();
            }
            Action::MoveDown => {
                self.cursor.move_down_normal_by(&self.buffer, count);
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
            Action::PageUp => self
                .viewport
                .page_up_by(&mut self.cursor, &self.buffer, count),
            Action::PageDown => self
                .viewport
                .page_down_by(&mut self.cursor, &self.buffer, count),
            Action::HalfPageUp => {
                self.viewport
                    .half_page_up_by(&mut self.cursor, &self.buffer, count)
            }
            Action::HalfPageDown => {
                self.viewport
                    .half_page_down_by(&mut self.cursor, &self.buffer, count)
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

    /// Execute one logical action without a count prefix.
    ///
    /// NOTE: when adding or changing action behavior, verify whether
    /// `execute_action_with_count` needs the same update for counted execution.
    fn execute_action(&mut self, action: Action) {
        match action {
            // Navigation
            Action::MoveLeft => {
                if self.mode.is_normal() {
                    self.cursor.move_left_normal();
                } else {
                    self.cursor.move_left(&self.buffer);
                }
            }
            Action::MoveRight => {
                if self.mode.is_normal() {
                    self.cursor.move_right_normal(&self.buffer);
                } else {
                    self.cursor.move_right(&self.buffer);
                }
            }
            Action::MoveUp => {
                if self.mode.is_normal() {
                    self.cursor.move_up_normal(&self.buffer);
                } else {
                    self.cursor.move_up(&self.buffer);
                }
            }
            Action::MoveDown => {
                if self.mode.is_normal() {
                    self.cursor.move_down_normal(&self.buffer);
                } else {
                    self.cursor.move_down(&self.buffer);
                }
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
            Action::EnterInsertMode => self.mode = Mode::Insert,
            Action::InsertAfterCursor => self.insert_after_cursor(),
            Action::OpenLineBelow => self.open_line_below(),
            Action::OpenLineAbove => self.open_line_above(),
            Action::EnterCommandMode => self.mode = Mode::command_empty(),
            Action::EnterSearchMode => self.mode = Mode::search_empty(),
            Action::ExitToNormalMode => self.mode = Mode::Normal,
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
        if self.mode.is_normal() {
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
        if !self.mode.is_normal() {
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
            self.cursor.clamp_to_line_normal(&self.buffer);
            self.viewport
                .ensure_cursor_visible(&self.cursor, &self.buffer);
        }

        // While waiting for find target, consume all keys to avoid accidental mode switches.
        true
    }

    /// Consume one key while a multi-key normal-mode sequence is pending.
    ///
    /// Returns `true` when this function consumed the key.
    fn handle_pending_sequence_key(&mut self, key: Key) -> bool {
        if !self.mode.is_normal() || self.pending_sequence.is_empty() {
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
        if !self.mode.is_normal()
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
        self.buffer.insert(char_idx, &c.to_string());
        self.cursor.move_right(&self.buffer);
    }

    fn insert_newline(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.buffer.insert(char_idx, "\n");
        self.cursor.move_down(&self.buffer);
        self.cursor.set_column(0);
    }

    fn open_line_below(&mut self) {
        let line = self.cursor.line();
        let line_end = self.buffer.line_to_char(line) + self.buffer.line_len(line);
        self.buffer.insert(line_end, "\n");
        self.cursor = Cursor::new(line + 1, 0);
        self.mode = Mode::Insert;
    }

    fn insert_after_cursor(&mut self) {
        let line_len = self.buffer.line_len(self.cursor.line());
        if line_len > 0 {
            self.cursor.move_right(&self.buffer);
        }
        self.mode = Mode::Insert;
    }

    fn open_line_above(&mut self) {
        let line = self.cursor.line();
        let line_start = self.buffer.line_to_char(line);
        self.buffer.insert(line_start, "\n");
        self.cursor = Cursor::new(line, 0);
        self.mode = Mode::Insert;
    }

    fn delete_char_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx > 0 {
            self.cursor.move_left(&self.buffer);
            self.buffer.remove(char_idx - 1, char_idx);
        }
    }

    fn delete_char_forward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx < self.buffer.chars_count() {
            self.buffer.remove(char_idx, char_idx + 1);
        }
    }

    fn delete_char_at_cursor(&mut self) {
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let line_len = self.buffer.line_len(self.cursor.line());
        if line_len > 0 {
            self.buffer.remove(char_idx, char_idx + 1);
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
        self.buffer.remove(char_idx, end);
    }

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
        self.buffer.remove(word_start, char_idx);
    }

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
        self.buffer.remove(line_start, char_idx);
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

        self.buffer.remove(start, end);
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
            self.mode = Mode::Insert;
        }
    }

    /// Repeat `ciw` deletions up to `count` times, then enter insert if anything changed.
    fn change_inner_word_count(&mut self, count: usize) {
        let before_total = self.buffer.chars_count();
        self.delete_inner_word_count(count);
        if self.buffer.chars_count() < before_total {
            self.mode = Mode::Insert;
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

        self.buffer.remove(start, end);
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
        match &self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command(_) => "COMMAND",
            Mode::Search(_) => "SEARCH",
        }
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
        if !self.mode.is_normal() {
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
                match key {
                    KeyInput::Char(c) => label.push(*c),
                    KeyInput::Ctrl(c) => label.push_str(&format!("^{}", c)),
                    KeyInput::Alt(c) => label.push_str(&format!("M-{}", c)),
                    KeyInput::Backspace => label.push_str("BS"),
                    KeyInput::Escape => label.push_str("Esc"),
                    KeyInput::BackTab => label.push_str("S-Tab"),
                    KeyInput::Up => label.push_str("Up"),
                    KeyInput::Down => label.push_str("Down"),
                    KeyInput::Left => label.push_str("Left"),
                    KeyInput::Right => label.push_str("Right"),
                    KeyInput::ShiftUp => label.push_str("S-Up"),
                    KeyInput::ShiftDown => label.push_str("S-Down"),
                    KeyInput::ShiftLeft => label.push_str("S-Left"),
                    KeyInput::ShiftRight => label.push_str("S-Right"),
                    KeyInput::AltUp => label.push_str("M-Up"),
                    KeyInput::AltDown => label.push_str("M-Down"),
                    KeyInput::AltLeft => label.push_str("M-Left"),
                    KeyInput::AltRight => label.push_str("M-Right"),
                    KeyInput::CtrlUp => label.push_str("C-Up"),
                    KeyInput::CtrlDown => label.push_str("C-Down"),
                    KeyInput::CtrlLeft => label.push_str("C-Left"),
                    KeyInput::CtrlRight => label.push_str("C-Right"),
                    KeyInput::Home => label.push_str("Home"),
                    KeyInput::CtrlHome => label.push_str("C-Home"),
                    KeyInput::End => label.push_str("End"),
                    KeyInput::CtrlEnd => label.push_str("C-End"),
                    KeyInput::PageUp => label.push_str("PgUp"),
                    KeyInput::PageDown => label.push_str("PgDn"),
                    KeyInput::Delete => label.push_str("Del"),
                    KeyInput::Insert => label.push_str("Ins"),
                    KeyInput::F(n) => label.push_str(&format!("F{}", n)),
                    KeyInput::Unsupported => label.push('?'),
                }
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
