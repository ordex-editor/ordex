//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::completion::{
    CompletionDirection, CompletionSession, CompletionSourceRegistry, build_request,
    refresh_session,
};
use crate::config::ConfigSettings;
use crate::cursor::Cursor;
use crate::dialogs::{
    BufferSwitchItem, BufferSwitchState, DEFAULT_FILE_PICKER_MAX_FILES, DefinitionPickerItem,
    DefinitionPickerState, FilePickerPollResult, FilePickerState,
};
use crate::keybindings::{Action, ActionBinding, KeyBindings, KeyInput, SequenceMatch};
use crate::lsp::{DefinitionLookupOutcome, DefinitionLookupResult, DefinitionTarget};
use crate::mode::{Mode, VisualKind};
use crate::navigation::{
    find_next_paragraph_line, find_next_word_start, find_prev_paragraph_line, find_prev_word_start,
    find_word_end,
};
use crate::session::{ProjectSession, SessionBuffer, normalize_session_buffer_path};
use crate::soft_wrap;
use crate::swap::{self, SwapHandle};
use crate::syntax::{BufferEdit, HighlightSpan, SyntaxClass, SyntaxEngine};
use crate::text_buffer::TextBuffer;
use crate::themes;
use crate::tui;
use crate::viewport::Viewport;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use termion::event::Key;

mod actions;
mod buffers;
mod commands;
mod history;
mod matching;
mod operator;
mod repeat;
mod view;

pub(crate) use buffers::BufferSummary;
use buffers::{
    BufferManager, BufferState, OrderedBufferState, display_file_name, normalize_lookup_path,
    paths_match,
};
pub(crate) use matching::VisibleMatchRole;
use operator::{ExecutedOperatorCommand, OperatorKind, PendingOperator};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    BufferSwitch,
    FilePicker,
    DefinitionPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveDefinitionLookup {
    token: u64,
    document_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOverwrite {
    target_path: PathBuf,
    update_file_path: bool,
    after_write_action: AfterWriteAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingQuitConfirmation {
    remaining_buffer_ids: VecDeque<usize>,
}

/// Dirty-buffer confirmation state for `:open-session`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSessionOpenConfirmation {
    session_name: String,
    remaining_buffer_ids: VecDeque<usize>,
}

/// Recovery prompt state for one buffer whose previous swap file still exists.
#[derive(Debug)]
struct PendingSwapRecovery {
    /// Buffer content loaded from the existing swap file.
    recovered_buffer: TextBuffer,
    /// Whether discarding recovery should immediately recreate a fresh swap file.
    recreate_handle_on_discard: bool,
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

/// One stored edit offset used when replaying an insert-style change.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RelativeHistoryEdit {
    /// Text inserted at a signed offset from the insert-session start position.
    Insert {
        char_idx_offset: isize,
        text: String,
    },
    /// Text removed at a signed offset from the insert-session start position.
    Remove {
        char_idx_offset: isize,
        text: String,
    },
}

/// One repeatable change recorded for Normal-mode `.` replay.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RepeatableChange {
    /// One change that can be replayed by re-running the same source command.
    Direct(RepeatSource),
    /// One insert-style session replayed by re-running setup and post-setup edits.
    InsertSession {
        /// Source command that recreates the setup phase before relative insert edits replay.
        source: RepeatSource,
        edits: Vec<RelativeHistoryEdit>,
        final_cursor_offset: isize,
    },
}

/// One replayable source command stored for `.` direct or insert-style repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RepeatSource {
    /// Replay one ordinary binding by executing it again with the stored count.
    Binding {
        binding: ActionBinding,
        count: Option<usize>,
    },
    /// Replay one resolved operator command such as `dw` or `ct,`.
    Operator(ExecutedOperatorCommand),
}

/// Pending metadata for one insert-style change that may become repeatable on exit.
///
/// This capture stays alive only while Ordex is still inside the insert session
/// entered from a Normal-mode binding. Once Escape commits the undo transaction,
/// the capture tells repeat replay where setup ended, where insert input began,
/// and which source command should be re-run before applying the recorded relative
/// insert-session edits.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveInsertRepeatCapture {
    /// Replay source that entered Insert mode from Normal mode.
    source: RepeatSource,
    /// Number of setup edits already present before user-driven insert edits begin.
    history_edit_start: usize,
    /// Cursor position at the moment Insert mode became active.
    session_start_char_idx: usize,
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
enum AfterWriteAction {
    StayOpen,
    Quit,
    ContinueQuitSequence(VecDeque<usize>),
    /// Resume a pending session-open flow after saving the currently shown buffer.
    ContinueSessionOpenSequence {
        session_name: String,
        remaining_buffer_ids: VecDeque<usize>,
    },
    CloseActiveBuffer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredWrite {
    pub(crate) path: PathBuf,
    pub(crate) update_file_path: bool,
    after_write_action: AfterWriteAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditorRequest {
    ReloadConfig,
    WriteBuffer(DeferredWrite),
    SaveSession(String),
    OpenSession(String),
    DeleteSession(String),
    GotoDefinition,
}

/// Runtime editor settings that have built-in defaults and may be overridden by config.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSettings {
    scroll_margin: usize,
    horizontal_scroll_margin: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    file_picker_max_files: usize,
    sequence_discovery_popup: bool,
    theme_name: &'static str,
    color_capability: themes::ColorCapability,
    swap_exclude_patterns: Vec<String>,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            scroll_margin: Viewport::DEFAULT_SCROLL_MARGIN,
            horizontal_scroll_margin: Viewport::DEFAULT_HORIZONTAL_SCROLL_MARGIN,
            relative_line_numbers: false,
            soft_wrap: true,
            file_picker_max_files: DEFAULT_FILE_PICKER_MAX_FILES,
            sequence_discovery_popup: true,
            theme_name: themes::DEFAULT_THEME_NAME,
            color_capability: themes::ColorCapability::Ansi256,
            swap_exclude_patterns: Vec::new(),
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
    /// Stable identifier of the active buffer.
    active_buffer_id: usize,
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
    /// Inactive buffers plus navigation order for all open buffers.
    buffer_manager: BufferManager,
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
    /// Pending delete/change/yank operator waiting for a motion or text object.
    pending_operator: Option<PendingOperator>,
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
    /// Pending confirmation for replacing dirty buffers while opening a session.
    pending_session_open_confirmation: Option<PendingSessionOpenConfirmation>,
    /// Pending recovery choice for an existing swap file.
    pending_swap_recovery: Option<PendingSwapRecovery>,
    /// Pending close confirmation for `:bd` with unsaved changes.
    pending_buffer_close_confirmation: bool,
    /// Active buffer-switch picker state while the overlay is open.
    buffer_switch: Option<BufferSwitchState>,
    /// Active file-picker state while the overlay is open.
    file_picker: Option<FilePickerState>,
    /// Active definition-target picker state while the overlay is open.
    definition_picker: Option<DefinitionPickerState>,
    /// Registered completion sources available to the insert-mode popup flow.
    completion_sources: CompletionSourceRegistry,
    /// Monotonic generation used to discard stale completion refreshes.
    completion_generation: usize,
    /// Active inline completion session for Insert mode, if any.
    completion_session: Option<CompletionSession>,
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
    /// Monotonic token source used to reject stale definition results.
    next_definition_lookup_token: u64,
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
    /// Swap file handle associated with the active buffer.
    swap: Option<SwapHandle>,
    /// Deadline for the next debounced swap refresh after an edit.
    pending_swap_refresh_at: Option<Instant>,
    /// Last repeatable change used by Normal-mode `.` replay.
    last_repeatable_change: Option<RepeatableChange>,
    /// Pending insert-style capture being assembled until Insert mode finishes.
    active_insert_repeat: Option<ActiveInsertRepeatCapture>,
    /// Suppress repeat capture while replaying a stored `.` change.
    replaying_repeat: bool,
    /// Monotonic document version sent to the language server for the active buffer.
    lsp_document_version: i32,
    /// Last active definition lookup request for the active buffer, if any.
    active_definition_lookup: Option<ActiveDefinitionLookup>,
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
    const RESERVED_TOP_ROWS: usize = 1;
    const RESERVED_BOTTOM_ROWS: usize = 2;
    const RESERVED_SCREEN_ROWS: usize = Self::RESERVED_TOP_ROWS + Self::RESERVED_BOTTOM_ROWS;
    /// Delay after the most recent edit before the debounced swap refresh runs.
    const SWAP_REFRESH_DELAY: Duration = Duration::from_millis(300);

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
            active_buffer_id: 0,
            cursor: Cursor::new(0, 0),
            mode: Mode::Normal,
            visual_anchor: None,
            viewport: Viewport::new(terminal_height.saturating_sub(Self::RESERVED_SCREEN_ROWS)),
            file_path: PathBuf::new(),
            syntax: SyntaxEngine::new(),
            buffer_manager: BufferManager::new(0),
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
            pending_operator: None,
            pending_find: None,
            last_find: None,
            last_visual_selection: None,
            yank_buffer: None,
            pending_overwrite: None,
            pending_quit_confirmation: None,
            pending_session_open_confirmation: None,
            pending_swap_recovery: None,
            pending_buffer_close_confirmation: false,
            buffer_switch: None,
            file_picker: None,
            definition_picker: None,
            completion_sources: CompletionSourceRegistry::new(),
            completion_generation: 0,
            completion_session: None,
            matching: matching::MatchingState::new(),
            ignore_input_escape_cancel_until: None,
            pending_request: None,
            next_definition_lookup_token: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            active_undo: None,
            saved_undo_depth: 0,
            replaying_history: false,
            swap: None,
            pending_swap_refresh_at: None,
            last_repeatable_change: None,
            active_insert_repeat: None,
            replaying_repeat: false,
            lsp_document_version: 0,
            active_definition_lookup: None,
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

        if let Some(limit) = settings.file_picker_max_files {
            self.settings.file_picker_max_files = limit.max(1);
        }

        if let Some(enabled) = settings.sequence_discovery_popup {
            self.settings.sequence_discovery_popup = enabled;
        }

        if let Some(theme_name) = settings.theme.as_deref()
            && let Some(theme) = themes::find(theme_name)
        {
            self.settings.theme_name = theme.name;
        }

        if let Some(patterns) = settings.swap_exclude_patterns.as_ref() {
            self.settings.swap_exclude_patterns = patterns.clone();
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
        for binding in &settings.operator_bindings {
            self.keybindings
                .set_operator_binding(binding.key.clone(), binding.binding);
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
        self.buffer_manager.apply_shared_view_settings(
            self.viewport.height(),
            self.settings.scroll_margin,
            self.settings.horizontal_scroll_margin,
            self.settings.soft_wrap,
        );
    }

    /// Load a file into the editor using chunked reading for efficiency
    pub(crate) fn load_file(&mut self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        let file = File::open(path)?;
        self.buffer = TextBuffer::from_reader(file)?;
        self.file_path = path.to_path_buf();
        self.cursor = Cursor::new(0, 0);
        self.desired_visual_column = None;
        self.viewport.set_first_visible_line(0);
        self.refresh_syntax();
        self.reset_history();
        self.load_swap_state_for_active_buffer();
        Ok(())
    }

    /// Open one additional buffer from `path` and make it active.
    pub(crate) fn open_buffer(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if paths_match(&self.file_path, path) {
            return Ok(());
        }

        if let Some(buffer) = self.buffer_manager.take_inactive_by_path(path) {
            self.activate_inactive_buffer(buffer);
            return Ok(());
        }

        let buffer_id = self.buffer_manager.allocate_id();
        let buffer = if path.exists() {
            BufferState::from_file(
                buffer_id,
                self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
                path,
            )?
        } else {
            BufferState::new_named_empty(
                buffer_id,
                self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
                path,
            )
        };
        self.buffer_manager.push_new_id(buffer_id);
        self.activate_inactive_buffer(buffer);
        self.load_swap_state_for_active_buffer();
        Ok(())
    }

    /// Open one additional unnamed empty buffer and make it active.
    pub(crate) fn open_empty_buffer(&mut self) {
        let buffer_id = self.buffer_manager.allocate_id();
        let buffer = BufferState::new_empty(
            buffer_id,
            self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
        );
        self.buffer_manager.push_new_id(buffer_id);
        self.activate_inactive_buffer(buffer);
    }

    /// Load recovery state for the startup buffer when no file argument was provided.
    pub(crate) fn load_startup_swap_state(&mut self) {
        self.load_swap_state_for_active_buffer();
    }

    /// Replace the active buffer path for startup of a missing file.
    pub(crate) fn set_startup_path(&mut self, path: impl AsRef<Path>) {
        self.file_path = path.as_ref().to_path_buf();
        self.refresh_syntax();
        self.load_swap_state_for_active_buffer();
    }

    /// Open additional startup buffers after the first initial buffer.
    pub(crate) fn open_startup_buffer(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        self.open_buffer(path)
    }

    /// Build a serializable snapshot of the current project session.
    pub(crate) fn build_project_session(&self, working_directory: PathBuf) -> ProjectSession {
        let ordered_buffers = self.ordered_project_buffers();
        let active_buffer = ordered_buffers
            .iter()
            .position(|buffer| buffer.active)
            .unwrap_or(0);
        let buffers = ordered_buffers
            .into_iter()
            .map(|buffer| SessionBuffer {
                path: normalize_session_buffer_path(&buffer.file_path, &working_directory),
                cursor: buffer.cursor,
            })
            .collect();
        ProjectSession {
            working_directory,
            active_buffer,
            buffers,
        }
    }

    /// Replace open buffers with the buffers stored in one project session.
    pub(crate) fn restore_project_session(&mut self, session: &ProjectSession) -> io::Result<()> {
        let terminal_height = self.viewport.height() + Self::RESERVED_SCREEN_ROWS;
        let settings = self.settings.clone();
        let mut restored = Self::new(terminal_height);
        restored.settings = settings;
        restored.keybindings = std::mem::take(&mut self.keybindings);
        restored.apply_runtime_settings();

        // Build the replacement editor off to the side so failed file opens do
        // not partially rewrite the current in-memory session.
        if let Err(error) = restored.restore_project_session_buffers(session) {
            self.keybindings = std::mem::take(&mut restored.keybindings);
            return Err(error);
        }

        self.cleanup_all_swap_files();
        *self = restored;
        Ok(())
    }

    /// Return the stable identifier of the currently active buffer.
    pub(crate) fn active_buffer_id(&self) -> usize {
        self.active_buffer_id
    }

    /// Activate one previously opened startup buffer by identifier.
    pub(crate) fn activate_buffer(&mut self, buffer_id: usize) {
        self.switch_to_buffer_id(buffer_id);
    }

    /// Swap the active buffer-local fields with `state` and return the previous active buffer.
    fn replace_active_buffer_state(&mut self, state: BufferState) -> BufferState {
        let BufferState {
            id,
            buffer,
            cursor,
            viewport,
            file_path,
            syntax,
            desired_visual_column,
            matching,
            undo_stack,
            redo_stack,
            active_undo,
            saved_undo_depth,
            replaying_history,
            swap,
            pending_swap_refresh_at,
            lsp_document_version,
            active_definition_lookup,
        } = state;
        let previous = BufferState {
            id: std::mem::replace(&mut self.active_buffer_id, id),
            buffer: std::mem::replace(&mut self.buffer, buffer),
            cursor: std::mem::replace(&mut self.cursor, cursor),
            viewport: std::mem::replace(&mut self.viewport, viewport),
            file_path: std::mem::replace(&mut self.file_path, file_path),
            syntax: std::mem::replace(&mut self.syntax, syntax),
            desired_visual_column: std::mem::replace(
                &mut self.desired_visual_column,
                desired_visual_column,
            ),
            matching: std::mem::replace(&mut self.matching, matching),
            undo_stack: std::mem::replace(&mut self.undo_stack, undo_stack),
            redo_stack: std::mem::replace(&mut self.redo_stack, redo_stack),
            active_undo: std::mem::replace(&mut self.active_undo, active_undo),
            saved_undo_depth: std::mem::replace(&mut self.saved_undo_depth, saved_undo_depth),
            replaying_history: std::mem::replace(&mut self.replaying_history, replaying_history),
            swap: std::mem::replace(&mut self.swap, swap),
            pending_swap_refresh_at: std::mem::replace(
                &mut self.pending_swap_refresh_at,
                pending_swap_refresh_at,
            ),
            lsp_document_version: std::mem::replace(
                &mut self.lsp_document_version,
                lsp_document_version,
            ),
            active_definition_lookup: std::mem::replace(
                &mut self.active_definition_lookup,
                active_definition_lookup,
            ),
        };
        self.viewport.set_scroll_margin(self.settings.scroll_margin);
        self.viewport.set_soft_wrap(self.settings.soft_wrap);
        self.viewport
            .set_horizontal_scroll_margin(self.settings.horizontal_scroll_margin);
        previous
    }

    /// Park the current active buffer and activate one inactive buffer in its place.
    fn activate_inactive_buffer(&mut self, target: BufferState) {
        let previous = self.replace_active_buffer_state(target);
        self.buffer_manager.store_inactive(previous);
        self.reset_mode_for_buffer_switch();
    }

    /// Switch to the next buffer in order, wrapping at the end.
    fn switch_to_next_buffer(&mut self) {
        let next_id = self.buffer_manager.next_buffer_id(self.active_buffer_id);
        self.switch_to_buffer_id(next_id);
    }

    /// Switch to the previous buffer in order, wrapping at the front.
    fn switch_to_prev_buffer(&mut self) {
        let prev_id = self.buffer_manager.prev_buffer_id(self.active_buffer_id);
        self.switch_to_buffer_id(prev_id);
    }

    /// Switch to one specific buffer identified by its stable id.
    fn switch_to_buffer_id(&mut self, buffer_id: usize) {
        if buffer_id == self.active_buffer_id {
            return;
        }

        if let Some(target) = self.buffer_manager.take_inactive_by_id(buffer_id) {
            self.activate_inactive_buffer(target);
        }
    }

    /// Reset transient editor-global state after changing the active buffer.
    fn reset_mode_for_buffer_switch(&mut self) {
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.dismiss_completion_session(false);
        self.clear_pending_modal_state();
        self.pending_overwrite = None;
        self.pending_quit_confirmation = None;
        self.pending_session_open_confirmation = None;
        self.pending_swap_recovery = None;
        self.pending_buffer_close_confirmation = false;
        self.buffer_switch = None;
        self.file_picker = None;
        self.definition_picker = None;
        self.status_message = None;
    }

    /// Return dirty buffer ids in list order, starting with the active buffer when dirty.
    fn dirty_buffer_ids(&self) -> Vec<usize> {
        let mut dirty_ids = self.buffer_manager.inactive_dirty_ids();
        if self.buffer.is_modified() {
            dirty_ids.insert(0, self.active_buffer_id);
        }
        dirty_ids
    }

    /// Return ordered project-buffer snapshots for session persistence.
    fn ordered_project_buffers(&self) -> Vec<OrderedBufferState> {
        self.buffer_manager
            .ordered_states(self.active_buffer_id, &self.file_path, &self.cursor)
    }

    /// Restore the current editor from one session snapshot.
    fn restore_project_session_buffers(&mut self, session: &ProjectSession) -> io::Result<()> {
        if session.buffers.is_empty() {
            return Ok(());
        }

        let mut buffer_ids = Vec::new();
        // Open each saved buffer in order so the buffer manager keeps the same
        // navigation sequence when the session is reopened later.
        for (index, buffer) in session.buffers.iter().enumerate() {
            self.restore_project_session_buffer(buffer, index == 0)?;
            buffer_ids.push(self.active_buffer_id);
        }

        if let Some(&active_id) = buffer_ids.get(session.active_buffer) {
            self.activate_buffer(active_id);
        }
        Ok(())
    }

    /// Restore one saved buffer entry into the active editor.
    fn restore_project_session_buffer(
        &mut self,
        buffer: &SessionBuffer,
        first_buffer: bool,
    ) -> io::Result<()> {
        if first_buffer {
            self.restore_first_project_session_buffer(buffer)?;
        } else {
            self.restore_additional_project_session_buffer(buffer)?;
        }
        self.restore_active_project_session_cursor(&buffer.cursor);
        Ok(())
    }

    /// Restore the first saved buffer into the editor's initial active slot.
    fn restore_first_project_session_buffer(&mut self, buffer: &SessionBuffer) -> io::Result<()> {
        if buffer.path.as_os_str().is_empty() {
            return Ok(());
        }

        if buffer.path.exists() {
            self.load_file(&buffer.path)?;
        } else {
            self.set_startup_path(&buffer.path);
        }
        Ok(())
    }

    /// Restore one additional saved buffer after the first entry.
    fn restore_additional_project_session_buffer(
        &mut self,
        buffer: &SessionBuffer,
    ) -> io::Result<()> {
        if buffer.path.as_os_str().is_empty() {
            // Session restore must preserve unnamed buffers as distinct entries in
            // the buffer list instead of collapsing them into the current buffer.
            self.open_empty_buffer();
            return Ok(());
        }

        self.open_startup_buffer(&buffer.path)
    }

    /// Clamp the active cursor to the current buffer after session restore.
    fn restore_active_project_session_cursor(&mut self, cursor: &Cursor) {
        let max_line = self.buffer.lines_count().saturating_sub(1);
        let line = cursor.line().min(max_line);
        let column = cursor.column().min(self.buffer.line_len(line));
        self.cursor = Cursor::new(line, column);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Return one single-line listing of every open buffer.
    fn format_buffer_list(&self) -> String {
        self.buffer_manager
            .summaries(
                self.active_buffer_id,
                self.file_name(),
                &self.file_path,
                self.buffer.is_modified(),
            )
            .into_iter()
            .map(|buffer| {
                let current = if buffer.active { "%" } else { " " };
                let modified = if buffer.modified { "+" } else { " " };
                format!("{current}{modified} {}", buffer.file_name)
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Return the picker dialog that currently owns the modal input stream.
    fn active_picker_kind(&self) -> Option<PickerKind> {
        match self.mode {
            Mode::BufferSwitch(_) => Some(PickerKind::BufferSwitch),
            Mode::FilePicker(_) => Some(PickerKind::FilePicker),
            Mode::DefinitionPicker(_) => Some(PickerKind::DefinitionPicker),
            _ => None,
        }
    }

    /// Open the buffer-switch picker with the current ordered buffer list.
    fn open_buffer_switcher(&mut self) {
        let items = self
            .buffer_manager
            .summaries(
                self.active_buffer_id,
                self.file_name(),
                &self.file_path,
                self.buffer.is_modified(),
            )
            .into_iter()
            .enumerate()
            .map(|(index, summary)| (usize::from(!summary.active), index, summary))
            .collect::<Vec<_>>();
        let mut items = items;
        // Keep the active buffer visible as contextual row zero while preserving
        // the existing order of every other buffer behind it.
        items.sort_by_key(|(active_rank, index, _)| (*active_rank, *index));
        let items = items
            .into_iter()
            .enumerate()
            .map(|(order, (_, _, summary))| BufferSwitchItem {
                buffer_id: summary.id,
                label: summary.display_path,
                active: summary.active,
                modified: summary.modified,
                order,
            })
            .collect();

        self.prepare_picker_open();
        self.buffer_switch = Some(BufferSwitchState::new(items));
        self.mode = Mode::buffer_switch_empty();
    }

    /// Close the buffer-switch picker without changing the active buffer.
    fn close_buffer_switcher(&mut self) {
        self.buffer_switch = None;
        self.mode = Mode::Normal;
    }

    /// Confirm the current picker selection, if one is available.
    fn confirm_buffer_switcher_selection(&mut self) {
        let Some(buffer_id) = self
            .buffer_switch
            .as_ref()
            .and_then(BufferSwitchState::selected_buffer_id)
        else {
            return;
        };
        if buffer_id == self.active_buffer_id {
            return;
        }

        self.close_buffer_switcher();
        self.switch_to_buffer_id(buffer_id);
    }

    /// Open the file picker rooted at the current working directory.
    fn open_file_picker(&mut self) {
        let root = match std::env::current_dir() {
            Ok(root) => root,
            Err(error) => {
                self.show_status_message(format!("Failed to read working directory: {error}"));
                return;
            }
        };

        self.prepare_picker_open();
        self.file_picker = Some(FilePickerState::new(
            root,
            self.settings.file_picker_max_files,
        ));
        self.mode = Mode::file_picker_empty();
    }

    /// Close the file picker without opening a selection.
    fn close_file_picker(&mut self) {
        if let Some(picker) = &mut self.file_picker {
            picker.cancel();
        }
        self.file_picker = None;
        self.mode = Mode::Normal;
    }

    /// Open the definition picker for one multi-target lookup result.
    fn open_definition_picker(&mut self, targets: Vec<DefinitionTarget>) {
        // Preserve server order so repeated queries stay stable while the user filters.
        let items = targets
            .into_iter()
            .enumerate()
            .map(|(order, target)| DefinitionPickerItem { target, order })
            .collect();
        self.prepare_picker_open();
        self.definition_picker = Some(DefinitionPickerState::new(items));
        self.mode = Mode::definition_picker_empty();
    }

    /// Close the definition picker without applying a selection.
    fn close_definition_picker(&mut self) {
        self.definition_picker = None;
        self.mode = Mode::Normal;
    }

    /// Confirm the current definition-picker selection, if one is available.
    fn confirm_definition_picker_selection(&mut self) {
        let Some(target) = self
            .definition_picker
            .as_ref()
            .and_then(DefinitionPickerState::selected_target)
        else {
            return;
        };
        let target = target.clone();

        self.close_definition_picker();
        self.goto_definition_target(&target);
    }

    /// Confirm the current file-picker selection, if one is available.
    fn confirm_file_picker_selection(&mut self) {
        let Some(path) = self
            .file_picker
            .as_ref()
            .and_then(FilePickerState::selected_path)
            .map(str::to_string)
        else {
            return;
        };

        self.close_file_picker();
        if let Err(error) = self.open_buffer(&path) {
            self.show_status_message(format!("Failed to open \"{path}\": {error}"));
        }
    }

    /// Poll background picker work and return whether visible state changed.
    ///
    /// Returns `true` when picker polling or a due swap refresh produced a UI
    /// change, and `false` when nothing visible changed on this poll tick.
    pub(crate) fn poll_background_tasks(&mut self) -> bool {
        let mut changed = false;

        if let Some(query) = self.mode.file_picker_string().map(str::to_string)
            && let Some(picker) = &mut self.file_picker
        {
            let FilePickerPollResult {
                changed: picker_changed,
                status_message,
            } = picker.poll(&query);
            if let Some(status_message) = status_message {
                self.show_status_message(status_message);
                changed = true;
            }
            changed |= picker_changed;
        }

        changed | self.flush_due_swap_refresh()
    }

    /// Clear transient modal UI so a newly-opened picker owns the overlay state.
    fn prepare_picker_open(&mut self) {
        self.dismiss_completion_session(false);
        self.clear_pending_modal_state();
        self.pending_overwrite = None;
        self.pending_quit_confirmation = None;
        self.pending_swap_recovery = None;
        self.pending_buffer_close_confirmation = false;
        self.status_message = None;
        self.buffer_switch = None;
        if let Some(picker) = &mut self.file_picker {
            picker.cancel();
        }
        self.file_picker = None;
        self.definition_picker = None;
    }

    /// Return whether the app loop should poll for asynchronous picker updates.
    ///
    /// Returns `true` when file-picker work or a pending swap flush needs a timed
    /// wakeup, and `false` when the editor can stay on the blocking input path.
    pub(crate) fn needs_background_poll(&self) -> bool {
        self.file_picker
            .as_ref()
            .is_some_and(FilePickerState::is_scanning)
            || self.pending_swap_refresh_at.is_some()
    }

    /// Queue a go-to-definition lookup for the current cursor position.
    fn request_goto_definition(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            self.show_status_message("No file is open for go-to-definition");
            return;
        }
        let document_version = self.lsp_document_version.saturating_add(1);
        let token = self.next_definition_lookup_token;
        self.next_definition_lookup_token = self.next_definition_lookup_token.saturating_add(1);
        self.lsp_document_version = document_version;
        self.active_definition_lookup = Some(ActiveDefinitionLookup {
            token,
            document_version,
        });
        self.pending_request = Some(EditorRequest::GotoDefinition);
        self.show_status_message("Resolving definition...");
    }

    /// Build the active-buffer snapshot required for one background definition lookup.
    pub(crate) fn definition_request_snapshot(
        &self,
    ) -> Option<crate::lsp::manager::DefinitionRequestSnapshot> {
        let lookup = self.active_definition_lookup?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        // Clone the rope so the worker thread keeps an immutable snapshot without
        // forcing one eager `String` allocation for every queued lookup.
        Some(crate::lsp::manager::DefinitionRequestSnapshot {
            buffer_id: self.active_buffer_id,
            lookup_token: lookup.token,
            document_version: lookup.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            line: self.cursor.line(),
            character: self.cursor.column(),
        })
    }

    /// Apply one completed definition lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result was accepted and changed editor-visible
    /// state, and `false` when it was stale or no longer mapped to an open buffer.
    pub(crate) fn apply_definition_lookup_result(
        &mut self,
        result: DefinitionLookupResult,
    ) -> bool {
        // Results are keyed by the originating buffer id, so switch back to that
        // buffer if it is still open before checking whether the lookup is stale.
        if self.active_buffer_id != result.buffer_id {
            self.switch_to_buffer_id(result.buffer_id);
        }
        if self.active_buffer_id != result.buffer_id {
            return false;
        }
        let Some(lookup) = self.active_definition_lookup else {
            return false;
        };
        if lookup.token != result.lookup_token || lookup.document_version != result.document_version
        {
            return false;
        }
        self.active_definition_lookup = None;
        match result.outcome {
            DefinitionLookupOutcome::Single(target) => self.goto_definition_target(&target),
            // Multiple locations need an explicit user choice before any jump happens.
            DefinitionLookupOutcome::Multiple(targets) => self.open_definition_picker(targets),
            DefinitionLookupOutcome::NotFound => self.show_status_message("No definition found"),
            DefinitionLookupOutcome::UnsupportedFile(message)
            | DefinitionLookupOutcome::UnsupportedProject(message)
            | DefinitionLookupOutcome::Unavailable(message)
            | DefinitionLookupOutcome::Error(message) => self.show_status_message(message),
        }
        true
    }

    /// Return a definition target path relative to the current directory when possible.
    fn goto_definition_open_path<'a>(&self, path: &'a Path) -> &'a Path {
        if path.is_absolute()
            && let Ok(current_directory) = std::env::current_dir()
            && let Ok(relative) = path.strip_prefix(&current_directory)
        {
            return relative;
        }
        path
    }

    /// Open one definition target and move the cursor to the returned position.
    fn goto_definition_target(&mut self, target: &DefinitionTarget) {
        let open_path = self.goto_definition_open_path(&target.file_path);
        // Open the returned file first so every follow-up cursor calculation uses
        // the destination buffer rather than stale line counts from the source file.
        if let Err(error) = self.open_buffer(open_path) {
            self.show_status_message(format!(
                "Failed to open definition target \"{}\": {error}",
                open_path.display()
            ));
            return;
        }
        // Clamp the reported position because rust-analyzer can target EOF or the
        // start of an empty line, both of which must remain valid cursor locations.
        let line = target.line.min(self.buffer.lines_count().saturating_sub(1));
        let max_column = self.buffer.line_len(line).saturating_sub(1);
        self.cursor = Cursor::new(line, target.character.min(max_column));
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Dismiss the active completion session, optionally restoring the typed prefix.
    fn dismiss_completion_session(&mut self, restore_prefix: bool) {
        let Some(session) = self.completion_session.take() else {
            return;
        };
        if !restore_prefix {
            return;
        }

        // Restoring the original prefix reuses the same buffer-edit path as previews.
        self.replace_completion_range(
            session.prefix_start_char_idx,
            session.replacement_end_char_idx(),
            session.original_prefix_text.as_str(),
        );
    }

    /// Replace the current completion span and leave the insert cursor at its end.
    fn replace_completion_range(&mut self, start_char_idx: usize, end_char_idx: usize, text: &str) {
        self.remove_buffer_range(start_char_idx, end_char_idx);
        self.insert_buffer_text(start_char_idx, text);
        self.cursor = Cursor::from_char_index(&self.buffer, start_char_idx + text.chars().count());
    }

    /// Advance the completion generation, resetting after `usize::MAX`.
    fn next_completion_generation(&mut self) -> usize {
        self.completion_generation = self.completion_generation.checked_add(1).unwrap_or(0);
        self.completion_generation
    }

    /// Dismiss completion whenever the editor is no longer in Insert mode.
    fn dismiss_completion_if_not_insert(&mut self) {
        if self.mode != Mode::Insert {
            self.dismiss_completion_session(false);
        }
    }

    /// Refresh the completion session from the current insert cursor context.
    fn refresh_completion_session(&mut self) {
        if self.mode != Mode::Insert {
            self.dismiss_completion_session(false);
            return;
        }

        let request_generation = self.next_completion_generation();
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        let Some(request) = build_request(
            &self.buffer,
            self.active_buffer_id,
            cursor_char_idx,
            request_generation,
        ) else {
            self.dismiss_completion_session(false);
            return;
        };
        // Keep the popup anchored to the location where this completion run began
        // so the suggestion box does not jitter rightward as the prefix grows.
        let popup_anchor_char_idx = self
            .completion_session
            .as_ref()
            .map_or(request.cursor_char_idx, |session| {
                session.popup_anchor_char_idx
            });

        if self
            .completion_session
            .as_ref()
            .is_some_and(|session| session.matches_request(&request))
        {
            // Reuse the current popup when the buffer, cursor, and prefix are unchanged.
            return;
        }

        self.completion_session = refresh_session(
            &self.completion_sources,
            &self.buffer,
            request,
            popup_anchor_char_idx,
        );
    }

    /// Move the completion selection if a session is active.
    ///
    /// Returns `true` when an active completion session consumed the movement,
    /// and `false` when no completion session was available to update.
    fn move_completion_selection(&mut self, direction: CompletionDirection) -> bool {
        let Some(mut session) = self.completion_session.take() else {
            return false;
        };
        let start_char_idx = session.prefix_start_char_idx;
        let end_char_idx = session.replacement_end_char_idx();
        session.move_selection(direction);
        let replacement = session.current_text().to_string();
        self.replace_completion_range(start_char_idx, end_char_idx, &replacement);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.completion_session = Some(session);
        true
    }

    /// Sync completion visibility after one action updates the editor state.
    fn sync_completion_after_action(&mut self, action: Action) {
        match action {
            Action::MoveLeft
            | Action::MoveRight
            | Action::MoveUp
            | Action::MoveDown
            | Action::MoveLineStart
            | Action::MovePastLineEnd
            | Action::DeleteCharBackward
            | Action::DeleteCharForward
            | Action::DeleteWordBackward
            | Action::DeleteToLineStart
            | Action::InsertNewline => self.refresh_completion_session(),
            Action::CompletionSelectUp | Action::CompletionSelectDown => {}
            Action::MoveWordForward
            | Action::MoveWordBackward
            | Action::MoveWordEnd
            | Action::MoveParagraphForward
            | Action::MoveParagraphBackward
            | Action::MoveLineEnd
            | Action::MoveFirstNonBlank
            | Action::MoveToFirstLine
            | Action::MoveToLastLine
            | Action::AlignViewportTop
            | Action::AlignViewportCenter
            | Action::AlignViewportBottom
            | Action::ScrollLineUp
            | Action::ScrollLineDown
            | Action::PageUp
            | Action::PageDown
            | Action::HalfPageUp
            | Action::HalfPageDown
            | Action::FindForward
            | Action::FindBackward
            | Action::TillForward
            | Action::TillBackward
            | Action::RepeatFindForward
            | Action::RepeatFindBackward
            | Action::RepeatLastChange
            | Action::MatchBracket
            | Action::EnterInsertMode
            | Action::EnterVisualMode
            | Action::EnterVisualLineMode
            | Action::SwapVisualAnchor
            | Action::RecreateLastSelection
            | Action::InsertAfterCursor
            | Action::OpenLineBelow
            | Action::OpenLineAbove
            | Action::EnterCommandMode
            | Action::EnterSearchMode
            | Action::OpenBufferSwitcher
            | Action::OpenFilePicker
            | Action::GotoDefinition
            | Action::ExitToNormalMode
            | Action::SearchNext
            | Action::SearchPrevious
            | Action::Undo
            | Action::Redo
            | Action::SaveCurrentFile
            | Action::SaveCurrentFileAndQuit
            | Action::UpdateCurrentFileAndQuit
            | Action::DeleteCharAtCursor
            | Action::DeleteSelection
            | Action::ChangeSelection
            | Action::YankSelection
            | Action::YankCurrentLine
            | Action::PasteAfterCursor
            | Action::PasteBeforeCursor
            | Action::BeginDeleteOperator
            | Action::BeginChangeOperator
            | Action::BeginYankOperator
            | Action::ExecuteCommand
            | Action::CancelCommand
            | Action::DeleteInputChar
            | Action::DeleteInputCharForward
            | Action::DeleteInputWordBackward
            | Action::DeleteInputToStart
            | Action::DeleteInputToEnd
            | Action::MoveInputStart
            | Action::MoveInputEnd
            | Action::MoveInputLeft
            | Action::MoveInputRight
            | Action::MoveInputWordLeft
            | Action::MoveInputWordRight => self.dismiss_completion_if_not_insert(),
        }
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
        self.sync_active_swap_after_buffer_change();
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
        self.sync_active_swap_after_buffer_change();
    }

    /// Return whether the active file path should skip fresh swap creation.
    ///
    /// Returns `true` when the active named file matches a configured exclusion
    /// pattern, and `false` when swap protection remains enabled for that path.
    fn active_path_is_swap_excluded(&self) -> bool {
        let Some(path) = normalize_lookup_path(&self.file_path) else {
            return false;
        };
        self.path_is_swap_excluded(&path)
    }

    /// Load swap recovery state for the active buffer without writing a new swap file.
    ///
    /// Startup only restores an existing recovery file here. Fresh swap files are
    /// created later, after the first buffer edit triggers the debounced refresh path.
    fn load_swap_state_for_active_buffer(&mut self) {
        self.swap = None;
        self.pending_swap_refresh_at = None;
        self.pending_swap_recovery = None;
        let active_path = normalize_lookup_path(&self.file_path);
        let is_excluded = active_path
            .as_ref()
            .is_some_and(|path| self.path_is_swap_excluded(path));
        let recovery = if let Some(path) = active_path.as_ref() {
            swap::load_recovery(path)
        } else if self.file_path.as_os_str().is_empty() {
            swap::load_unnamed_recovery()
        } else {
            return;
        };

        match recovery {
            Ok(Some(recovery)) => {
                self.swap = Some(recovery.handle);
                self.pending_swap_recovery = Some(PendingSwapRecovery {
                    recovered_buffer: recovery.buffer,
                    recreate_handle_on_discard: self.file_path.as_os_str().is_empty()
                        || !is_excluded,
                });
            }
            Ok(None) => {}
            Err(error) => {
                self.show_status_message(format!("Swap recovery unavailable: {error}"));
            }
        }
    }

    /// Create a fresh swap file handle for the active buffer path.
    fn create_active_swap_handle(&mut self) -> io::Result<()> {
        let handle = if let Some(path) = normalize_lookup_path(&self.file_path) {
            SwapHandle::create_from_buffer(&path, &self.buffer)?
        } else if self.file_path.as_os_str().is_empty() {
            SwapHandle::create_for_unnamed_buffer(&self.buffer)?
        } else {
            return Ok(());
        };
        self.swap = Some(handle);
        Ok(())
    }

    /// Schedule one debounced swap refresh after a buffer mutation.
    fn sync_active_swap_after_buffer_change(&mut self) {
        if !self.file_path.as_os_str().is_empty() && self.active_path_is_swap_excluded() {
            self.pending_swap_refresh_at = None;
            return;
        }
        // The debounce keeps swap writes off the hot typing path. A background
        // worker would need extra snapshot handoff, coalescing, and shutdown
        // coordination for the same small atomic write, so the app loop flushes
        // it synchronously only after the user pauses editing.
        self.pending_swap_refresh_at = Some(Instant::now() + Self::SWAP_REFRESH_DELAY);
    }

    /// Flush one due debounced swap refresh from the app-loop polling path.
    ///
    /// Returns `true` when the flush changed visible state by surfacing an error,
    /// and `false` when no swap work was due or the refresh completed quietly.
    fn flush_due_swap_refresh(&mut self) -> bool {
        let Some(deadline) = self.pending_swap_refresh_at else {
            return false;
        };
        if Instant::now() < deadline {
            return false;
        }
        self.pending_swap_refresh_at = None;
        self.flush_active_swap_refresh()
    }

    /// Flush one pending swap refresh immediately.
    pub(crate) fn flush_pending_swap_refresh(&mut self) {
        self.pending_swap_refresh_at = None;
        let _ = self.flush_active_swap_refresh();
    }

    /// Rewrite or recreate the active swap file from the current buffer contents.
    ///
    /// Returns `true` when a swap error produced a status message that needs a
    /// redraw, and `false` when the refresh completed without changing UI state.
    fn flush_active_swap_refresh(&mut self) -> bool {
        if let Some(swap) = self.swap.as_mut() {
            if let Err(error) = swap.refresh(&self.buffer) {
                self.show_swap_unavailable_error(&error);
                return true;
            }
            return false;
        }

        // Durable saves clear the old handle, so the first new edit recreates a
        // clean swap file from the current in-memory contents before more edits land.
        let created = self.create_active_swap_handle();
        if created.is_ok() {
            debug_assert!(self.swap.is_some());
        }
        if let Err(error) = created {
            self.show_swap_unavailable_error(&error);
            return true;
        }
        false
    }

    /// Return whether `path` matches one configured swap-exclusion pattern.
    ///
    /// Returns `true` when `path` should skip swap creation, and `false` when
    /// swap protection remains enabled for that absolute path.
    fn path_is_swap_excluded(&self, path: &Path) -> bool {
        path.to_str()
            .is_some_and(|path| swap::glob::matches_any(&self.settings.swap_exclude_patterns, path))
    }

    /// Show a consistent status message for swap-creation or refresh failures.
    fn show_swap_unavailable_error(&mut self, error: &io::Error) {
        self.show_status_message(format!(
            "Swap protection unavailable for {}: {error}",
            display_file_name(&self.file_path)
        ));
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
    ///
    /// Returns `true` for `\n` and `\r`, and `false` for every other character.
    fn is_line_break(ch: char) -> bool {
        matches!(ch, '\n' | '\r')
    }

    /// Return whether the provided text already ends with a line break.
    ///
    /// Returns `true` when the last character is `\n` or `\r`, and `false`
    /// when the text is empty or ends with any non-line-break character.
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

    /// Delete one prompt word forward while keeping the input cursor anchored.
    fn delete_input_word_forward(&mut self) {
        self.mode.delete_input_word_forward();
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
    use test_utils::{TempFile, TempTree};

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
                EditorRequest::SaveSession(_)
                | EditorRequest::OpenSession(_)
                | EditorRequest::DeleteSession(_) => {
                    panic!("unit tests should assert session requests directly")
                }
                EditorRequest::GotoDefinition => {
                    panic!("unit tests should assert LSP requests directly")
                }
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

    /// Restore the process current directory when one cwd-sensitive test ends.
    struct CurrentDirectoryGuard {
        previous: PathBuf,
    }

    impl CurrentDirectoryGuard {
        /// Switch to `path` until the guard drops.
        fn change_to(path: &Path) -> Self {
            let previous = std::env::current_dir().expect("capture current directory");
            std::env::set_current_dir(path).expect("switch current directory");
            Self { previous }
        }
    }

    impl Drop for CurrentDirectoryGuard {
        /// Restore the directory captured before the guard changed it.
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous).expect("restore current directory");
        }
    }

    /// Restoring a session should rebuild buffer order and preserve per-buffer cursors.
    #[test]
    fn test_restore_project_session_reopens_saved_buffers() {
        let session_dir =
            std::env::temp_dir().join(format!("ordex_restore_session_{}", std::process::id()));
        let existing_path = session_dir.join("main.rs");
        let _ = fs::remove_dir_all(&session_dir);
        fs::create_dir_all(&session_dir).expect("create session dir");
        fs::write(&existing_path, "fn main() {}\nlet value = 1;\n").expect("write session file");

        let mut editor = create_editor_with_content("kept");
        editor.file_path = PathBuf::from("kept.txt");
        editor
            .restore_project_session(&crate::session::ProjectSession {
                working_directory: session_dir.clone(),
                active_buffer: 0,
                buffers: vec![
                    crate::session::SessionBuffer {
                        path: existing_path.clone(),
                        cursor: Cursor::new(1, 4),
                    },
                    crate::session::SessionBuffer {
                        path: PathBuf::from("missing.txt"),
                        cursor: Cursor::new(3, 4),
                    },
                    crate::session::SessionBuffer {
                        path: PathBuf::new(),
                        cursor: Cursor::new(0, 0),
                    },
                ],
            })
            .expect("restore project session");

        let summaries = editor.buffer_summaries();
        assert_eq!(summaries.len(), 3);
        assert_eq!(editor.file_name(), "main.rs");
        assert_eq!(editor.cursor_line(), 1);
        assert_eq!(editor.cursor_column(), 4);

        // Re-activating saved buffers should reveal the missing-path named buffer
        // and the unnamed buffer exactly where the session stored them.
        editor.activate_buffer(summaries[1].id);
        assert_eq!(editor.file_name(), "missing.txt");
        assert_eq!(editor.cursor_line(), 0);
        assert_eq!(editor.cursor_column(), 0);

        editor.activate_buffer(summaries[2].id);
        assert_eq!(editor.file_name(), "[No Name]");
        assert_eq!(editor.cursor_line(), 0);
        assert_eq!(editor.cursor_column(), 0);

        let _ = fs::remove_dir_all(session_dir);
    }

    #[test]
    /// Confirm completion generation resets cleanly after reaching the usize limit.
    fn test_next_completion_generation_wraps_after_usize_max() {
        let mut editor = create_editor_with_content("alpha");
        editor.completion_generation = usize::MAX;

        assert_eq!(editor.next_completion_generation(), 0);
        assert_eq!(editor.next_completion_generation(), 1);
    }

    #[test]
    /// Confirm typing more prefix characters keeps the popup anchored where it began.
    fn test_completion_popup_anchor_stays_at_first_popup_position() {
        let mut editor = create_editor_with_content("alphabet\n");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(1, 0);

        handle_key_and_flush_requests(&mut editor, Key::Char('a'));
        let first_anchor = editor
            .completion_session
            .as_ref()
            .expect("completion popup should open after first character")
            .popup_anchor_char_idx;
        let first_cursor_char_idx = editor.cursor.to_char_index(&editor.buffer);

        handle_key_and_flush_requests(&mut editor, Key::Char('l'));
        let session = editor
            .completion_session
            .as_ref()
            .expect("completion popup should stay open while typing");

        assert_eq!(first_anchor, first_cursor_char_idx);
        assert_eq!(session.popup_anchor_char_idx, first_anchor);
        assert!(session.cursor_char_idx > session.popup_anchor_char_idx);
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
                    SequenceDiscoveryEntry {
                        keys: "d".to_string(),
                        action: "Go to definition".to_string(),
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
                        keys: "d".to_string(),
                        action: "Delete current line".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "w".to_string(),
                        action: "Delete word forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "e".to_string(),
                        action: "Delete word end".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "b".to_string(),
                        action: "Delete word backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "W".to_string(),
                        action: "Delete WORD forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "E".to_string(),
                        action: "Delete WORD end".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "B".to_string(),
                        action: "Delete WORD backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "{".to_string(),
                        action: "Delete paragraph backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "}".to_string(),
                        action: "Delete paragraph forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "f".to_string(),
                        action: "Delete find forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "F".to_string(),
                        action: "Delete find backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "t".to_string(),
                        action: "Delete till forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "T".to_string(),
                        action: "Delete till backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "%".to_string(),
                        action: "Delete matching delimiter".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "i".to_string(),
                        action: "Delete inner text object".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "a".to_string(),
                        action: "Delete around text object".to_string(),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_operator_discovery_popup_shows_text_object_targets_after_i_prefix() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));

        assert_eq!(
            editor.sequence_discovery_popup(),
            Some(SequenceDiscoveryPopup {
                prefix: "di".to_string(),
                entries: vec![
                    SequenceDiscoveryEntry {
                        keys: "w".to_string(),
                        action: "Delete inner word".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "W".to_string(),
                        action: "Delete inner WORD".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "(".to_string(),
                        action: "Delete inner paren".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "[".to_string(),
                        action: "Delete inner bracket".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "{".to_string(),
                        action: "Delete inner brace".to_string(),
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
        assert_eq!(editor.viewport.first_visible_line(), 2);
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
    fn test_di_big_word_deletes_inner_word_object() {
        let mut editor = create_editor_with_content("alpha.beta gamma");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('W'));

        assert_eq!(editor.buffer.to_string(), " gamma");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_da_big_word_deletes_word_and_separator() {
        let mut editor = create_editor_with_content("alpha.beta gamma");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('W'));

        assert_eq!(editor.buffer.to_string(), "gamma");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_di_brace_deletes_inside_smallest_surrounding_pair() {
        let mut editor = create_editor_with_content("x{a{b}c}y");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('{'));

        assert_eq!(editor.buffer.to_string(), "x{a{}c}y");
        assert_eq!(editor.cursor.column(), 4);
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
    fn test_dw_deletes_to_next_word_boundary() {
        let mut editor = create_editor_with_content("alpha beta gamma");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), "beta gamma");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_c_e_replaces_through_word_end() {
        let mut editor = create_editor_with_content("alpha.beta");

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('E'));

        assert_eq!(editor.buffer.to_string(), "");
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn test_ye_yanks_through_word_end() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('e'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "alpha".to_string(),
                kind: YankKind::Character,
            })
        );
        assert_eq!(editor.buffer.to_string(), "alpha beta");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_dfx_deletes_through_target_char() {
        let mut editor = create_editor_with_content("alpha,beta");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('f'));
        editor.handle_key(Key::Char(','));

        assert_eq!(editor.buffer.to_string(), "beta");
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_ct_comma_enters_insert_after_deleting_until_target() {
        let mut editor = create_editor_with_content("alpha,beta");

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('t'));
        editor.handle_key(Key::Char(','));

        assert_eq!(editor.buffer.to_string(), ",beta");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_yy_uses_operator_linewise_yank() {
        let mut editor = create_editor_with_content("alpha\nbeta\n");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('y'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "alpha\n".to_string(),
                kind: YankKind::Line,
            })
        );
    }

    #[test]
    fn test_operator_motion_uses_configured_single_key_binding() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.apply_config(&ConfigSettings {
            operator_bindings: vec![crate::config::ConfiguredOperatorBinding {
                key: KeyInput::Char('é'),
                binding: crate::keybindings::OperatorBinding::WordForward,
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('é'));

        assert_eq!(editor.buffer.to_string(), "beta");
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_text_object_uses_configured_word_binding() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.apply_config(&ConfigSettings {
            operator_bindings: vec![crate::config::ConfiguredOperatorBinding {
                key: KeyInput::Char('é'),
                binding: crate::keybindings::OperatorBinding::WordForward,
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('é'));

        assert_eq!(editor.buffer.to_string(), " beta");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_normal_mode_motion_remap_does_not_change_operator_motion_keys() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('é'),
                actions: ActionBinding::single(Action::MoveWordForward),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('é'));

        assert_eq!(editor.buffer.to_string(), "alpha beta");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_dot_repeats_dw() {
        let mut editor = create_editor_with_content("one two three");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('w'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "three");
        assert_eq!(editor.mode, Mode::Normal);
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
                after_write_action: AfterWriteAction::StayOpen,
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
    /// Stale definition results should be ignored without clearing the live lookup.
    fn test_apply_definition_lookup_result_rejects_stale_token() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_definition_lookup = Some(ActiveDefinitionLookup {
            token: 7,
            document_version: 3,
        });

        let changed = editor.apply_definition_lookup_result(DefinitionLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 8,
            document_version: 3,
            outcome: DefinitionLookupOutcome::NotFound,
        });

        assert!(!changed);
        assert_eq!(
            editor.active_definition_lookup,
            Some(ActiveDefinitionLookup {
                token: 7,
                document_version: 3,
            })
        );
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// Definition results should still apply after switching to another open buffer.
    fn test_apply_definition_lookup_result_considers_inactive_origin_buffer() {
        let first = TempFile::with_suffix("_first.rs").expect("create first file");
        first
            .write_all(b"fn first() {}\n")
            .expect("seed first file");
        let second = TempFile::with_suffix("_second.rs").expect("create second file");
        second
            .write_all(b"fn second() {}\n")
            .expect("seed second file");
        let target = TempFile::with_suffix("_target.rs").expect("create target file");
        target
            .write_all(b"fn target() {}\n")
            .expect("seed target file");

        let mut editor = EditorState::new(24);
        editor
            .load_file(first.path())
            .expect("load first workspace file");
        editor.active_definition_lookup = Some(ActiveDefinitionLookup {
            token: 11,
            document_version: 4,
        });
        let first_id = editor.active_buffer_id;
        editor
            .open_startup_buffer(second.path())
            .expect("open second buffer");
        let second_id = editor.active_buffer_id;
        editor.activate_buffer(second_id);

        let changed = editor.apply_definition_lookup_result(DefinitionLookupResult {
            buffer_id: first_id,
            lookup_token: 11,
            document_version: 4,
            outcome: DefinitionLookupOutcome::Single(DefinitionTarget {
                file_path: target.path().to_path_buf(),
                line: 0,
                character: 3,
                display_label: "target.rs:1:4".to_string(),
            }),
        });

        assert!(changed);
        assert_ne!(editor.active_buffer_id, second_id);
        assert_eq!(editor.file_path, target.path());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    /// Multiple definition targets should open the picker instead of jumping immediately.
    fn test_apply_definition_lookup_result_opens_definition_picker() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_definition_lookup = Some(ActiveDefinitionLookup {
            token: 5,
            document_version: 2,
        });

        let changed = editor.apply_definition_lookup_result(DefinitionLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 5,
            document_version: 2,
            outcome: DefinitionLookupOutcome::Multiple(vec![
                DefinitionTarget {
                    file_path: PathBuf::from("src/lib.rs"),
                    line: 0,
                    character: 7,
                    display_label: "src/lib.rs:1:8".to_string(),
                },
                DefinitionTarget {
                    file_path: PathBuf::from("src/other.rs"),
                    line: 1,
                    character: 2,
                    display_label: "src/other.rs:2:3".to_string(),
                },
            ]),
        });

        assert!(changed);
        assert!(matches!(editor.mode, Mode::DefinitionPicker(_)));
        assert!(editor.definition_picker.is_some());
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// A not-found lookup should clear the pending request and surface feedback.
    fn test_apply_definition_lookup_result_sets_not_found_message() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_definition_lookup = Some(ActiveDefinitionLookup {
            token: 9,
            document_version: 6,
        });

        let changed = editor.apply_definition_lookup_result(DefinitionLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 9,
            document_version: 6,
            outcome: DefinitionLookupOutcome::NotFound,
        });

        assert!(changed);
        assert_eq!(editor.active_definition_lookup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No definition found")
        );
    }

    #[test]
    /// Definition jumps should keep buffer paths relative when they stay under cwd.
    fn test_goto_definition_target_opens_relative_path_within_current_directory() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.rs", "fn main() { helper(); }\n")
            .expect("write main file");
        tree.write_file("src/lib.rs", "pub fn helper() {}\n")
            .expect("write lib file");
        let _guard = CurrentDirectoryGuard::change_to(tree.path());

        let mut editor = EditorState::new(24);
        editor
            .load_file(tree.path().join("src/main.rs"))
            .expect("load source file");

        editor.goto_definition_target(&DefinitionTarget {
            file_path: tree.path().join("src/lib.rs"),
            line: 0,
            character: 7,
            display_label: "src/lib.rs:1:8".to_string(),
        });

        assert_eq!(editor.file_path, PathBuf::from("src/lib.rs"));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 7);
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

    #[test]
    fn test_dot_repeats_delete_char_change() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "cd");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_count_on_dot_repeats_last_change_multiple_times() {
        let mut editor = create_editor_with_content("abcdef");

        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "def");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_dot_repeats_insert_session_text() {
        let mut editor = create_editor_with_content("helo\nhelo");
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Esc);

        editor.cursor = Cursor::new(1, 2);
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "hello\nhello");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_dot_repeats_open_line_insert_session() {
        let mut editor = create_editor_with_content("one\ntwo");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "one\nx\nx\ntwo");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_dot_repeats_change_inner_word_session() {
        let mut editor = create_editor_with_content("alpha beta gamma");
        editor.cursor = Cursor::new(0, 7);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));
        editor.handle_key(Key::Char('Z'));
        editor.handle_key(Key::Esc);

        editor.cursor = Cursor::new(0, 8);
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "alpha Z Z");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 8);
    }

    #[test]
    fn test_undo_does_not_replace_repeatable_change() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Char('u'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "bcd");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_visual_delete_is_not_repeatable_by_dot() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "cd");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.status_message.as_deref(), Some("Nothing to repeat"));
    }
}
