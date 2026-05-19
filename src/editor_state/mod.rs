//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::completion::{
    CompletionCandidate, CompletionDirection, CompletionRequest, CompletionRequestIdentity,
    CompletionSession, CompletionSourceId, CompletionSourceRegistry, PendingAsyncCompletion,
    build_lsp_trigger_request_identity, build_request_identity, refresh_session,
    shift_popup_anchor_for_insert, shift_popup_anchor_for_removal,
};
use crate::config::ConfigSettings;
use crate::cursor::Cursor;
use crate::dialogs::{
    BufferSwitchItem, BufferSwitchState, CodeActionPickerItem, CodeActionPickerState,
    DEFAULT_FILE_PICKER_MAX_FILES, DiagnosticPickerItem, DiagnosticPickerState,
    FilePickerPollResult, FilePickerState, HoverPopup, LocationPickerItem, LocationPickerState,
    SignatureHelpPopup,
};
use crate::keybindings::{Action, ActionBinding, KeyBindings, KeyInput, SequenceMatch};
use crate::lsp::protocol::{
    LspCompletionItem, LspPosition, LspRange, LspTextChange, LspWorkspaceEdit,
};
use crate::lsp::{
    CodeActionLookupOutcome, CodeActionLookupResult, CodeActionRequestSnapshot,
    CompletionLookupOutcome, CompletionLookupResult, CompletionRequestSnapshot,
    DocumentSyncOutcome, HoverLookupOutcome, HoverLookupResult, HoverRequestSnapshot,
    LspCodeAction, LspDiagnostic, LspDiagnosticSeverity, LspFileDiagnostics, NavigationKind,
    NavigationLookupOutcome, NavigationLookupResult, NavigationRequestSnapshot, NavigationTarget,
    RenameLookupOutcome, RenameLookupResult, RenameRequestSnapshot, SignatureHelpLookupOutcome,
    SignatureHelpLookupResult, SignatureHelpRequestSnapshot,
};
use crate::mode::{Mode, VisualKind};
use crate::navigation::{
    WordStyle, find_next_paragraph_line, find_next_word_start_with_style, find_prev_paragraph_line,
    find_prev_word_end, find_prev_word_end_with_style, find_prev_word_start_with_style,
    find_word_end_with_style,
};
use crate::path_utils::current_dir_relative_path;
use crate::search::{SearchMatch, SearchQuery};
use crate::session::{ProjectSession, SessionBuffer, normalize_session_buffer_path};
use crate::soft_wrap;
use crate::swap::{self, SwapHandle};
use crate::syntax::helpers::identifier_can_continue;
use crate::syntax::profile::{IdentifierPattern, ascii_identifier};
use crate::syntax::profiles::detect_language_details;
use crate::syntax::{BufferEdit, HighlightSpan, SyntaxClass, SyntaxEngine};
use crate::text_buffer::TextBuffer;
use crate::themes;
use crate::tui;
use crate::viewport::Viewport;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use termion::event::Key;

mod actions;
mod buffers;
mod commands;
mod editing;
mod ex_commands;
mod go_to;
mod history;
mod indent;
mod jump_history;
mod lookup;
mod lsp_edits;
mod macros;
mod matching;
mod operator;
mod prompt_history;
mod repeat;
mod search_highlighting;
mod substitute_preview;
mod view;

pub(crate) use buffers::BufferSummary;
use buffers::{
    BufferManager, BufferState, OrderedBufferState, display_file_name, normalize_lookup_path,
    paths_match,
};
use jump_history::JumpHistory;
use lookup::{
    ActiveCodeActionLookup, ActiveHoverLookup, ActiveNavigationLookup, ActiveRenameLookup,
    ActiveSignatureHelpLookup, LookupTokenSource,
};
use macros::{MacroState, PendingMacro};
pub(crate) use matching::VisibleMatchRole;
use operator::{ExecutedOperatorCommand, OperatorKind, PendingOperator};
use prompt_history::{PromptHistory, PromptHistoryKind, PromptHistoryScope};

const DEFAULT_INDENT_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindDirection {
    Forward,
    Backward,
}

impl FindDirection {
    /// Return the opposite direction for repeated find motions.
    fn reversed(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }
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

/// Describe whether a repeated find keeps or flips the stored direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindRepeatDirection {
    Same,
    Reversed,
}

/// Describe whether a find motion should replace the stored last-find state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastFindUpdate {
    Store,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LastVisualSelection {
    /// First selected character index for characterwise or linewise recreation.
    start_char_idx: usize,
    /// One-past-the-end selected character index for characterwise or linewise recreation.
    end_char_idx: usize,
    /// Number of logical lines touched by the saved selection.
    line_count: usize,
    /// Whether the active cursor sat on the lower bound of the saved selection.
    cursor_at_start: bool,
    /// Character index of the saved visual anchor before later buffer edits.
    anchor_char_idx: usize,
    /// Character index of the saved active cursor before later buffer edits.
    cursor_char_idx: usize,
    /// Visual-mode variant used by the saved selection.
    kind: VisualKind,
}

impl LastVisualSelection {
    /// Shift the stored selection to account for one insertion into the buffer.
    fn shift_for_insert(&mut self, insert_char_idx: usize, inserted_char_count: usize) {
        if insert_char_idx < self.start_char_idx {
            self.start_char_idx += inserted_char_count;
            self.end_char_idx += inserted_char_count;
        } else if insert_char_idx < self.end_char_idx {
            self.end_char_idx += inserted_char_count;
        }
        if insert_char_idx <= self.anchor_char_idx {
            self.anchor_char_idx += inserted_char_count;
        }
        if insert_char_idx <= self.cursor_char_idx {
            self.cursor_char_idx += inserted_char_count;
        }
    }

    /// Shift the stored selection to account for one removal from the buffer.
    fn shift_for_removal(&mut self, start_char: usize, end_char: usize) {
        self.start_char_idx =
            shift_selection_index_for_removal(self.start_char_idx, start_char, end_char);
        self.end_char_idx =
            shift_selection_index_for_removal(self.end_char_idx, start_char, end_char);
        self.anchor_char_idx =
            shift_selection_index_for_removal(self.anchor_char_idx, start_char, end_char);
        self.cursor_char_idx =
            shift_selection_index_for_removal(self.cursor_char_idx, start_char, end_char);
    }
}

/// Translate one stored selection boundary after removing a character range.
fn shift_selection_index_for_removal(index: usize, start_char: usize, end_char: usize) -> usize {
    if end_char <= index {
        index - (end_char - start_char)
    } else if start_char < index {
        start_char
    } else {
        index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    BufferSwitch,
    FilePicker,
    LocationPicker,
    DiagnosticPicker,
    CodeActionPicker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOverwrite {
    target_path: PathBuf,
    update_file_path: bool,
    after_write_action: AfterWriteAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSoftReadOnlySave {
    target_path: PathBuf,
    /// Whether the confirmed write should also adopt `target_path` as the buffer path.
    ///
    /// `true` means this save behaves like `:write <path>` and updates the active
    /// buffer to that path after success. `false` keeps the current buffer path.
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

/// Action taken when a swap prompt is cancelled.
#[derive(Debug)]
enum PendingSwapCancelAction {
    CloseBuffer,
    Quit,
}

/// Why one swap prompt is currently shown.
#[derive(Debug)]
enum PendingSwapPromptKind {
    Recovery,
    Conflict,
}

/// Recovery prompt state for one buffer whose previous swap file still exists.
#[derive(Debug)]
struct PendingSwapPrompt {
    /// Full prompt text rendered on the message line.
    prompt: String,
    /// Buffer content loaded from the existing swap file.
    recovered_buffer: TextBuffer,
    /// Swap-file path that may be discarded by one prompt action.
    swap_path: PathBuf,
    /// Why this prompt is currently shown.
    kind: PendingSwapPromptKind,
    /// What cancel should do after dismissing the prompt.
    cancel_action: PendingSwapCancelAction,
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
    /// Replay one selection-shaped change at the current cursor position.
    Selection(SelectionRepeatCommand),
    /// Replay one completed `r{char}` replacement with its captured count.
    ReplaceChar { count: usize, replacement: char },
}

/// One repeatable change that reuses a stored selection shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionRepeatCommand {
    /// Change to apply over the resolved selection.
    action: SelectionRepeatAction,
    /// Selection shape to rebuild from the current cursor position.
    target: SelectionRepeatTarget,
}

/// Distinguish the selection-shaped changes that `.` can replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionRepeatAction {
    Delete,
    Change,
    ToggleCase,
    Reindent,
    Indent,
    Dedent,
    InsertBlockStart,
    AppendBlockEnd,
}

/// Describe how `.` rebuilds one stored selection at the current cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionRepeatTarget {
    /// Repeat over the next `char_count` characters from the cursor.
    Character { char_count: usize },
    /// Repeat over the next `line_count` logical lines from the cursor line.
    Lines { line_count: usize },
    /// Repeat over the next rectangular block from the cursor.
    Block {
        line_count: usize,
        column_count: usize,
    },
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

/// One auto-indented blank line that may still shed its inserted indentation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAutoIndentLine {
    /// Logical line index that received the auto-inserted indentation.
    line: usize,
    /// Exact indentation text inserted for that line.
    indent: String,
    /// Whether user edits touched the line after auto-indentation.
    touched: bool,
}

/// Distinguish the two block-aligned insert targets available from Visual mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualInsertKind {
    BlockStart,
    BlockEnd,
}

/// Track one blockwise insert session mirrored across multiple lines.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VisualInsertSession {
    /// Insert-session start position for the primary line that owns the real cursor.
    primary_start_char_idx: usize,
    /// Fixed block column where mirrored insertions should stay anchored.
    target_column: usize,
    /// Whether mirrored targets land at the block start or block end.
    kind: VisualInsertKind,
    /// Mirrored secondary lines ordered from bottom to top.
    secondary_lines: Vec<usize>,
}

/// One pending Normal-mode replace command waiting for its replacement character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingReplace {
    /// Number of same-line characters to replace once the target arrives.
    count: usize,
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

/// One blockwise Visual selection defined by logical rows plus inclusive columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockSelection {
    /// First logical line covered by the block.
    start_line: usize,
    /// Last logical line covered by the block.
    end_line: usize,
    /// Left-most selected logical column.
    left_column: usize,
    /// Right-most selected logical column.
    right_column: usize,
}

impl BlockSelection {
    /// Return the number of logical lines touched by this block.
    fn line_count(self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }

    /// Return whether this block highlights `column` on `line_idx`.
    ///
    /// Returns `true` when the logical cell falls inside the block and maps to a
    /// real buffer character on that line, and `false` for lines or columns
    /// outside the block bounds or beyond the line end.
    fn contains_cell(self, buffer: &TextBuffer, line_idx: usize, column: usize) -> bool {
        (self.start_line..=self.end_line).contains(&line_idx)
            && (self.left_column..=self.right_column).contains(&column)
            && column < buffer.line_len(line_idx)
    }

    /// Return the concrete character ranges selected on each touched line.
    fn segments(self, buffer: &TextBuffer) -> Vec<SelectionRange> {
        let mut segments = Vec::with_capacity(self.line_count());

        // Block selections truncate to real characters on each line, so short
        // lines simply contribute no segment instead of introducing virtual cells.
        for line_idx in self.start_line..=self.end_line {
            let line_len = buffer.line_len(line_idx);
            if self.left_column >= line_len {
                continue;
            }
            let start = buffer.line_to_char(line_idx) + self.left_column;
            let end = buffer.line_to_char(line_idx) + (self.right_column + 1).min(line_len);
            if end > start {
                segments.push(SelectionRange { start, end });
            }
        }
        segments
    }

    /// Return one block-yank payload row for every touched logical line.
    fn yank_lines(self, buffer: &TextBuffer) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.line_count());

        // Keep one payload row per touched line so blockwise puts can preserve
        // vertical shape even when some lines contribute no visible characters.
        for line_idx in self.start_line..=self.end_line {
            let line_len = buffer.line_len(line_idx);
            if self.left_column >= line_len {
                lines.push(String::new());
                continue;
            }
            let start = buffer.line_to_char(line_idx) + self.left_column;
            let end = buffer.line_to_char(line_idx) + (self.right_column + 1).min(line_len);
            lines.push(buffer.slice_string(start, end));
        }
        lines
    }

    /// Convert this block into one whole-line range for indent-style commands.
    fn line_selection_range(self, buffer: &TextBuffer) -> SelectionRange {
        let start = buffer.line_to_char(self.start_line);
        let end = if self.end_line + 1 < buffer.lines_count() {
            buffer.line_to_char(self.end_line + 1)
        } else {
            buffer.chars_count()
        };
        SelectionRange { start, end }
    }
}

/// One active Visual selection resolved from the current mode and endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualSelection {
    /// One contiguous characterwise selection.
    Character(SelectionRange),
    /// One contiguous linewise selection.
    Line(SelectionRange),
    /// One rectangular blockwise selection.
    Block(BlockSelection),
}

impl VisualSelection {
    /// Return whether this selection highlights `column` on `line_idx`.
    ///
    /// Returns `true` when the given logical cell belongs to this selection and
    /// `false` when it falls outside the selected region.
    fn contains_cell(self, buffer: &TextBuffer, line_idx: usize, column: usize) -> bool {
        match self {
            Self::Character(selection) | Self::Line(selection) => {
                let char_idx = buffer.line_to_char(line_idx) + column;
                (selection.start..selection.end).contains(&char_idx)
            }
            Self::Block(selection) => selection.contains_cell(buffer, line_idx, column),
        }
    }
}

/// Distinguish characterwise and linewise unnamed-register contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YankKind {
    Character,
    Line,
    Block,
}

/// Stored contents of the editor-owned unnamed paste buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct YankBuffer {
    text: String,
    kind: YankKind,
}

/// One debounced automatic LSP completion request waiting to be dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingLspCompletion {
    /// Request identity and generation captured when the lookup was queued.
    request: CompletionRequest,
    /// Popup anchor preserved while asynchronous candidates are still pending.
    popup_anchor_char_idx: usize,
    /// Buffer version captured when the lookup was queued.
    document_version: i32,
    /// Deadline when the app loop may dispatch this completion lookup.
    due_at: Instant,
    /// Recently typed trigger text used to classify one immediate trigger request.
    trigger_text: Option<String>,
}

/// One debounced automatic LSP signature-help request waiting to be dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingLspSignatureHelp {
    /// Monotonic lookup token used to reject stale responses from older requests.
    lookup_token: u64,
    /// Buffer version captured when the lookup was queued.
    document_version: i32,
    /// Cursor character index captured when the lookup was queued.
    cursor_char_idx: usize,
    /// Stable anchor kept at the start of the active call context.
    anchor_char_idx: usize,
    /// Deadline when the app loop may dispatch this signature-help lookup.
    due_at: Instant,
    /// Recently typed trigger text used to classify one immediate trigger request.
    trigger_text: Option<String>,
    /// Whether this request refreshes an already-visible signature-help popup.
    is_retrigger: bool,
}

/// One in-flight automatic LSP completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveLspCompletion {
    /// Request identity and generation captured when the lookup started.
    request: CompletionRequest,
    /// Buffer version captured when the lookup started.
    document_version: i32,
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
    ContinueWriteAllSequence {
        remaining_buffer_ids: VecDeque<usize>,
        return_to_buffer_id: usize,
    },
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
    LspNavigation(NavigationKind),
    LspHover,
    LspRename(String),
    LspCodeAction,
}

/// Runtime editor settings that have built-in defaults and may be overridden by config.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorSettings {
    scroll_margin: usize,
    horizontal_scroll_margin: usize,
    relative_line_numbers: bool,
    soft_wrap: bool,
    indent_width: usize,
    indent_with_tabs: bool,
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
            indent_width: DEFAULT_INDENT_WIDTH,
            indent_with_tabs: false,
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

/// Active-buffer diagnostic totals shown in compact UI chrome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DiagnosticCounts {
    /// Number of active error diagnostics in the current buffer.
    pub(crate) errors: usize,
    /// Number of active warning diagnostics in the current buffer.
    pub(crate) warnings: usize,
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
    /// Whether the active file is currently reported as read-only by the filesystem.
    read_only: bool,
    /// Whether the active buffer was intentionally opened in soft read-only mode.
    soft_read_only: bool,
    /// Derived syntax-highlighting state for the current document.
    syntax: SyntaxEngine,
    /// Inactive buffers plus navigation order for all open buffers.
    buffer_manager: BufferManager,
    /// Status message to display on the next render pass.
    status_message: Option<String>,
    /// Whether the terminal message row still needs one redraw to remove a one-shot status.
    ///
    /// `status_message` is only the in-memory source for the next render pass. Once
    /// the message row has been painted, the editor clears `status_message`, but the
    /// terminal still shows those bytes until some later redraw touches the message
    /// row again. Setting `status_message` to `None` alone therefore does not clear
    /// the visible row, especially when the next state change qualifies for a
    /// cursor-only redraw that skips the message line entirely.
    message_line_needs_clear: bool,
    /// Whether a previously rendered multi-line status overlay still needs one full redraw.
    ///
    /// Multi-line errors live in the buffer area instead of the bottom message row,
    /// so clearing them requires repainting the underlying text rather than only
    /// touching the terminal's last line.
    status_overlay_needs_clear: bool,
    /// Bounded LSP progress lines rendered above the bottom bars.
    lsp_progress_lines: Vec<String>,
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
    /// Last successfully compiled search used by / search.
    last_search: Option<SearchQuery>,
    /// Pending multi-key sequence in normal mode (e.g. 'g' waiting for continuation).
    pending_sequence: Vec<KeyInput>,
    /// Count prefix typed before a normal-mode command.
    pending_count: Option<usize>,
    /// Count prefix captured before `/` so Enter can advance the initial search.
    pending_search_count: Option<usize>,
    /// Count prefix captured when entering a pending multi-key sequence.
    pending_sequence_count: Option<usize>,
    /// Motion-side count typed after an operator prefix like `d`/`c`.
    pending_sequence_motion_count: Option<usize>,
    /// Pending delete/change/yank operator waiting for a motion or text object.
    pending_operator: Option<PendingOperator>,
    /// Pending macro action waiting for one register key.
    pending_macro: Option<PendingMacro>,
    /// Pending find/till motion waiting for a target character.
    pending_find: Option<FindMotion>,
    /// Pending `r` replacement waiting for the typed replacement character.
    pending_replace: Option<PendingReplace>,
    /// Last attempted character find/till motion used by ';' and ','.
    last_find: Option<LastFind>,
    /// Last visual selection that can be recreated via normal-mode `gv`.
    last_visual_selection: Option<LastVisualSelection>,
    /// Editor-owned unnamed register used by yank, delete, and paste actions.
    yank_buffer: Option<YankBuffer>,
    /// Session-local macro registers plus active recording/playback state.
    macro_state: MacroState,
    /// Pending overwrite confirmation for save commands targeting an existing file.
    pending_overwrite: Option<PendingOverwrite>,
    /// Pending confirmation before saving a soft read-only buffer in place.
    pending_soft_read_only_save: Option<PendingSoftReadOnlySave>,
    /// Pending quit confirmation for `:q` with unsaved changes.
    pending_quit_confirmation: Option<PendingQuitConfirmation>,
    /// Pending confirmation for replacing dirty buffers while opening a session.
    pending_session_open_confirmation: Option<PendingSessionOpenConfirmation>,
    /// Pending recovery choice for an existing swap file.
    pending_swap_recovery: Option<PendingSwapPrompt>,
    /// Pending close confirmation for `:bd` with unsaved changes.
    pending_buffer_close_confirmation: bool,
    /// Active buffer-switch picker state while the overlay is open.
    buffer_switch: Option<BufferSwitchState>,
    /// Active file-picker state while the overlay is open.
    file_picker: Option<FilePickerState>,
    /// Active navigation-target picker state while the overlay is open.
    location_picker: Option<LocationPickerState>,
    /// Active diagnostics picker state while the overlay is open.
    diagnostic_picker: Option<DiagnosticPickerState>,
    /// Active code-action picker state while the overlay is open.
    code_action_picker: Option<CodeActionPickerState>,
    /// Registered completion sources available to the insert-mode popup flow.
    completion_sources: CompletionSourceRegistry,
    /// Monotonic generation used to discard stale completion refreshes.
    completion_generation: usize,
    /// Active inline completion session for Insert mode, if any.
    completion_session: Option<CompletionSession>,
    /// In-flight asynchronous completion request owned by one local source, if any.
    pending_async_completion: Option<PendingAsyncCompletion>,
    /// Debounced automatic LSP completion request waiting for dispatch, if any.
    pending_lsp_completion: Option<PendingLspCompletion>,
    /// Debounced automatic LSP signature-help request waiting for dispatch, if any.
    pending_lsp_signature_help: Option<PendingLspSignatureHelp>,
    /// In-flight automatic LSP completion request, if any.
    active_lsp_completion: Option<ActiveLspCompletion>,
    /// `%`-matching cache and visible passive highlight state.
    matching: matching::MatchingState,
    /// Search-result preview plus visible viewport highlights.
    search_highlighting: search_highlighting::SearchHighlightState,
    /// Transient `:s` preview state rendered without mutating the committed buffer.
    substitute_preview: Option<substitute_preview::SubstitutePreviewState>,
    /// Monotonic token that forces full redraws when substitute preview changes.
    substitute_preview_revision: u64,
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
    /// Shared monotonic token source used to reject stale LSP lookup results.
    lookup_tokens: LookupTokenSource,
    /// Next global edit generation assigned to any buffer mutation in this editor.
    next_edit_generation: u64,
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
    /// Session-wide jump history for meaningful non-local navigation.
    jump_history: JumpHistory,
    /// Session-local `:` and `/` prompt history with traversal state.
    prompt_history: PromptHistory,
    /// Swap file handle associated with the active buffer.
    swap: Option<SwapHandle>,
    /// Deadline for the next debounced swap refresh after an edit.
    pending_swap_refresh_at: Option<Instant>,
    /// Whether the active buffer must not create or refresh a swap file right now.
    ///
    /// This stays enabled while another Ordex instance still owns the swap file,
    /// so this editor does not overwrite that foreign "file is open" marker.
    suppress_swap_creation: bool,
    /// Last repeatable change used by Normal-mode `.` replay.
    last_repeatable_change: Option<RepeatableChange>,
    /// Selection-shaped Visual change waiting to become the next `.` target.
    pending_visual_repeat: Option<SelectionRepeatCommand>,
    /// Cursor position after the latest committed change in the active buffer.
    last_committed_change_char_idx: Option<usize>,
    /// Pending insert-style capture being assembled until Insert mode finishes.
    active_insert_repeat: Option<ActiveInsertRepeatCapture>,
    /// Active mirrored insert session started from one Visual selection, if any.
    visual_insert_session: Option<VisualInsertSession>,
    /// Most-recent-first history of named buffers visited during this session.
    recent_named_buffers: VecDeque<usize>,
    /// One untouched auto-indented blank line that may still be cleaned up.
    pending_auto_indent: Option<PendingAutoIndentLine>,
    /// Suppress repeat capture while replaying a stored `.` change.
    replaying_repeat: bool,
    /// Monotonic document version sent to the language server for the active buffer.
    lsp_document_version: i32,
    /// Ordered edits queued for the next successful LSP sync of the active buffer.
    pending_lsp_changes: Vec<LspTextChange>,
    /// Deadline when the next proactive LSP sync may be dispatched.
    pending_lsp_sync_at: Option<Instant>,
    /// Most recent global edit generation applied to the active buffer.
    last_edit_generation: u64,
    /// Last active navigation lookup request for the active buffer, if any.
    active_navigation_lookup: Option<ActiveNavigationLookup>,
    /// Last active hover lookup request for the active buffer, if any.
    active_hover_lookup: Option<ActiveHoverLookup>,
    /// Last active signature-help lookup request for the active buffer, if any.
    active_signature_help_lookup: Option<ActiveSignatureHelpLookup>,
    /// Last active rename lookup request for the active buffer, if any.
    active_rename_lookup: Option<ActiveRenameLookup>,
    /// Last active code-action lookup request for the active buffer, if any.
    active_code_action_lookup: Option<ActiveCodeActionLookup>,
    /// Read-only hover popup rendered near the active cursor, if visible.
    hover_popup: Option<HoverPopup>,
    /// Read-only signature-help popup rendered near the active cursor, if visible.
    signature_help_popup: Option<SignatureHelpPopup>,
    /// File diagnostics keyed by normalized file path.
    lsp_diagnostics: HashMap<PathBuf, LspFileDiagnostics>,
    /// Force the next render snapshot comparison to request one full redraw.
    redraw_requested: bool,
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
    /// Delay after the most recent edit before proactive LSP sync is dispatched.
    const LSP_SYNC_DEBOUNCE_DELAY: Duration = Duration::from_millis(75);
    /// Delay after one ordinary insert-mode edit before automatic LSP completion runs.
    const LSP_COMPLETION_DEBOUNCE_DELAY: Duration = Duration::from_millis(50);
    /// Delay after one ordinary insert-mode edit before automatic signature help runs.
    const LSP_SIGNATURE_HELP_DEBOUNCE_DELAY: Duration = Duration::from_millis(50);

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
            read_only: false,
            soft_read_only: false,
            syntax: SyntaxEngine::new(),
            buffer_manager: BufferManager::new(0),
            status_message: None,
            message_line_needs_clear: false,
            status_overlay_needs_clear: false,
            lsp_progress_lines: Vec::new(),
            settings: EditorSettings::default(),
            desired_visual_column: None,
            keybindings: KeyBindings::new(),
            should_quit: false,
            quit_exit_code: 0,
            last_search: None,
            pending_sequence: Vec::new(),
            pending_count: None,
            pending_search_count: None,
            pending_sequence_count: None,
            pending_sequence_motion_count: None,
            pending_operator: None,
            pending_macro: None,
            pending_find: None,
            pending_replace: None,
            last_find: None,
            last_visual_selection: None,
            yank_buffer: None,
            macro_state: MacroState::default(),
            pending_overwrite: None,
            pending_soft_read_only_save: None,
            pending_quit_confirmation: None,
            pending_session_open_confirmation: None,
            pending_swap_recovery: None,
            pending_buffer_close_confirmation: false,
            buffer_switch: None,
            file_picker: None,
            location_picker: None,
            diagnostic_picker: None,
            code_action_picker: None,
            completion_sources: CompletionSourceRegistry::new(),
            completion_generation: 0,
            completion_session: None,
            pending_async_completion: None,
            pending_lsp_completion: None,
            pending_lsp_signature_help: None,
            active_lsp_completion: None,
            matching: matching::MatchingState::new(),
            search_highlighting: search_highlighting::SearchHighlightState::new(),
            substitute_preview: None,
            substitute_preview_revision: 0,
            ignore_input_escape_cancel_until: None,
            pending_request: None,
            lookup_tokens: LookupTokenSource::new(),
            next_edit_generation: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            active_undo: None,
            saved_undo_depth: 0,
            replaying_history: false,
            jump_history: JumpHistory::new(),
            prompt_history: PromptHistory::new(),
            swap: None,
            pending_swap_refresh_at: None,
            suppress_swap_creation: false,
            last_repeatable_change: None,
            pending_visual_repeat: None,
            last_committed_change_char_idx: None,
            active_insert_repeat: None,
            visual_insert_session: None,
            recent_named_buffers: VecDeque::new(),
            pending_auto_indent: None,
            replaying_repeat: false,
            lsp_document_version: 0,
            pending_lsp_changes: Vec::new(),
            pending_lsp_sync_at: None,
            last_edit_generation: 0,
            active_navigation_lookup: None,
            active_hover_lookup: None,
            active_signature_help_lookup: None,
            active_rename_lookup: None,
            active_code_action_lookup: None,
            hover_popup: None,
            signature_help_popup: None,
            lsp_diagnostics: HashMap::new(),
            redraw_requested: false,
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

        if let Some(width) = settings.indent_width {
            self.settings.indent_width = width.max(1);
        }

        if let Some(enabled) = settings.indent_with_tabs {
            self.settings.indent_with_tabs = enabled;
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

    /// Return the active file path when the current buffer is named.
    fn active_named_file_path(&self) -> Option<&Path> {
        (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.as_path())
    }

    /// Clamp one logical line and column to a valid Normal-mode cursor position.
    fn clamped_normal_cursor(&self, line: usize, column: usize) -> Cursor {
        let line = line.min(self.buffer.lines_count().saturating_sub(1));
        let column = column.min(self.buffer.line_len(line).saturating_sub(1));
        Cursor::new(line, column)
    }

    /// Normalize modal state after a non-local navigation sets the active cursor.
    fn finish_nonlocal_navigation(&mut self) {
        self.visual_anchor = None;
        self.mode = Mode::Normal;
        self.desired_visual_column = None;
        self.clear_pending_modal_state();
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
    }

    /// Load a file into the editor using chunked reading for efficiency
    pub(crate) fn load_file(&mut self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        let file = File::open(path)?;
        self.buffer = TextBuffer::from_reader(file)?;
        self.file_path = path.to_path_buf();
        self.soft_read_only = false;
        self.refresh_active_read_only_state();
        self.cursor = Cursor::new(0, 0);
        self.desired_visual_column = None;
        self.viewport.set_first_visible_line(0);
        self.refresh_syntax();
        self.reset_history();
        // Loading from disk establishes a fresh LSP baseline, so drop any queued
        // edits and schedule one clean `didOpen` for the new path.
        self.lsp_document_version = 0;
        self.pending_lsp_changes.clear();
        self.pending_lsp_sync_at = Some(Instant::now());
        self.clear_active_lookup_state();
        self.hover_popup = None;
        self.dismiss_signature_help();
        self.record_active_named_buffer();
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
        self.soft_read_only = false;
        self.refresh_active_read_only_state();
        self.refresh_syntax();
        self.lsp_document_version = 0;
        self.pending_lsp_changes.clear();
        self.pending_lsp_sync_at = (!self.file_path.as_os_str().is_empty()).then(Instant::now);
        self.clear_active_lookup_state();
        self.hover_popup = None;
        self.dismiss_signature_help();
        self.record_active_named_buffer();
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
            read_only,
            soft_read_only,
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
            suppress_swap_creation,
            lsp_document_version,
            pending_lsp_changes,
            pending_lsp_sync_at,
            last_edit_generation,
            last_committed_change_char_idx,
            active_navigation_lookup,
            active_rename_lookup,
            active_code_action_lookup,
        } = state;
        let previous = BufferState {
            id: std::mem::replace(&mut self.active_buffer_id, id),
            buffer: std::mem::replace(&mut self.buffer, buffer),
            cursor: std::mem::replace(&mut self.cursor, cursor),
            viewport: std::mem::replace(&mut self.viewport, viewport),
            file_path: std::mem::replace(&mut self.file_path, file_path),
            read_only: std::mem::replace(&mut self.read_only, read_only),
            soft_read_only: std::mem::replace(&mut self.soft_read_only, soft_read_only),
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
            suppress_swap_creation: std::mem::replace(
                &mut self.suppress_swap_creation,
                suppress_swap_creation,
            ),
            lsp_document_version: std::mem::replace(
                &mut self.lsp_document_version,
                lsp_document_version,
            ),
            pending_lsp_changes: std::mem::replace(
                &mut self.pending_lsp_changes,
                pending_lsp_changes,
            ),
            pending_lsp_sync_at: std::mem::replace(
                &mut self.pending_lsp_sync_at,
                pending_lsp_sync_at,
            ),
            last_edit_generation: std::mem::replace(
                &mut self.last_edit_generation,
                last_edit_generation,
            ),
            last_committed_change_char_idx: std::mem::replace(
                &mut self.last_committed_change_char_idx,
                last_committed_change_char_idx,
            ),
            active_navigation_lookup: std::mem::replace(
                &mut self.active_navigation_lookup,
                active_navigation_lookup,
            ),
            active_rename_lookup: std::mem::replace(
                &mut self.active_rename_lookup,
                active_rename_lookup,
            ),
            active_code_action_lookup: std::mem::replace(
                &mut self.active_code_action_lookup,
                active_code_action_lookup,
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
        self.record_active_named_buffer();
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
        self.dismiss_signature_help();
        self.clear_pending_modal_state();
        self.pending_overwrite = None;
        self.pending_soft_read_only_save = None;
        self.pending_quit_confirmation = None;
        self.pending_session_open_confirmation = None;
        self.pending_swap_recovery = None;
        self.pending_buffer_close_confirmation = false;
        self.buffer_switch = None;
        self.clear_picker_and_hover_state();
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
        self.cursor = self.clamped_normal_cursor(cursor.line(), cursor.column());
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
            Mode::LocationPicker(_) => Some(PickerKind::LocationPicker),
            Mode::DiagnosticPicker(_) => Some(PickerKind::DiagnosticPicker),
            Mode::CodeActionPicker(_) => Some(PickerKind::CodeActionPicker),
            _ => None,
        }
    }

    /// Clear all in-flight lookup state tied to the active buffer snapshot.
    fn clear_active_lookup_state(&mut self) {
        self.active_navigation_lookup = None;
        self.active_hover_lookup = None;
        self.active_signature_help_lookup = None;
        self.active_rename_lookup = None;
        self.active_code_action_lookup = None;
    }

    /// Clear hover UI state together with hover, rename, and code-action requests.
    fn clear_hover_and_rename_state(&mut self) {
        self.active_hover_lookup = None;
        self.dismiss_signature_help();
        self.active_rename_lookup = None;
        self.active_code_action_lookup = None;
        self.hover_popup = None;
    }

    /// Build the prefilled command text used by the rename shortcut.
    fn prefilled_rename_command(&self) -> String {
        let mut command = String::from("rename ");
        if let Some(symbol) = self.current_rename_symbol() {
            command.push_str(&symbol);
        }
        command
    }

    /// Return the identifier-like symbol under the cursor for rename prefill.
    fn current_rename_symbol(&self) -> Option<String> {
        let cursor_idx = self.cursor.to_char_index(&self.buffer);
        let symbol_idx = if self
            .buffer
            .char_at(cursor_idx)
            .is_some_and(|ch| self.is_identifier_char_in_current_buffer(ch))
        {
            cursor_idx
        } else if cursor_idx > 0
            && self
                .buffer
                .char_at(cursor_idx - 1)
                .is_some_and(|ch| self.is_identifier_char_in_current_buffer(ch))
        {
            // When the cursor sits just after an identifier, reuse the symbol on
            // the left so the shortcut still prefills the visible item name.
            cursor_idx - 1
        } else {
            return None;
        };
        let mut start = symbol_idx;
        while start > 0
            && self
                .buffer
                .char_at(start - 1)
                .is_some_and(|ch| self.is_identifier_char_in_current_buffer(ch))
        {
            start -= 1;
        }
        let mut end = symbol_idx + 1;
        while self
            .buffer
            .char_at(end)
            .is_some_and(|ch| self.is_identifier_char_in_current_buffer(ch))
        {
            end += 1;
        }
        Some(self.buffer.slice_string(start, end))
    }

    /// Return the identifier pattern used by word-oriented helpers in this buffer.
    fn current_buffer_identifier_pattern(&self) -> Option<IdentifierPattern> {
        if self.file_path.as_os_str().is_empty() {
            return Some(ascii_identifier());
        }
        let path = self.file_path.as_path();
        match detect_language_details(Some(path)) {
            Some((profile, _)) => profile.identifier,
            None => Some(ascii_identifier()),
        }
    }

    /// Return whether `ch` belongs to an identifier-like word in this buffer.
    ///
    /// Returns `true` for characters that the active syntax profile allows in a
    /// buffer-specific identifier, and `false` for separators, punctuation, or
    /// languages that intentionally expose no identifier pattern.
    fn is_identifier_char_in_current_buffer(&self, ch: char) -> bool {
        self.current_buffer_identifier_pattern()
            .is_some_and(|pattern| identifier_can_continue(pattern, ch))
    }

    /// Open the buffer-switch picker with the current ordered buffer list.
    fn open_buffer_switcher(&mut self) {
        self.prepare_picker_open();
        self.buffer_switch = Some(BufferSwitchState::new(self.buffer_switch_items()));
        self.mode = Mode::buffer_switch_empty();
    }

    /// Build buffer-switch picker rows with the active buffer pinned first.
    fn buffer_switch_items(&self) -> Vec<BufferSwitchItem> {
        let recent_named_ranks = self
            .recent_named_buffers
            .iter()
            .copied()
            .enumerate()
            .map(|(rank, buffer_id)| (buffer_id, rank))
            .collect::<HashMap<_, _>>();
        let mut items = self
            .buffer_manager
            .summaries(
                self.active_buffer_id,
                self.file_name(),
                &self.file_path,
                self.buffer.is_modified(),
            )
            .into_iter()
            .enumerate()
            .map(|(open_order, summary)| {
                // Keep the active buffer pinned, then prefer named buffers by
                // session recency so the alternate file stays near the top.
                let sort_group = if summary.active {
                    0
                } else if let Some(rank) = recent_named_ranks.get(&summary.id) {
                    return (1, *rank, open_order, summary);
                } else if self.named_file_path_for_buffer_id(summary.id).is_some() {
                    2
                } else {
                    3
                };
                (sort_group, open_order, open_order, summary)
            })
            .collect::<Vec<_>>();

        // Preserve stable open-buffer order inside each fallback group so this
        // picker change does not affect unnamed buffers or untracked named ones.
        items
            .sort_by_key(|(group, recent_rank, open_order, _)| (*group, *recent_rank, *open_order));
        items
            .into_iter()
            .enumerate()
            .map(|(order, (_, _, _, summary))| BufferSwitchItem {
                buffer_id: summary.id,
                label: summary.display_path,
                active: summary.active,
                modified: summary.modified,
                order,
            })
            .collect()
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

    /// Open the location picker for one multi-target lookup result.
    fn open_location_picker(&mut self, kind: NavigationKind, targets: Vec<NavigationTarget>) {
        // Preserve server order so repeated queries stay stable while the user filters.
        let items = targets
            .into_iter()
            .enumerate()
            .map(|(order, target)| LocationPickerItem { target, order })
            .collect();
        self.prepare_picker_open();
        self.location_picker = Some(LocationPickerState::new(kind, items));
        self.mode = Mode::location_picker_empty();
    }

    /// Open the diagnostics picker for the active buffer.
    fn open_diagnostics_picker(&mut self) {
        let Some(diagnostics) = self.active_file_diagnostics() else {
            self.show_status_message("No diagnostics in active buffer");
            return;
        };
        if diagnostics.diagnostics.is_empty() {
            self.show_status_message("No diagnostics in active buffer");
            return;
        }
        let items = diagnostics
            .diagnostics
            .iter()
            .cloned()
            .enumerate()
            .map(|(diagnostic_index, diagnostic)| DiagnosticPickerItem {
                diagnostic_index,
                diagnostic,
            })
            .collect();
        self.prepare_picker_open();
        self.diagnostic_picker = Some(DiagnosticPickerState::new(items));
        self.mode = Mode::diagnostic_picker_empty();
    }

    /// Open the code-action picker for one ordered code-action result list.
    fn open_code_action_picker(
        &mut self,
        source_buffer_id: usize,
        request_edit_generation: u64,
        actions: Vec<LspCodeAction>,
    ) {
        // Preserve server order so repeated filter edits keep the picker stable.
        let items = actions
            .into_iter()
            .enumerate()
            .map(|(order, action)| CodeActionPickerItem { action, order })
            .collect();
        self.prepare_picker_open();
        self.code_action_picker = Some(CodeActionPickerState::new(
            source_buffer_id,
            request_edit_generation,
            items,
        ));
        self.mode = Mode::code_action_picker_empty();
    }

    /// Close the location picker without applying a selection.
    fn close_location_picker(&mut self) {
        self.location_picker = None;
        self.mode = Mode::Normal;
    }

    /// Close the diagnostics picker without applying a selection.
    fn close_diagnostics_picker(&mut self) {
        self.diagnostic_picker = None;
        self.mode = Mode::Normal;
    }

    /// Close the code-action picker without applying a selection.
    fn close_code_action_picker(&mut self) {
        self.code_action_picker = None;
        self.mode = Mode::Normal;
    }

    /// Confirm the current location-picker selection, if one is available.
    fn confirm_location_picker_selection(&mut self) {
        let Some(target) = self
            .location_picker
            .as_ref()
            .and_then(LocationPickerState::selected_target)
        else {
            return;
        };
        let target = target.clone();

        self.close_location_picker();
        self.goto_navigation_target(&target);
    }

    /// Confirm the current diagnostics-picker selection, if one is available.
    fn confirm_diagnostics_picker_selection(&mut self) {
        let Some(selected_index) = self
            .diagnostic_picker
            .as_ref()
            .and_then(DiagnosticPickerState::selected_index)
        else {
            return;
        };
        self.close_diagnostics_picker();
        self.goto_active_buffer_diagnostic(selected_index);
    }

    /// Confirm the current code-action picker selection, if one is available.
    fn confirm_code_action_picker_selection(&mut self) {
        // Capture the selected action before closing the picker because closing it
        // resets the modal state that owns the stored selection.
        let Some((action, source_buffer_id, request_edit_generation)) =
            self.code_action_picker.as_ref().and_then(|picker| {
                picker.selected_action().cloned().map(|action| {
                    (
                        action,
                        picker.source_buffer_id(),
                        picker.request_edit_generation(),
                    )
                })
            })
        else {
            return;
        };
        self.close_code_action_picker();
        self.apply_selected_code_action(&action, source_buffer_id, request_edit_generation);
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

    /// Poll background picker and completion work plus any due swap refreshes.
    pub(crate) fn poll_background_tasks(&mut self) {
        if let Some(query) = self.mode.file_picker_string().map(str::to_string)
            && let Some(picker) = &mut self.file_picker
        {
            let FilePickerPollResult {
                changed: _picker_changed,
                status_message,
            } = picker.poll(&query);
            if let Some(status_message) = status_message {
                self.show_status_message(status_message);
            }
        }

        self.poll_completion_background_tasks();
        self.flush_due_swap_refresh();
    }

    /// Clear transient modal UI so a newly-opened picker owns the overlay state.
    fn prepare_picker_open(&mut self) {
        self.dismiss_completion_session(false);
        self.clear_pending_modal_state();
        self.pending_overwrite = None;
        self.pending_soft_read_only_save = None;
        self.pending_quit_confirmation = None;
        self.pending_swap_recovery = None;
        self.pending_buffer_close_confirmation = false;
        self.status_message = None;
        self.buffer_switch = None;
        self.clear_picker_and_hover_state();
    }

    /// Return the current diagnostics snapshot for the active buffer, if any.
    fn active_file_diagnostics(&self) -> Option<&LspFileDiagnostics> {
        let file_path = normalize_lookup_path(&self.file_path)?;
        let diagnostics = self.lsp_diagnostics.get(&file_path)?;
        match diagnostics.version {
            Some(version) if version >= self.lsp_document_version => Some(diagnostics),
            // Versionless diagnostics stay visible only while no newer local edits
            // are waiting to be synchronized to the language server.
            None if self.pending_lsp_changes.is_empty() && self.pending_lsp_sync_at.is_none() => {
                Some(diagnostics)
            }
            _ => None,
        }
    }

    /// Return the strongest diagnostic starting on `line`, if any.
    pub(crate) fn line_diagnostic_severity(&self, line: usize) -> Option<LspDiagnosticSeverity> {
        self.active_file_diagnostics()?.line_severity(line)
    }

    /// Return the strongest diagnostic covering `line` and `character`, if any.
    pub(crate) fn diagnostic_severity_at_position(
        &self,
        line: usize,
        character: usize,
    ) -> Option<LspDiagnosticSeverity> {
        self.active_file_diagnostics()?
            .severity_at_position(line, character)
    }

    /// Return the strongest active-buffer diagnostic covering the cursor, if any.
    pub(crate) fn cursor_diagnostic(&self) -> Option<&crate::lsp::LspDiagnostic> {
        let cursor = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        self.active_file_diagnostics()?
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.covers_position(cursor.line, cursor.character))
            .min_by_key(|diagnostic| diagnostic.severity.sort_rank())
    }

    /// Return all active-buffer diagnostics covering the cursor in stable display order.
    fn cursor_code_action_diagnostics(&self) -> Vec<LspDiagnostic> {
        let cursor = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        self.active_file_diagnostics()
            .map(|diagnostics| {
                diagnostics
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.covers_position(cursor.line, cursor.character))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return the active-buffer error and warning counts.
    pub(crate) fn active_diagnostic_counts(&self) -> DiagnosticCounts {
        let Some(diagnostics) = self.active_file_diagnostics() else {
            return DiagnosticCounts::default();
        };
        // The status line summarizes only the severities that need immediate attention.
        DiagnosticCounts {
            errors: diagnostics
                .diagnostics
                .iter()
                .filter(|diagnostic| matches!(diagnostic.severity, LspDiagnosticSeverity::Error))
                .count(),
            warnings: diagnostics
                .diagnostics
                .iter()
                .filter(|diagnostic| matches!(diagnostic.severity, LspDiagnosticSeverity::Warning))
                .count(),
        }
    }

    /// Return whether one incoming diagnostics update should be ignored.
    ///
    /// Returns `true` when the incoming snapshot is older than the stored
    /// snapshot or would clear newer diagnostics with an empty mixed-transport
    /// or unversioned update, and `false` when the update should replace the
    /// stored snapshot.
    fn should_ignore_lsp_diagnostics_update(
        existing: &LspFileDiagnostics,
        update: &LspFileDiagnostics,
    ) -> bool {
        crate::lsp::diagnostics::should_ignore_update(existing, update)
    }

    /// Apply one diagnostics update routed from the LSP manager.
    ///
    /// Returns `true` when the updated file matches the active buffer and should
    /// trigger a visible redraw, and `false` when the update only changed an
    /// inactive file's stored diagnostics.
    pub(crate) fn apply_lsp_file_diagnostics(&mut self, update: LspFileDiagnostics) -> bool {
        let is_active_file = normalize_lookup_path(&self.file_path)
            .is_some_and(|file_path| file_path == update.file_path);
        if let Some(existing) = self.lsp_diagnostics.get(&update.file_path) {
            // Ignore stale clears before mutating the stored diagnostics snapshot.
            if Self::should_ignore_lsp_diagnostics_update(existing, &update) {
                return false;
            }
        }
        if update.is_empty() {
            self.lsp_diagnostics.remove(&update.file_path);
        } else {
            self.lsp_diagnostics
                .insert(update.file_path.clone(), update);
        }
        if matches!(self.mode, Mode::DiagnosticPicker(_)) {
            self.close_diagnostics_picker();
        }
        is_active_file
    }

    /// Jump to the next diagnostic in the active buffer.
    fn goto_next_diagnostic(&mut self) {
        let Some(diagnostics) = self.active_file_diagnostics() else {
            self.show_status_message("No diagnostics in active buffer");
            return;
        };
        let cursor = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        let Some(index) = diagnostics.next_index_after(cursor.line, cursor.character) else {
            self.show_status_message("No next diagnostic");
            return;
        };
        self.goto_active_buffer_diagnostic(index);
    }

    /// Jump to the previous diagnostic in the active buffer.
    fn goto_prev_diagnostic(&mut self) {
        let Some(diagnostics) = self.active_file_diagnostics() else {
            self.show_status_message("No diagnostics in active buffer");
            return;
        };
        let cursor = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        let Some(index) = diagnostics.previous_index_before(cursor.line, cursor.character) else {
            self.show_status_message("No previous diagnostic");
            return;
        };
        self.goto_active_buffer_diagnostic(index);
    }

    /// Move the active cursor to one diagnostic in the current buffer.
    fn goto_active_buffer_diagnostic(&mut self, diagnostic_index: usize) {
        let Some((line, character, label)) = self
            .active_file_diagnostics()
            .and_then(|diagnostics| diagnostics.diagnostics.get(diagnostic_index))
            .map(|diagnostic| {
                (
                    diagnostic.range.start.line,
                    diagnostic.range.start.character,
                    diagnostic.display_label(),
                )
            })
        else {
            self.show_status_message("No diagnostics in active buffer");
            return;
        };
        if !self.record_jump_origin_for_destination(&self.file_path.clone(), line, character) {
            self.show_status_message(label);
            return;
        }
        self.move_cursor_to_lsp_position(line, character);
        self.show_status_message(label);
    }

    /// Return whether the app loop should poll for asynchronous picker updates.
    ///
    /// Returns `true` when file-picker work, a queued app-layer request, or a
    /// pending swap flush needs a timed wakeup, and `false` when the editor can
    /// stay on the blocking input path.
    pub(crate) fn needs_background_poll(&self) -> bool {
        self.file_picker
            .as_ref()
            .is_some_and(FilePickerState::is_scanning)
            || self.pending_request.is_some()
            || self.pending_async_completion.is_some()
            || self.pending_lsp_completion.is_some()
            || self.pending_lsp_signature_help.is_some()
            || self.active_lsp_completion.is_some()
            || self.active_signature_help_lookup.is_some()
            || self.pending_swap_refresh_at.is_some()
            || self.pending_lsp_sync_at.is_some()
    }

    /// Queue one navigation lookup for the current cursor position.
    fn request_navigation(&mut self, kind: NavigationKind) {
        if self.file_path.as_os_str().is_empty() {
            self.show_status_message(kind.unavailable_file_message());
            return;
        }
        self.clear_hover_and_rename_state();
        let token = self.lookup_tokens.next();
        self.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind,
            token,
            document_version: self.lsp_document_version,
        });
        self.pending_request = Some(EditorRequest::LspNavigation(kind));
        self.show_status_message(kind.resolving_message());
    }

    /// Queue one hover lookup for the current cursor position.
    fn request_hover(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            self.show_status_message("No file is open for hover");
            return;
        }
        self.clear_hover_and_rename_state();
        let token = self.lookup_tokens.next();
        self.active_hover_lookup = Some(ActiveHoverLookup {
            token,
            document_version: self.lsp_document_version,
        });
        self.pending_request = Some(EditorRequest::LspHover);
        self.show_status_message("Resolving hover...");
    }

    /// Queue one rename lookup for the current cursor position.
    fn request_rename(&mut self, new_name: String) {
        if self.file_path.as_os_str().is_empty() {
            self.show_status_message("No file is open for rename");
            return;
        }
        self.clear_hover_and_rename_state();
        self.active_navigation_lookup = None;
        let token = self.lookup_tokens.next();
        self.active_rename_lookup = Some(ActiveRenameLookup {
            token,
            document_version: self.lsp_document_version,
            request_edit_generation: self.next_edit_generation,
            new_name: new_name.clone(),
        });
        self.pending_request = Some(EditorRequest::LspRename(new_name));
        self.show_status_message("Renaming symbol...");
    }

    /// Queue one code-action lookup for the current cursor context.
    fn request_code_actions(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            self.show_status_message("No file is open for code actions");
            return;
        }
        // Code actions depend on the current buffer snapshot, so older hover,
        // rename, and navigation requests must be discarded before queuing one.
        self.clear_hover_and_rename_state();
        self.active_navigation_lookup = None;
        let token = self.lookup_tokens.next();
        self.active_code_action_lookup = Some(ActiveCodeActionLookup {
            token,
            document_version: self.lsp_document_version,
            request_edit_generation: self.next_edit_generation,
        });
        self.pending_request = Some(EditorRequest::LspCodeAction);
        self.show_status_message("Loading code actions...");
    }

    /// Take the active-buffer snapshot required for one due proactive document sync.
    pub(crate) fn take_due_document_sync_snapshot(
        &mut self,
        now: Instant,
    ) -> Option<crate::lsp::DocumentSyncSnapshot> {
        let deadline = self.pending_lsp_sync_at?;
        if deadline > now {
            return None;
        }
        let Some(file_path) = normalize_lookup_path(&self.file_path) else {
            self.pending_lsp_sync_at = None;
            return None;
        };
        self.pending_lsp_sync_at = None;
        // Clone the rope so the app loop can synchronize the latest buffer text
        // without borrowing the live editor state across the LSP session call.
        Some(crate::lsp::DocumentSyncSnapshot {
            buffer_id: self.active_buffer_id,
            document_version: self.lsp_document_version,
            file_path,
            text: self.buffer.clone_rope(),
            changes: self.pending_lsp_changes.clone(),
        })
    }

    /// Build the active-buffer snapshot required for one completed save notification.
    pub(crate) fn document_save_snapshot(
        &self,
        target_path: &Path,
        update_file_path: bool,
    ) -> Option<crate::lsp::DocumentSaveSnapshot> {
        let file_path = normalize_lookup_path(target_path)?;
        // Save-as retains the old URI so the LSP layer can close it before the
        // new path becomes the live document owner inside the session.
        let previous_file_path = update_file_path
            .then(|| normalize_lookup_path(&self.file_path))
            .flatten()
            .filter(|previous| previous != &file_path);
        Some(crate::lsp::DocumentSaveSnapshot {
            buffer_id: self.active_buffer_id,
            document_version: self.lsp_document_version,
            previous_file_path,
            file_path,
            text: self.buffer.clone_rope_for_save(),
            changes: self.pending_lsp_changes.clone(),
        })
    }

    /// Build the active-buffer snapshot required for one background navigation lookup.
    pub(crate) fn navigation_request_snapshot(&self) -> Option<NavigationRequestSnapshot> {
        let lookup = self.active_navigation_lookup?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let position = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        // Clone the rope so the worker thread keeps an immutable snapshot without
        // forcing one eager `String` allocation for every queued lookup.
        Some(NavigationRequestSnapshot {
            buffer_id: self.active_buffer_id,
            lookup_token: lookup.token,
            document_version: lookup.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            // Dirty buffers resend one whole-document snapshot so every lookup
            // sees the latest text even while background sync work is still
            // catching up on the same modified buffer state.
            force_full_sync: self.buffer.is_modified(),
            changes: self.pending_lsp_changes.clone(),
            line: position.line,
            character: position.character,
        })
    }

    /// Build the active-buffer snapshot required for one background hover lookup.
    pub(crate) fn hover_request_snapshot(&self) -> Option<HoverRequestSnapshot> {
        let lookup = self.active_hover_lookup?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let position = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        Some(HoverRequestSnapshot {
            buffer_id: self.active_buffer_id,
            lookup_token: lookup.token,
            document_version: lookup.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            force_full_sync: self.buffer.is_modified(),
            changes: self.pending_lsp_changes.clone(),
            line: position.line,
            character: position.character,
        })
    }

    /// Build the active-buffer snapshot required for one background rename lookup.
    pub(crate) fn rename_request_snapshot(&self, new_name: &str) -> Option<RenameRequestSnapshot> {
        let lookup = self.active_rename_lookup.as_ref()?;
        // The pending request stores the user-entered target name separately, so
        // reject snapshots from an older prompt once a newer rename replaced it.
        if lookup.new_name != new_name {
            return None;
        }
        let file_path = normalize_lookup_path(&self.file_path)?;
        let position = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        Some(RenameRequestSnapshot {
            buffer_id: self.active_buffer_id,
            lookup_token: lookup.token,
            document_version: lookup.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            force_full_sync: self.buffer.is_modified(),
            changes: self.pending_lsp_changes.clone(),
            line: position.line,
            character: position.character,
            new_name: new_name.to_string(),
        })
    }

    /// Build the active-buffer snapshot required for one background code-action lookup.
    pub(crate) fn code_action_request_snapshot(&self) -> Option<CodeActionRequestSnapshot> {
        let lookup = self.active_code_action_lookup?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let position = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        // Ordex has no normal-mode selection yet, so request actions for the
        // current cursor position and include any diagnostics covering it.
        Some(CodeActionRequestSnapshot {
            buffer_id: self.active_buffer_id,
            lookup_token: lookup.token,
            document_version: lookup.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            force_full_sync: self.buffer.is_modified(),
            changes: self.pending_lsp_changes.clone(),
            range: LspRange {
                start: position,
                end: position,
            },
            diagnostics: self.cursor_code_action_diagnostics(),
        })
    }

    /// Return one due automatic LSP completion snapshot ready for background dispatch.
    pub(crate) fn take_due_completion_request_snapshot(
        &mut self,
    ) -> Option<CompletionRequestSnapshot> {
        let pending = self.pending_lsp_completion.as_ref()?;
        if pending.due_at > Instant::now() {
            return None;
        }
        let pending = self.pending_lsp_completion.take()?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let position = self.char_idx_to_lsp_position(pending.request.cursor_char_idx());
        self.active_lsp_completion = Some(ActiveLspCompletion {
            request: pending.request.clone(),
            document_version: pending.document_version,
        });
        Some(CompletionRequestSnapshot {
            buffer_id: self.active_buffer_id,
            document_version: pending.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            force_full_sync: self.buffer.is_modified(),
            changes: self.pending_lsp_changes.clone(),
            line: position.line,
            character: position.character,
            request: pending.request,
            popup_anchor_char_idx: pending.popup_anchor_char_idx,
            trigger_text: pending.trigger_text,
        })
    }

    /// Return one due automatic LSP signature-help snapshot ready for background dispatch.
    pub(crate) fn take_due_signature_help_request_snapshot(
        &mut self,
    ) -> Option<SignatureHelpRequestSnapshot> {
        let pending = self.pending_lsp_signature_help.as_ref()?;
        if pending.due_at > Instant::now() {
            // The app loop polls frequently while insert-mode background work is
            // pending, so requests must stay queued until their debounce window
            // expires instead of dispatching one server round-trip per keypress.
            return None;
        }
        let pending = self.pending_lsp_signature_help.take()?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let position = self.char_idx_to_lsp_position(self.cursor.to_char_index(&self.buffer));
        self.active_signature_help_lookup = Some(ActiveSignatureHelpLookup {
            token: pending.lookup_token,
            document_version: pending.document_version,
            cursor_char_idx: pending.cursor_char_idx,
            anchor_char_idx: pending.anchor_char_idx,
        });
        Some(SignatureHelpRequestSnapshot {
            buffer_id: self.active_buffer_id,
            lookup_token: pending.lookup_token,
            document_version: pending.document_version,
            file_path,
            text: self.buffer.clone_rope(),
            force_full_sync: self.buffer.is_modified(),
            changes: self.pending_lsp_changes.clone(),
            line: position.line,
            character: position.character,
            anchor_char_idx: pending.anchor_char_idx,
            trigger_text: pending.trigger_text,
            is_retrigger: pending.is_retrigger,
        })
    }

    /// Apply one foreground document-sync result to the active buffer bookkeeping.
    ///
    /// Returns `true` when the sync completion changes whether active-buffer
    /// diagnostics are visible, and `false` when the completion is invisible to
    /// the current screen state.
    pub(crate) fn apply_document_sync_outcome(&mut self, outcome: DocumentSyncOutcome) -> bool {
        let redraw_before = self.active_file_diagnostics().is_some();
        match outcome {
            DocumentSyncOutcome::Synced {
                buffer_id,
                document_version,
            } => self.finish_document_sync(buffer_id, document_version, true),
            DocumentSyncOutcome::Unsupported {
                buffer_id,
                document_version,
            } => self.finish_document_sync(buffer_id, document_version, true),
            // Failed attempts only clear the one-shot request flag so later
            // lookups can still fall back to a full-text sync for correctness.
            DocumentSyncOutcome::Failed {
                buffer_id,
                document_version,
            } => self.finish_document_sync(buffer_id, document_version, false),
        }
        redraw_before != self.active_file_diagnostics().is_some()
    }

    /// Apply one completed navigation lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result was accepted and changed editor-visible
    /// state, and `false` when it was stale or no longer mapped to an open buffer.
    pub(crate) fn apply_navigation_lookup_result(
        &mut self,
        result: NavigationLookupResult,
    ) -> bool {
        // Results are keyed by the originating buffer id, so switch back to that
        // buffer if it is still open before checking whether the lookup is stale.
        if self.active_buffer_id != result.buffer_id {
            self.switch_to_buffer_id(result.buffer_id);
        }
        if self.active_buffer_id != result.buffer_id {
            return false;
        }
        let Some(lookup) = self.active_navigation_lookup else {
            return false;
        };
        if lookup.kind != result.kind
            || lookup.token != result.lookup_token
            || lookup.document_version != result.document_version
        {
            return false;
        }
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        self.active_navigation_lookup = None;
        match result.outcome {
            NavigationLookupOutcome::Single(target) => self.goto_navigation_target(&target),
            // Multiple locations need an explicit user choice before any jump happens.
            NavigationLookupOutcome::Multiple(targets) => {
                self.open_location_picker(result.kind, targets)
            }
            NavigationLookupOutcome::NotFound => {
                self.show_status_message(result.kind.not_found_message())
            }
            NavigationLookupOutcome::UnsupportedFile(message)
            | NavigationLookupOutcome::UnsupportedProject(message)
            | NavigationLookupOutcome::Unavailable(message)
            | NavigationLookupOutcome::Error(message) => self.show_status_message(message),
        }
        true
    }

    /// Apply one completed completion lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result refreshed the visible popup, and `false`
    /// when the result was stale, unsupported, or invisible to the current UI.
    pub(crate) fn apply_completion_lookup_result(
        &mut self,
        result: CompletionLookupResult,
    ) -> bool {
        if self.active_buffer_id != result.buffer_id {
            self.switch_to_buffer_id(result.buffer_id);
        }
        if self.active_buffer_id != result.buffer_id {
            return false;
        }
        let Some(active) = self.active_lsp_completion.as_ref() else {
            return false;
        };
        if active.document_version != result.document_version || active.request != result.request {
            return false;
        }
        self.active_lsp_completion = None;
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        let Some(identity) = self.current_completion_identity_for_request(&result.request) else {
            self.completion_session = None;
            return true;
        };
        if !result
            .request
            .matches_identity(self.active_buffer_id, &identity)
        {
            return false;
        }
        let CompletionLookupOutcome::Found(items) = result.outcome else {
            return false;
        };
        let candidates = self.lsp_completion_candidates(&result.request, items);
        let Some(updated_session) = refresh_session(
            &self.completion_sources,
            &self.buffer,
            result.request,
            result.popup_anchor_char_idx,
            &candidates,
        ) else {
            self.completion_session = None;
            return true;
        };
        let mut active_session = self.completion_session.take();
        match &mut active_session {
            Some(session) if session.matches_identity(self.active_buffer_id, &identity) => {
                let preview_start = session.current_replace_start_char_idx();
                let preview_end = session.replacement_end_char_idx();
                let preview_changed = session.replace_candidates(updated_session.candidates);
                if preview_changed {
                    let replacement = session.current_text();
                    self.replace_completion_range(preview_start, preview_end, replacement);
                }
                self.completion_session = active_session;
            }
            _ => {
                self.completion_session = Some(updated_session);
            }
        }
        true
    }

    /// Convert one batch of LSP completion items into popup candidates.
    fn lsp_completion_candidates(
        &self,
        request: &CompletionRequest,
        items: Vec<LspCompletionItem>,
    ) -> Vec<CompletionCandidate> {
        let source_rank = self
            .completion_sources
            .source_priority(CompletionSourceId::Lsp);
        let normalized_prefix = request.normalized_match_prefix();
        items
            .into_iter()
            .enumerate()
            .filter_map(|(rank, item)| {
                let normalized_match_text = crate::completion::normalize_text(&item.filter_text);
                if !normalized_prefix.is_empty()
                    && !normalized_match_text.starts_with(normalized_prefix)
                {
                    return None;
                }
                let (replace_start_char_idx, replace_end_char_idx) = item
                    .replace_range
                    .as_ref()
                    .and_then(|range| self.lsp_completion_replace_range(range))
                    .unwrap_or((request.replace_start_char_idx(), request.cursor_char_idx()));
                (replace_end_char_idx >= replace_start_char_idx).then_some(CompletionCandidate {
                    source_id: CompletionSourceId::Lsp,
                    insert_text: item.insert_text,
                    popup_label: item.label,
                    popup_detail: item.kind.map(|kind| kind.detail_label()),
                    normalized_match_text,
                    replace_start_char_idx,
                    replace_end_char_idx,
                    rank: source_rank.saturating_mul(1000).saturating_add(rank),
                })
            })
            .collect()
    }

    /// Return async candidates from the current popup that still match `request`.
    ///
    /// This lets the popup keep showing compatible async entries while a newer
    /// request is still running. The retained entries are only a temporary UI
    /// bridge: they avoid hide/show flicker during typing, but the next async
    /// response still rebuilds the full session from the authoritative source
    /// results for the refreshed request.
    fn retained_async_candidates(&self, request: &CompletionRequest) -> Vec<CompletionCandidate> {
        let Some(session) = self.completion_session.as_ref() else {
            return Vec::new();
        };
        // The retained list is only valid while the edited replacement span has
        // not moved, so typing continues to narrow the same popup contents.
        if session.request().replace_start_char_idx() != request.replace_start_char_idx() {
            return Vec::new();
        }
        session
            .candidates
            .iter()
            .filter(|candidate| self.completion_sources.source_is_async(candidate.source_id))
            .filter(|candidate| {
                request.normalized_match_prefix().is_empty()
                    || candidate
                        .normalized_match_text
                        .starts_with(request.normalized_match_prefix())
            })
            .cloned()
            .collect()
    }

    /// Convert one LSP replacement range into buffer character indices.
    fn lsp_completion_replace_range(&self, range: &LspRange) -> Option<(usize, usize)> {
        let start = self.lsp_position_to_char_idx(range.start)?;
        let end = self.lsp_position_to_char_idx(range.end)?;
        Some((start, end))
    }

    /// Apply one completed hover lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result was accepted and changed editor-visible
    /// state, and `false` when it was stale or no longer mapped to an open buffer.
    pub(crate) fn apply_hover_lookup_result(&mut self, result: HoverLookupResult) -> bool {
        if self.active_buffer_id != result.buffer_id {
            return false;
        }
        // Hover is anchored to the active buffer only, so once the user leaves
        // that buffer the result is stale even if the worker eventually completes.
        let Some(lookup) = self.active_hover_lookup else {
            return false;
        };
        if lookup.token != result.lookup_token || lookup.document_version != result.document_version
        {
            return false;
        }
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        self.active_hover_lookup = None;
        match result.outcome {
            HoverLookupOutcome::Found(text) => {
                // Successful hover results replace the previous popup content and
                // intentionally clear the message line so the popup is the single
                // user-facing representation of the lookup.
                self.hover_popup = Some(HoverPopup::new(&text));
                self.status_message = None;
            }
            HoverLookupOutcome::NotFound => {
                // Empty hover results dismiss stale popup content so the editor
                // never suggests the old symbol still owns the visible hover text.
                self.hover_popup = None;
                self.show_status_message("No hover information found");
            }
            HoverLookupOutcome::UnsupportedFile(message)
            | HoverLookupOutcome::UnsupportedProject(message)
            | HoverLookupOutcome::Unavailable(message)
            | HoverLookupOutcome::Error(message) => {
                // Transport and capability failures also clear the popup because
                // error feedback must not leave an older hover overlay onscreen.
                self.hover_popup = None;
                self.show_status_message(message);
            }
        }
        true
    }

    /// Apply one completed signature-help lookup result and report whether UI state changed.
    ///
    /// Returns `true` when the result was accepted and changed editor-visible
    /// state, and `false` when it was stale or no longer mapped to an open buffer.
    pub(crate) fn apply_signature_help_lookup_result(
        &mut self,
        result: SignatureHelpLookupResult,
    ) -> bool {
        let missing_server_binary = result.missing_server_binary;
        if self.active_buffer_id != result.buffer_id {
            return false;
        }
        let Some(lookup) = self.active_signature_help_lookup else {
            return false;
        };
        if lookup.token != result.lookup_token || lookup.document_version != result.document_version
        {
            return false;
        }
        self.finish_document_sync(result.buffer_id, result.document_version, true);
        self.active_signature_help_lookup = None;
        match result.outcome {
            SignatureHelpLookupOutcome::Found(help) => {
                self.signature_help_popup =
                    Some(SignatureHelpPopup::new(&help, lookup.anchor_char_idx));
                self.status_message = None;
            }
            SignatureHelpLookupOutcome::NotFound => {
                self.signature_help_popup = None;
            }
            SignatureHelpLookupOutcome::UnsupportedFile(message)
            | SignatureHelpLookupOutcome::UnsupportedProject(message)
            | SignatureHelpLookupOutcome::Error(message) => {
                self.signature_help_popup = None;
                self.show_status_message(message);
            }
            SignatureHelpLookupOutcome::Unavailable(message) => {
                self.signature_help_popup = None;
                if !missing_server_binary {
                    self.show_status_message(message);
                }
            }
        }
        true
    }

    /// Finish one document-sync attempt for the matching buffer version when still current.
    fn finish_document_sync(
        &mut self,
        buffer_id: usize,
        document_version: i32,
        clear_changes: bool,
    ) {
        if self.active_buffer_id == buffer_id {
            Self::finish_buffer_sync_state(
                &mut self.pending_lsp_sync_at,
                &mut self.pending_lsp_changes,
                self.lsp_document_version,
                document_version,
                clear_changes,
            );
            return;
        }
        let Some(mut buffer) = self.buffer_manager.take_inactive_by_id(buffer_id) else {
            return;
        };
        Self::finish_buffer_sync_state(
            &mut buffer.pending_lsp_sync_at,
            &mut buffer.pending_lsp_changes,
            buffer.lsp_document_version,
            document_version,
            clear_changes,
        );
        self.buffer_manager.store_inactive(buffer);
    }

    /// Clear one buffer's queued sync state when the completed version still matches.
    fn finish_buffer_sync_state(
        pending_lsp_sync_at: &mut Option<Instant>,
        pending_lsp_changes: &mut Vec<LspTextChange>,
        current_version: i32,
        document_version: i32,
        clear_changes: bool,
    ) {
        if current_version != document_version {
            return;
        }
        // Background sync can finish after the user switches buffers, so stale
        // completions are rejected by version rather than by active-buffer identity.
        *pending_lsp_sync_at = None;
        if clear_changes {
            pending_lsp_changes.clear();
        }
    }

    /// Open one navigation target and move the cursor to the returned position.
    fn goto_navigation_target(&mut self, target: &NavigationTarget) {
        if !self.record_jump_origin_for_destination(
            &target.file_path,
            target.line,
            target.character,
        ) {
            return;
        }
        let open_path = current_dir_relative_path(&target.file_path);
        // Open the returned file first so every follow-up cursor calculation uses
        // the destination buffer rather than stale line counts from the source file.
        if let Err(error) = self.open_buffer(open_path.as_ref()) {
            self.show_status_message(format!(
                "Failed to open navigation target \"{}\": {error}",
                open_path.display()
            ));
            return;
        }
        // Clamp the reported position because servers can target EOF or the start
        // of an empty line, both of which must remain valid cursor locations.
        self.cursor = self.clamped_normal_cursor(target.line, target.character);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Move the active cursor to one zero-based LSP position inside the current buffer.
    fn move_cursor_to_lsp_position(&mut self, line: usize, character: usize) {
        self.cursor = self.clamped_normal_cursor(line, character);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
    }

    /// Dismiss the visible hover popup and reject any in-flight hover result.
    fn dismiss_hover(&mut self) {
        self.hover_popup = None;
        self.active_hover_lookup = None;
    }

    /// Dismiss the visible signature-help popup and reject any in-flight result.
    fn dismiss_signature_help(&mut self) {
        self.signature_help_popup = None;
        self.active_signature_help_lookup = None;
        self.pending_lsp_signature_help = None;
    }

    /// Clear transient file/location picker state together with any hover overlay.
    fn clear_picker_and_hover_state(&mut self) {
        if let Some(picker) = &mut self.file_picker {
            picker.cancel();
        }
        self.file_picker = None;
        self.location_picker = None;
        self.diagnostic_picker = None;
        self.code_action_picker = None;
        self.dismiss_hover();
        self.dismiss_signature_help();
    }

    /// Dismiss the active completion session, optionally restoring the typed prefix.
    fn dismiss_completion_session(&mut self, restore_prefix: bool) {
        let Some(session) = self.completion_session.take() else {
            self.cancel_pending_async_completion();
            self.cancel_pending_lsp_completion();
            return;
        };
        self.cancel_pending_async_completion();
        self.cancel_pending_lsp_completion();
        if !restore_prefix {
            return;
        }

        // Restoring the original prefix reuses the same buffer-edit path as previews.
        self.replace_completion_range(
            session.request().replace_start_char_idx(),
            session.replacement_end_char_idx(),
            session.request().original_text(),
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

        let Some(identity) = self.current_completion_identity() else {
            self.dismiss_completion_session(false);
            return;
        };
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        // Keep the popup anchored to the location where this completion run began
        // so the suggestion box does not jitter rightward as the prefix grows.
        let popup_anchor_char_idx = self
            .completion_session
            .as_ref()
            .map_or(cursor_char_idx, |session| session.popup_anchor_char_idx);
        if self.completion_request_matches_identity(&identity)
            || self.pending_completion_matches_identity(&identity)
        {
            return;
        }
        let request_generation = self.next_completion_generation();
        let request = CompletionRequest::new(self.active_buffer_id, request_generation, identity);
        let retained_async_candidates = self.retained_async_candidates(&request);

        let mut refreshed_session = refresh_session(
            &self.completion_sources,
            &self.buffer,
            request.clone(),
            popup_anchor_char_idx,
            &retained_async_candidates,
        );
        if let (Some(previous), Some(ref mut refreshed)) =
            (self.completion_session.as_ref(), refreshed_session.as_mut())
            && self.should_preserve_completion_popup_metrics(&request)
        {
            refreshed.preserve_popup_metrics_from(previous);
        }
        // Keep the visible path popup onscreen while the new async directory
        // scan is still in flight. Without this, typing one more character can
        // briefly drop the popup before the refreshed path results arrive.
        let preserve_existing_path_popup = request.is_file_path()
            && refreshed_session.is_none()
            && self.completion_session.as_ref().is_some_and(|session| {
                session.request().is_file_path()
                    && session.selected_index.is_none()
                    && session.request().replace_start_char_idx()
                        == request.replace_start_char_idx()
            });
        if !preserve_existing_path_popup {
            self.completion_session = refreshed_session;
        }
        self.restart_async_completion(request.clone(), popup_anchor_char_idx);
        self.schedule_lsp_completion(request, popup_anchor_char_idx);
    }

    /// Refresh signature help from the current insert cursor context when one session is active.
    fn refresh_signature_help_session(&mut self) {
        if self.mode != Mode::Insert || self.file_path.as_os_str().is_empty() {
            self.dismiss_signature_help();
            return;
        }
        if self.signature_help_popup.is_none()
            && self.pending_lsp_signature_help.is_none()
            && self.active_signature_help_lookup.is_none()
        {
            return;
        }
        self.schedule_signature_help_request(None, true, false);
    }

    /// Return the stable anchor for the current signature-help session.
    fn current_signature_help_anchor_char_idx(&self) -> usize {
        // Reuse the anchor already attached to the current popup lifecycle so
        // retriggers keep the popup column stable while the user keeps typing.
        if let Some(pending) = self.pending_lsp_signature_help.as_ref() {
            return pending.anchor_char_idx;
        }
        if let Some(active) = self.active_signature_help_lookup.as_ref() {
            return active.anchor_char_idx;
        }
        if let Some(popup) = self.signature_help_popup.as_ref() {
            return popup.anchor_char_idx;
        }
        self.cursor.to_char_index(&self.buffer)
    }

    /// Build the active completion identity for the current insert cursor, if any.
    fn current_completion_identity(&self) -> Option<CompletionRequestIdentity> {
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        build_request_identity(&self.buffer, self.active_named_file_path(), cursor_char_idx)
    }

    /// Build the active completion identity compatible with one accepted request.
    fn current_completion_identity_for_request(
        &self,
        request: &CompletionRequest,
    ) -> Option<CompletionRequestIdentity> {
        if let Some(identity) = self.current_completion_identity() {
            return Some(identity);
        }
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        (request.match_prefix().is_empty() && cursor_char_idx == request.cursor_char_idx())
            .then(|| build_lsp_trigger_request_identity(cursor_char_idx))
    }

    /// Return whether the visible completion session already matches `identity`.
    fn completion_request_matches_identity(&self, identity: &CompletionRequestIdentity) -> bool {
        self.completion_session
            .as_ref()
            .is_some_and(|session| session.matches_identity(self.active_buffer_id, identity))
    }

    /// Return whether one asynchronous completion request already matches `identity`.
    fn pending_completion_matches_identity(&self, identity: &CompletionRequestIdentity) -> bool {
        self.pending_async_completion
            .as_ref()
            .is_some_and(|pending| pending.matches_identity(self.active_buffer_id, identity))
            || self.pending_lsp_completion.as_ref().is_some_and(|pending| {
                pending
                    .request
                    .matches_identity(self.active_buffer_id, identity)
            })
            || self.active_lsp_completion.as_ref().is_some_and(|pending| {
                pending
                    .request
                    .matches_identity(self.active_buffer_id, identity)
            })
    }

    /// Restart one asynchronous local completion source for `request` when it applies.
    fn restart_async_completion(
        &mut self,
        request: CompletionRequest,
        popup_anchor_char_idx: usize,
    ) {
        self.cancel_pending_async_completion();
        self.pending_async_completion =
            PendingAsyncCompletion::spawn(&self.completion_sources, request, popup_anchor_char_idx);
    }

    /// Cancel any in-flight asynchronous local completion request.
    fn cancel_pending_async_completion(&mut self) {
        if let Some(pending) = &mut self.pending_async_completion {
            pending.cancel();
        }
        self.pending_async_completion = None;
    }

    /// Cancel any queued automatic signature-help lookup.
    fn cancel_pending_signature_help(&mut self) {
        self.pending_lsp_signature_help = None;
    }

    /// Queue or clear one automatic LSP completion request for the current insert context.
    fn schedule_lsp_completion(
        &mut self,
        request: CompletionRequest,
        popup_anchor_char_idx: usize,
    ) {
        self.cancel_pending_lsp_completion();
        if !self.completion_sources.lsp_enabled()
            || request.is_file_path()
            || self.file_path.as_os_str().is_empty()
        {
            return;
        }
        self.pending_lsp_completion = Some(PendingLspCompletion {
            request,
            popup_anchor_char_idx,
            document_version: self.lsp_document_version,
            due_at: Instant::now() + Self::LSP_COMPLETION_DEBOUNCE_DELAY,
            trigger_text: None,
        });
    }

    /// Queue or clear one automatic signature-help request for the current insert context.
    fn schedule_signature_help_request(
        &mut self,
        trigger_text: Option<String>,
        is_retrigger: bool,
        immediate: bool,
    ) {
        self.cancel_pending_signature_help();
        if self.mode != Mode::Insert || self.file_path.as_os_str().is_empty() {
            return;
        }
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        let anchor_char_idx = self.current_signature_help_anchor_char_idx();
        self.pending_lsp_signature_help = Some(PendingLspSignatureHelp {
            lookup_token: self.lookup_tokens.next(),
            document_version: self.lsp_document_version,
            cursor_char_idx,
            anchor_char_idx,
            due_at: if immediate {
                Instant::now()
            } else {
                Instant::now() + Self::LSP_SIGNATURE_HELP_DEBOUNCE_DELAY
            },
            trigger_text,
            is_retrigger,
        });
    }

    /// Cancel any queued or active automatic LSP completion request.
    fn cancel_pending_lsp_completion(&mut self) {
        self.pending_lsp_completion = None;
        self.active_lsp_completion = None;
    }

    /// Return the trailing text before `cursor_char_idx`, capped to `max_chars`.
    fn completion_text_before_cursor(
        &self,
        cursor_char_idx: usize,
        max_chars: usize,
    ) -> Option<String> {
        if max_chars == 0 || cursor_char_idx == 0 {
            return None;
        }
        let start_char_idx = cursor_char_idx.saturating_sub(max_chars);
        let recent_text = self.buffer.slice_string(start_char_idx, cursor_char_idx);
        (!recent_text.is_empty()).then_some(recent_text)
    }

    /// Drain one completed asynchronous local completion request and merge its candidates.
    ///
    /// Returns `true` when the visible completion popup changed, and `false`
    /// when no asynchronous completion update was accepted on this poll tick.
    fn poll_completion_background_tasks(&mut self) -> bool {
        let Some(mut pending) = self.pending_async_completion.take() else {
            return false;
        };
        let poll_result = pending.poll();
        if !poll_result.finished {
            self.pending_async_completion = Some(pending);
            return false;
        }
        let Some(candidates) = poll_result.candidates else {
            return false;
        };
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        let Some(identity) =
            build_request_identity(&self.buffer, self.active_named_file_path(), cursor_char_idx)
        else {
            return false;
        };
        if !pending
            .request()
            .matches_identity(self.active_buffer_id, &identity)
        {
            return false;
        }

        // Accepted async results rebuild the full candidate list so sorting and
        // deduplication stay identical between sync-only and merged refreshes.
        let Some(updated_session) = refresh_session(
            &self.completion_sources,
            &self.buffer,
            pending.request().clone(),
            pending.popup_anchor_char_idx(),
            &candidates,
        ) else {
            self.completion_session = None;
            return true;
        };
        let mut active_session = self.completion_session.take();
        match &mut active_session {
            Some(session) if session.matches_identity(self.active_buffer_id, &identity) => {
                let preview_start = session.current_replace_start_char_idx();
                let preview_end = session.replacement_end_char_idx();
                let preview_changed = session.replace_candidates(updated_session.candidates);
                if preview_changed {
                    let replacement = session.current_text().to_string();
                    self.replace_completion_range(preview_start, preview_end, &replacement);
                }
                self.completion_session = active_session;
            }
            _ => {
                self.completion_session = Some(updated_session);
            }
        }
        true
    }

    /// Return whether popup dimensions should stay reserved while async sources are pending.
    fn should_preserve_completion_popup_metrics(&self, request: &CompletionRequest) -> bool {
        request.is_file_path()
            || (!request.is_file_path()
                && self.completion_sources.lsp_enabled()
                && !self.file_path.as_os_str().is_empty())
    }

    /// Return whether one trigger-only LSP completion already targets `cursor_char_idx`.
    ///
    /// Returns `true` when a queued or active trigger-only request already owns
    /// the current cursor position, and `false` when a newer trigger context
    /// should replace any older completion request.
    fn has_trigger_only_lsp_completion_at(&self, cursor_char_idx: usize) -> bool {
        if let Some(pending) = self.pending_lsp_completion.as_ref()
            && pending.request.match_prefix().is_empty()
            && pending.request.cursor_char_idx() == cursor_char_idx
        {
            return true;
        }
        if let Some(active) = self.active_lsp_completion.as_ref()
            && active.request.match_prefix().is_empty()
            && active.request.cursor_char_idx() == cursor_char_idx
        {
            return true;
        }
        false
    }

    /// Return the current file path for one queued LSP completion that may be trigger-driven.
    pub(crate) fn pending_lsp_trigger_file_path(&self) -> Option<PathBuf> {
        self.pending_lsp_completion.as_ref()?;
        normalize_lookup_path(&self.file_path)
    }

    /// Return the current file path and trailing text for one queued LSP completion.
    pub(crate) fn pending_lsp_trigger_context(
        &self,
        max_trigger_chars: usize,
    ) -> Option<(PathBuf, String)> {
        let pending = self.pending_lsp_completion.as_ref()?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let recent_text = self
            .completion_text_before_cursor(pending.request.cursor_char_idx(), max_trigger_chars)?;
        Some((file_path, recent_text))
    }

    /// Return the current file path when trigger-only completion may apply.
    pub(crate) fn lsp_trigger_candidate_file_path(&self) -> Option<PathBuf> {
        if self.mode != Mode::Insert || self.completion_session.is_some() {
            return None;
        }
        normalize_lookup_path(&self.file_path)
    }

    /// Return the current trailing text when no regular completion request exists.
    pub(crate) fn lsp_trigger_candidate_context(
        &self,
        max_trigger_chars: usize,
    ) -> Option<(PathBuf, String)> {
        if self.mode != Mode::Insert || self.completion_session.is_some() {
            return None;
        }
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        if self.has_trigger_only_lsp_completion_at(cursor_char_idx) {
            return None;
        }
        let recent_text = self.completion_text_before_cursor(cursor_char_idx, max_trigger_chars)?;
        Some((normalize_lookup_path(&self.file_path)?, recent_text))
    }

    /// Record the matched LSP trigger text for one queued completion request.
    pub(crate) fn set_pending_lsp_trigger_text(&mut self, trigger_text: &str) {
        let Some(pending) = self.pending_lsp_completion.as_mut() else {
            return;
        };
        pending.trigger_text = Some(trigger_text.to_string());
    }

    /// Queue one trigger-only LSP completion for the just-typed trigger text.
    pub(crate) fn queue_lsp_trigger_completion(&mut self, trigger_text: &str) {
        if self.mode != Mode::Insert
            || self.completion_session.is_some()
            || self.file_path.as_os_str().is_empty()
        {
            return;
        }
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        if self.has_trigger_only_lsp_completion_at(cursor_char_idx) {
            return;
        }
        let Some(recent_text) =
            self.completion_text_before_cursor(cursor_char_idx, trigger_text.chars().count())
        else {
            return;
        };
        if !recent_text.ends_with(trigger_text) {
            return;
        }
        self.cancel_pending_lsp_completion();
        let request_generation = self.next_completion_generation();
        // Trigger-only requests replace zero characters because the separator is
        // already in the buffer and the server should append members after it.
        let request = CompletionRequest::new(
            self.active_buffer_id,
            request_generation,
            build_lsp_trigger_request_identity(cursor_char_idx),
        );
        self.pending_lsp_completion = Some(PendingLspCompletion {
            request,
            popup_anchor_char_idx: cursor_char_idx,
            document_version: self.lsp_document_version,
            due_at: Instant::now(),
            trigger_text: Some(trigger_text.to_string()),
        });
    }

    /// Return whether one trigger-only signature-help request already targets `cursor_char_idx`.
    fn has_trigger_only_signature_help_at(&self, cursor_char_idx: usize) -> bool {
        if let Some(pending) = self.pending_lsp_signature_help.as_ref()
            && pending.trigger_text.is_some()
            && pending.cursor_char_idx == cursor_char_idx
        {
            return true;
        }
        if let Some(active) = self.active_signature_help_lookup.as_ref()
            && self.signature_help_popup.is_none()
            && active.cursor_char_idx == cursor_char_idx
        {
            return true;
        }
        false
    }

    /// Return the current file path for one queued signature-help request that may be trigger-driven.
    pub(crate) fn pending_signature_help_trigger_file_path(&self) -> Option<PathBuf> {
        // Trigger probing only makes sense when a queued signature-help request
        // already exists; the guard preserves the Option-based early return shape
        // before we normalize the current file path for that queued work item.
        self.pending_lsp_signature_help.as_ref()?;
        normalize_lookup_path(&self.file_path)
    }

    /// Return the current file path and trailing text for one queued signature-help request.
    pub(crate) fn pending_signature_help_trigger_context(
        &self,
        max_trigger_chars: usize,
    ) -> Option<(PathBuf, String)> {
        let pending = self.pending_lsp_signature_help.as_ref()?;
        let file_path = normalize_lookup_path(&self.file_path)?;
        let recent_text =
            self.completion_text_before_cursor(pending.cursor_char_idx, max_trigger_chars)?;
        Some((file_path, recent_text))
    }

    /// Return the current file path when trigger-only signature help may apply.
    pub(crate) fn signature_help_trigger_candidate_file_path(&self) -> Option<PathBuf> {
        if self.mode != Mode::Insert || self.file_path.as_os_str().is_empty() {
            return None;
        }
        normalize_lookup_path(&self.file_path)
    }

    /// Return the current trailing text when no regular signature-help request exists.
    pub(crate) fn signature_help_trigger_candidate_context(
        &self,
        max_trigger_chars: usize,
    ) -> Option<(PathBuf, String)> {
        if self.mode != Mode::Insert || self.file_path.as_os_str().is_empty() {
            return None;
        }
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        if self.has_trigger_only_signature_help_at(cursor_char_idx) {
            return None;
        }
        let recent_text = self.completion_text_before_cursor(cursor_char_idx, max_trigger_chars)?;
        Some((normalize_lookup_path(&self.file_path)?, recent_text))
    }

    /// Record the matched signature-help trigger text for one queued request.
    pub(crate) fn set_pending_signature_help_trigger_text(&mut self, trigger_text: &str) {
        let Some(pending) = self.pending_lsp_signature_help.as_mut() else {
            return;
        };
        pending.trigger_text = Some(trigger_text.to_string());
    }

    /// Queue one trigger-only signature-help request for the just-typed trigger text.
    pub(crate) fn queue_lsp_trigger_signature_help(&mut self, trigger_text: &str) {
        if self.mode != Mode::Insert || self.file_path.as_os_str().is_empty() {
            return;
        }
        let cursor_char_idx = self.cursor.to_char_index(&self.buffer);
        if self.has_trigger_only_signature_help_at(cursor_char_idx) {
            return;
        }
        let Some(recent_text) =
            self.completion_text_before_cursor(cursor_char_idx, trigger_text.chars().count())
        else {
            return;
        };
        if !recent_text.ends_with(trigger_text) {
            return;
        }
        self.schedule_signature_help_request(Some(trigger_text.to_string()), false, true);
    }

    /// Promote one queued signature-help request to run immediately.
    pub(crate) fn promote_pending_signature_help(&mut self) {
        if let Some(pending) = &mut self.pending_lsp_signature_help {
            pending.due_at = Instant::now();
        }
    }

    /// Promote one queued LSP completion to run immediately.
    pub(crate) fn promote_pending_lsp_completion(&mut self) {
        if let Some(pending) = &mut self.pending_lsp_completion {
            pending.due_at = Instant::now();
        }
    }

    /// Move the completion selection if a session is active.
    ///
    /// Returns `true` when an active completion session consumed the movement,
    /// and `false` when no completion session was available to update.
    fn move_completion_selection(&mut self, direction: CompletionDirection) -> bool {
        let Some(mut session) = self.completion_session.take() else {
            return false;
        };
        let start_char_idx = session.current_replace_start_char_idx();
        let end_char_idx = session.replacement_end_char_idx();
        session.move_selection(direction);
        let replacement = session.current_text().to_string();
        self.replace_completion_range(start_char_idx, end_char_idx, &replacement);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.completion_session = Some(session);
        true
    }

    /// Shift visible and queued completion popup anchors after one insertion.
    fn shift_completion_popup_anchors_for_insert(
        &mut self,
        insert_char_idx: usize,
        inserted_char_count: usize,
    ) {
        // Every queued popup owner tracks the same logical anchor position, so
        // text inserted before that point must shift all saved indices together.
        if let Some(session) = &mut self.completion_session {
            session.shift_popup_anchor_for_insert(insert_char_idx, inserted_char_count);
        }
        if let Some(pending) = &mut self.pending_async_completion {
            pending.shift_popup_anchor_for_insert(insert_char_idx, inserted_char_count);
        }
        if let Some(pending) = &mut self.pending_lsp_completion {
            shift_popup_anchor_for_insert(
                &mut pending.popup_anchor_char_idx,
                insert_char_idx,
                inserted_char_count,
            );
        }
    }

    /// Shift visible and queued completion popup anchors after one removal.
    fn shift_completion_popup_anchors_for_removal(&mut self, start_char: usize, end_char: usize) {
        // Deletions can collapse the saved anchor into the removed span, so each
        // completion owner must be updated before any popup renders again.
        if let Some(session) = &mut self.completion_session {
            session.shift_popup_anchor_for_removal(start_char, end_char);
        }
        if let Some(pending) = &mut self.pending_async_completion {
            pending.shift_popup_anchor_for_removal(start_char, end_char);
        }
        if let Some(pending) = &mut self.pending_lsp_completion {
            shift_popup_anchor_for_removal(
                &mut pending.popup_anchor_char_idx,
                start_char,
                end_char,
            );
        }
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
            | Action::InsertNewline
            | Action::IndentCurrentLine
            | Action::DedentCurrentLine => self.refresh_completion_session(),
            Action::CompletionSelectUp | Action::CompletionSelectDown => {}
            Action::MoveWordForward
            | Action::MoveBigWordForward
            | Action::MoveWordBackward
            | Action::MoveBigWordBackward
            | Action::MoveWordEnd
            | Action::MoveBigWordEnd
            | Action::MoveWordEndBackward
            | Action::MoveBigWordEndBackward
            | Action::MoveParagraphForward
            | Action::MoveParagraphBackward
            | Action::MoveLineEnd
            | Action::MoveFirstNonBlank
            | Action::MoveDownFirstNonBlank
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
            | Action::JumpOlder
            | Action::JumpNewer
            | Action::MatchBracket
            | Action::EnterInsertMode
            | Action::VisualInsertBlockStart
            | Action::VisualAppendBlockEnd
            | Action::EnterVisualMode
            | Action::EnterVisualLineMode
            | Action::EnterVisualBlockMode
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
            | Action::GotoReferences
            | Action::GotoFileUnderCursor
            | Action::GotoFileUnderCursorAtPosition
            | Action::GotoAlternateFile
            | Action::GotoLastModification
            | Action::ShowHover
            | Action::OpenCodeActions
            | Action::OpenDiagnosticsPicker
            | Action::NextDiagnostic
            | Action::PrevDiagnostic
            | Action::PromptRenameSymbol
            | Action::BeginMacroRecord
            | Action::BeginMacroPlayback
            | Action::ExitToNormalMode
            | Action::HideSearchHighlighting
            | Action::SearchNext
            | Action::SearchPrevious
            | Action::Undo
            | Action::Redo
            | Action::SaveCurrentFile
            | Action::SaveCurrentFileAndQuit
            | Action::UpdateCurrentFileAndQuit
            | Action::RequestFullRedraw
            | Action::ToggleCaseAtCursor
            | Action::DeleteToLineEnd
            | Action::ChangeToLineEnd
            | Action::IncrementNextNumber
            | Action::DecrementNextNumber
            | Action::JoinLines
            | Action::BeginReplaceChar
            | Action::SearchWordUnderCursor
            | Action::DeleteCharAtCursor
            | Action::DeleteSelection
            | Action::IndentSelection
            | Action::ReindentSelection
            | Action::DedentSelection
            | Action::ChangeSelection
            | Action::YankSelection
            | Action::YankCurrentLine
            | Action::PasteAfterCursor
            | Action::PasteBeforeCursor
            | Action::BeginDeleteOperator
            | Action::BeginChangeOperator
            | Action::BeginYankOperator
            | Action::BeginIndentOperator
            | Action::BeginReindentOperator
            | Action::BeginDedentOperator
            | Action::ExecuteCommand
            | Action::CancelCommand
            | Action::PromptHistoryPrev
            | Action::PromptHistoryNext
            | Action::PromptHistoryPrevFull
            | Action::PromptHistoryNextFull
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

    /// Sync signature-help visibility after one action updates the editor state.
    fn sync_signature_help_after_action(&mut self, action: Action) {
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
            | Action::InsertNewline
            | Action::IndentCurrentLine
            | Action::DedentCurrentLine => self.refresh_signature_help_session(),
            Action::CompletionSelectUp | Action::CompletionSelectDown => {}
            Action::MoveWordForward
            | Action::MoveBigWordForward
            | Action::MoveWordBackward
            | Action::MoveBigWordBackward
            | Action::MoveWordEnd
            | Action::MoveBigWordEnd
            | Action::MoveWordEndBackward
            | Action::MoveBigWordEndBackward
            | Action::MoveParagraphForward
            | Action::MoveParagraphBackward
            | Action::MoveLineEnd
            | Action::MoveFirstNonBlank
            | Action::MoveDownFirstNonBlank
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
            | Action::JumpOlder
            | Action::JumpNewer
            | Action::MatchBracket
            | Action::EnterInsertMode
            | Action::VisualInsertBlockStart
            | Action::VisualAppendBlockEnd
            | Action::EnterVisualMode
            | Action::EnterVisualLineMode
            | Action::EnterVisualBlockMode
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
            | Action::GotoReferences
            | Action::GotoFileUnderCursor
            | Action::GotoFileUnderCursorAtPosition
            | Action::GotoAlternateFile
            | Action::GotoLastModification
            | Action::ShowHover
            | Action::OpenCodeActions
            | Action::OpenDiagnosticsPicker
            | Action::NextDiagnostic
            | Action::PrevDiagnostic
            | Action::PromptRenameSymbol
            | Action::BeginMacroRecord
            | Action::BeginMacroPlayback
            | Action::ExitToNormalMode
            | Action::HideSearchHighlighting
            | Action::SearchNext
            | Action::SearchPrevious
            | Action::Undo
            | Action::Redo
            | Action::SaveCurrentFile
            | Action::SaveCurrentFileAndQuit
            | Action::UpdateCurrentFileAndQuit
            | Action::RequestFullRedraw
            | Action::ToggleCaseAtCursor
            | Action::DeleteToLineEnd
            | Action::ChangeToLineEnd
            | Action::IncrementNextNumber
            | Action::DecrementNextNumber
            | Action::JoinLines
            | Action::BeginReplaceChar
            | Action::SearchWordUnderCursor
            | Action::DeleteCharAtCursor
            | Action::DeleteSelection
            | Action::IndentSelection
            | Action::ReindentSelection
            | Action::DedentSelection
            | Action::ChangeSelection
            | Action::YankSelection
            | Action::YankCurrentLine
            | Action::PasteAfterCursor
            | Action::PasteBeforeCursor
            | Action::BeginDeleteOperator
            | Action::BeginChangeOperator
            | Action::BeginYankOperator
            | Action::BeginIndentOperator
            | Action::BeginReindentOperator
            | Action::BeginDedentOperator
            | Action::ExecuteCommand
            | Action::CancelCommand
            | Action::PromptHistoryPrev
            | Action::PromptHistoryNext
            | Action::PromptHistoryPrevFull
            | Action::PromptHistoryNextFull
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
            | Action::MoveInputWordRight => {
                if self.mode != Mode::Insert {
                    self.dismiss_signature_help();
                }
            }
        }
    }

    /// Convert one buffer character index into zero-based LSP line/UTF-16 coordinates.
    fn char_idx_to_lsp_position(&self, char_idx: usize) -> LspPosition {
        let line = self
            .buffer
            .char_to_line(char_idx.min(self.buffer.chars_count()));
        let line_start = self.buffer.line_to_char(line);
        let column_text = self
            .buffer
            .slice_string(line_start, char_idx.min(self.buffer.chars_count()));
        // LSP columns count UTF-16 code units, so multibyte UTF-8 scalar values
        // need one last pass across the line prefix before the position is sent.
        LspPosition {
            line,
            character: column_text.chars().map(char::len_utf16).sum(),
        }
    }

    /// Convert one zero-based LSP line/UTF-16 position into a buffer character index.
    fn lsp_position_to_char_idx(&self, position: LspPosition) -> Option<usize> {
        if position.line >= self.buffer.lines_count() {
            return None;
        }
        let line_start = self.buffer.line_to_char(position.line);
        let line_end = self.buffer.line_to_char(position.line + 1);
        let line_text = self.buffer.slice_string(line_start, line_end);
        let mut utf16_offset = 0;
        let mut char_offset = 0;
        // Completion edits arrive in UTF-16 units, so walk the line until the
        // requested code-unit offset is reached or the line ends.
        for character in line_text.chars() {
            if utf16_offset >= position.character {
                break;
            }
            utf16_offset += character.len_utf16();
            char_offset += 1;
        }
        Some(line_start + char_offset)
    }

    /// Queue one editor mutation for later LSP synchronization.
    fn queue_lsp_change(&mut self, change: LspTextChange) {
        self.next_edit_generation = self.next_edit_generation.saturating_add(1);
        self.last_edit_generation = self.next_edit_generation;
        self.lsp_document_version = self.lsp_document_version.saturating_add(1);
        self.pending_lsp_changes.push(change);
        self.pending_lsp_sync_at = (!self.file_path.as_os_str().is_empty())
            .then(|| Instant::now() + Self::LSP_SYNC_DEBOUNCE_DELAY);
    }

    /// Insert `text` at `char_idx` and notify the syntax engine about the edit.
    fn insert_buffer_text(&mut self, char_idx: usize, text: &str) {
        self.ensure_insert_history_transaction();
        if !self.replaying_history {
            self.record_history_insert(char_idx, text);
        }
        let inserted_char_count = text.chars().count();
        let position = self.char_idx_to_lsp_position(char_idx);
        let start_line = self
            .buffer
            .char_to_line(char_idx.min(self.buffer.chars_count()));
        // Popup anchors are stored as absolute buffer indices, so they must move
        // with any text inserted before the saved anchor position.
        self.shift_completion_popup_anchors_for_insert(char_idx, inserted_char_count);
        if let Some(selection) = self.last_visual_selection.as_mut() {
            selection.shift_for_insert(char_idx, inserted_char_count);
        }
        self.buffer.insert(char_idx, text);
        // Insertions replace an empty range at the pre-edit cursor position.
        self.queue_lsp_change(LspTextChange {
            range: Some(LspRange {
                start: position,
                end: position,
            }),
            text: text.to_string(),
        });
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
        let start = self.char_idx_to_lsp_position(start_char);
        let end = self.char_idx_to_lsp_position(end_char);
        let start_line = self.buffer.char_to_line(start_char);
        let old_end_line = self.removal_old_end_line(start_char, end_char);
        // Removing text before the popup anchor would otherwise leave it pointing
        // at a later buffer position, potentially even beyond the current line.
        self.shift_completion_popup_anchors_for_removal(start_char, end_char);
        if let Some(selection) = self.last_visual_selection.as_mut() {
            selection.shift_for_removal(start_char, end_char);
        }
        self.buffer.remove(start_char, end_char);
        // Deletions send the pre-edit span with an empty replacement string.
        self.queue_lsp_change(LspTextChange {
            range: Some(LspRange { start, end }),
            text: String::new(),
        });
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

    /// Recompute the active buffer's effective read-only indicator.
    fn refresh_active_read_only_state(&mut self) {
        self.read_only =
            self.soft_read_only || buffers::path_is_read_only(self.file_path.as_path());
    }

    /// Return what cancel should do for the active swap prompt.
    fn pending_swap_cancel_action(&self) -> PendingSwapCancelAction {
        if self.buffer_manager.has_single_buffer() {
            PendingSwapCancelAction::Quit
        } else {
            PendingSwapCancelAction::CloseBuffer
        }
    }

    /// Build one prompt for a stale swap file that may be recovered or discarded.
    fn build_recovery_swap_prompt(
        &self,
        recovery: swap::SwapRecovery,
        recreate_handle_on_discard: bool,
    ) -> PendingSwapPrompt {
        PendingSwapPrompt {
            prompt: "Recovery swap found. [r] recover [d] discard [c] cancel".to_string(),
            recovered_buffer: recovery.buffer,
            swap_path: recovery.swap_path,
            kind: PendingSwapPromptKind::Recovery,
            cancel_action: self.pending_swap_cancel_action(),
            recreate_handle_on_discard,
        }
    }

    /// Build one prompt for a swap file that likely belongs to another instance.
    fn build_conflicting_swap_prompt(&self, conflict: swap::SwapConflict) -> PendingSwapPrompt {
        let explanation = match conflict.state {
            swap::SwapConflictState::RunningLocally => {
                format!("Ordex pid {} owns this swap.", conflict.meta.pid)
            }
            swap::SwapConflictState::OtherHost => {
                format!(
                    "Swap came from ordex pid {}@{}.",
                    conflict.meta.pid, conflict.meta.hostname
                )
            }
            swap::SwapConflictState::UnknownLocalStatus => {
                format!(
                    "Ordex could not verify pid {}@{}.",
                    conflict.meta.pid, conflict.meta.hostname
                )
            }
        };

        PendingSwapPrompt {
            prompt: format!(
                "{explanation} [o] read-only [e] edit [r] recover [d] discard [c] cancel"
            ),
            recovered_buffer: conflict.buffer,
            swap_path: conflict.swap_path,
            kind: PendingSwapPromptKind::Conflict,
            cancel_action: self.pending_swap_cancel_action(),
            recreate_handle_on_discard: self.file_path.as_os_str().is_empty()
                || !self.active_path_is_swap_excluded(),
        }
    }

    /// Load swap state for the active buffer and establish current ownership.
    fn load_swap_state_for_active_buffer(&mut self) {
        self.swap = None;
        self.pending_swap_refresh_at = None;
        self.suppress_swap_creation = false;
        self.soft_read_only = false;
        self.refresh_active_read_only_state();
        self.pending_swap_recovery = None;
        let active_path = normalize_lookup_path(&self.file_path);
        let is_excluded = active_path
            .as_ref()
            .is_some_and(|path| self.path_is_swap_excluded(path));
        let existing_swap = if let Some(path) = active_path.as_ref() {
            swap::inspect_existing_swap(path)
        } else if self.file_path.as_os_str().is_empty() {
            swap::inspect_unnamed_swap()
        } else {
            return;
        };

        match existing_swap {
            Ok(Some(swap::ExistingSwap::Recoverable(recovery))) => {
                self.pending_swap_recovery = Some(self.build_recovery_swap_prompt(
                    recovery,
                    self.file_path.as_os_str().is_empty() || !is_excluded,
                ));
            }
            Ok(Some(swap::ExistingSwap::Conflicting(conflict))) => {
                self.pending_swap_recovery = Some(self.build_conflicting_swap_prompt(conflict));
            }
            Ok(None) => {
                if active_path.is_some()
                    && !is_excluded
                    && let Err(error) = self.create_active_swap_handle()
                {
                    self.show_swap_unavailable_error(&error);
                }
            }
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
        if self.suppress_swap_creation {
            self.pending_swap_refresh_at = None;
            return;
        }
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
        if self.suppress_swap_creation {
            return false;
        }
        if let Some(swap) = self.swap.as_mut() {
            if let Err(error) = swap.refresh(&self.buffer) {
                self.show_swap_unavailable_error(&error);
                return true;
            }
            return false;
        }

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
            VisualKind::Block => YankKind::Block,
        }
    }

    /// Copy one buffer range into the unnamed register with the requested shape.
    fn store_yank_range(&mut self, selection: SelectionRange, kind: YankKind) {
        self.yank_buffer = Some(YankBuffer {
            text: self.buffer.slice_string(selection.start, selection.end),
            kind,
        });
    }

    /// Copy one block selection into the unnamed register as one blockwise payload.
    fn store_yank_block(&mut self, selection: BlockSelection) {
        self.yank_buffer = Some(YankBuffer {
            text: selection.yank_lines(&self.buffer).join("\n"),
            kind: YankKind::Block,
        });
    }

    /// Delete one buffer range after first copying it into the unnamed register.
    fn delete_range_into_yank_buffer(&mut self, selection: SelectionRange, kind: YankKind) {
        self.store_yank_range(selection, kind);
        if selection.end > selection.start {
            self.remove_buffer_range(selection.start, selection.end);
        }
    }

    /// Delete one block selection after first copying it into the unnamed register.
    fn delete_block_into_yank_buffer(&mut self, selection: BlockSelection) {
        let segments = selection.segments(&self.buffer);
        self.store_yank_block(selection);

        // Remove from bottom to top so later rows do not invalidate the earlier
        // absolute character indices captured before the first deletion.
        for segment in segments.iter().rev() {
            self.remove_buffer_range(segment.start, segment.end);
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
            YankKind::Block => self.paste_blockwise(&payload.text, position),
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

    /// Ensure the active buffer contains `target_line` before one blockwise put.
    fn ensure_buffer_has_line(&mut self, target_line: usize) {
        while self.buffer.lines_count() <= target_line {
            self.insert_buffer_text(self.buffer.chars_count(), "\n");
        }
    }

    /// Paste one blockwise payload before or after the current cursor column.
    fn paste_blockwise(&mut self, text: &str, position: PastePosition) {
        let base_column = match position {
            PastePosition::After if self.buffer.line_len(self.cursor.line()) > 0 => {
                self.cursor.column().saturating_add(1)
            }
            PastePosition::Before | PastePosition::After => self.cursor.column(),
        };

        // A blockwise put inserts each payload row into the corresponding logical
        // line, clamping short lines to their current end instead of padding them
        // or flattening the payload into one contiguous span.
        for (offset, row_text) in text.split('\n').enumerate() {
            let target_line = self.cursor.line().saturating_add(offset);
            self.ensure_buffer_has_line(target_line);
            if row_text.is_empty() {
                continue;
            }
            let insert_column = base_column.min(self.buffer.line_len(target_line));
            let insert_idx = self.buffer.line_to_char(target_line) + insert_column;
            self.insert_buffer_text(insert_idx, row_text);
        }

        let mut cursor = Cursor::new(self.cursor.line(), base_column);
        cursor.clamp_to_buffer_normal(&self.buffer);
        self.cursor = cursor;
    }

    /// Yank the current visual selection into the unnamed register and leave Visual mode.
    fn yank_visual_selection(&mut self) {
        let Some(selection) = self.visual_selection() else {
            return;
        };
        match selection {
            VisualSelection::Character(range) => {
                self.store_yank_range(range, YankKind::Character);
            }
            VisualSelection::Line(range) => {
                self.store_yank_range(range, YankKind::Line);
            }
            VisualSelection::Block(selection) => self.store_yank_block(selection),
        }
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
        self.touch_pending_auto_indent();
        let char_idx = self.cursor.to_char_index(&self.buffer);
        self.insert_buffer_text(char_idx, &c.to_string());
        self.cursor.move_right(&self.buffer);
        self.auto_dedent_current_line_after_insert();
    }

    /// Insert one newline at the cursor and keep syntax state in sync.
    fn insert_newline(&mut self) {
        self.insert_newline_with_auto_indent();
    }

    /// Open a new line below the cursor and enter insert mode.
    fn open_line_below(&mut self) {
        self.open_line_below_with_auto_indent();
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
        self.open_line_above_with_auto_indent();
    }

    /// Delete one character backward in insert mode.
    fn delete_char_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx > 0 {
            self.touch_pending_auto_indent();
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
            self.touch_pending_auto_indent();
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

        self.touch_pending_auto_indent();
        let word_start = find_prev_word_start_with_style(&self.buffer, char_idx, WordStyle::Small);
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

        self.touch_pending_auto_indent();
        // Get the start of the current line in char index
        let line_start = self.buffer.line_to_char(line);
        let char_idx = self.cursor.to_char_index(&self.buffer);

        self.cursor.set_column(0);
        self.remove_buffer_range(line_start, char_idx);
    }

    fn delete_input_char(&mut self) {
        self.edit_prompt_input(|mode| mode.pop_char());
    }

    fn delete_input_char_forward(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_char_forward());
    }

    fn delete_input_word_backward(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_word_backward());
    }

    /// Delete one prompt word forward while keeping the input cursor anchored.
    fn delete_input_word_forward(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_word_forward());
    }

    fn delete_input_to_start(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_to_start());
    }

    fn delete_input_to_end(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_to_end());
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

    /// Append one typed character to the active command or search prompt.
    fn append_prompt_char(&mut self, c: char) {
        self.edit_prompt_input(|mode| mode.append_char(c));
    }

    /// Replace the active command or search prompt with one recalled history entry.
    fn replace_active_prompt_text(&mut self, text: String) {
        self.mode.replace_input_text(text);
        self.sync_prompt_previews();
    }

    /// Return the active command or search prompt text.
    fn active_prompt_text(&self) -> Option<&str> {
        self.mode
            .command_string()
            .or_else(|| self.mode.search_string())
    }

    /// Return which prompt history should react to the active mode.
    fn active_prompt_history_kind(&self) -> Option<PromptHistoryKind> {
        match self.mode {
            Mode::Command(_) => Some(PromptHistoryKind::Command),
            Mode::Search(_) => Some(PromptHistoryKind::Search),
            _ => None,
        }
    }

    /// Reset the active prompt-history traversal session, if any.
    fn reset_active_prompt_history(&mut self) {
        let Some(kind) = self.active_prompt_history_kind() else {
            return;
        };
        self.prompt_history.reset_traversal(kind);
    }

    /// Apply one prompt edit and clear traversal state only when the text changed.
    fn edit_prompt_input<F>(&mut self, edit: F)
    where
        F: FnOnce(&mut Mode),
    {
        let before = self.active_prompt_text().map(str::to_string);
        edit(&mut self.mode);
        if before.as_deref() != self.active_prompt_text() {
            self.reset_active_prompt_history();
        }
        self.sync_prompt_previews();
    }

    /// Enter command mode with one provided initial prompt text.
    fn enter_command_prompt(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.mode = if text.is_empty() {
            Mode::command_empty()
        } else {
            Mode::command_with_text(text)
        };
        self.prompt_history
            .reset_traversal(PromptHistoryKind::Command);
        self.sync_prompt_previews();
    }

    /// Enter search mode with one empty prompt.
    fn enter_search_prompt(&mut self) {
        self.mode = Mode::search_empty();
        self.prompt_history
            .reset_traversal(PromptHistoryKind::Search);
        self.sync_prompt_previews();
    }

    /// Refresh every prompt-scoped preview surface after one prompt edit.
    fn sync_prompt_previews(&mut self) {
        self.refresh_substitute_preview();
        self.sync_search_highlights_for_viewport();
    }

    /// Leave command or search mode while clearing transient prompt-only UI state.
    fn cancel_prompt_input(&mut self) {
        self.pending_search_count = None;
        self.reset_active_prompt_history();
        self.mode = Mode::Normal;
        self.clear_substitute_preview(true);
        self.sync_search_highlights_for_viewport();
    }

    /// Recall one older prompt-history entry for the active prompt.
    fn recall_prompt_history_previous(&mut self, scope: PromptHistoryScope) {
        let Some(kind) = self.active_prompt_history_kind() else {
            return;
        };
        let Some(current_input) = self.active_prompt_text().map(str::to_string) else {
            return;
        };
        if let Some(entry) = self.prompt_history.previous(kind, &current_input, scope) {
            self.replace_active_prompt_text(entry);
        }
    }

    /// Recall one newer prompt-history entry for the active prompt.
    fn recall_prompt_history_next(&mut self, scope: PromptHistoryScope) {
        let Some(kind) = self.active_prompt_history_kind() else {
            return;
        };
        let Some(current_input) = self.active_prompt_text().map(str::to_string) else {
            return;
        };
        if let Some(entry) = self.prompt_history.next(kind, &current_input, scope) {
            self.replace_active_prompt_text(entry);
        }
    }

    /// Delete the active visual selection and optionally enter insert mode.
    fn delete_visual_selection(&mut self, enter_insert: bool) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };

        let action = if enter_insert {
            SelectionRepeatAction::Change
        } else {
            SelectionRepeatAction::Delete
        };
        self.prepare_visual_repeat(saved_selection, action);
        self.last_visual_selection = Some(saved_selection);
        self.apply_delete_visual_selection(selection, enter_insert);
    }

    /// Delete one explicit Visual selection and optionally enter Insert mode afterward.
    fn apply_delete_visual_selection(&mut self, selection: VisualSelection, enter_insert: bool) {
        match selection {
            VisualSelection::Character(selection) => {
                self.apply_delete_selection(selection, VisualKind::Character, enter_insert);
            }
            VisualSelection::Line(selection) => {
                self.apply_delete_selection(selection, VisualKind::Line, enter_insert);
            }
            VisualSelection::Block(selection) => {
                self.apply_delete_block_selection(selection, enter_insert);
            }
        }
    }

    /// Delete one explicit selection and optionally enter Insert mode afterward.
    fn apply_delete_selection(
        &mut self,
        selection: SelectionRange,
        kind: VisualKind,
        enter_insert: bool,
    ) {
        self.begin_history_transaction();
        self.delete_range_into_yank_buffer(selection, Self::yank_kind_for_visual(kind));

        // Characterwise deletion resumes at the removed span, while linewise
        // deletion snaps to column zero on the first affected line.
        self.cursor = match kind {
            VisualKind::Character => {
                let target = selection.start.min(self.buffer.chars_count());
                Cursor::from_char_index(&self.buffer, target)
            }
            VisualKind::Line => {
                let target = selection.start.min(self.buffer.chars_count());
                Cursor::new(self.buffer.char_to_line(target), 0)
            }
            VisualKind::Block => unreachable!("block selections use apply_delete_block_selection"),
        };

        if enter_insert {
            self.clear_visual_mode(Mode::Insert);
        } else {
            self.clear_visual_mode(Mode::Normal);
            self.finish_history_transaction();
        }
    }

    /// Delete one explicit block selection and optionally enter Insert mode afterward.
    fn apply_delete_block_selection(&mut self, selection: BlockSelection, enter_insert: bool) {
        self.begin_history_transaction();
        self.delete_block_into_yank_buffer(selection);

        let mut cursor = Cursor::new(selection.start_line, selection.left_column);
        if enter_insert {
            cursor.clamp_to_buffer(&self.buffer);
            self.cursor = cursor;
            self.clear_visual_mode(Mode::Insert);
        } else {
            cursor.clamp_to_buffer_normal(&self.buffer);
            self.cursor = cursor;
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
    use test_utils::{CurrentDirectoryGuard, TempFile, TempTree};

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
        let mut lsp_manager = crate::lsp::LspManager::new();
        for _ in 0..64 {
            let Some(request) = editor.take_pending_request() else {
                return;
            };
            // Test helpers still drain request chains, but the hard cap turns a
            // runaway loop into a direct failure instead of hanging the suite.
            match request {
                EditorRequest::ReloadConfig => {
                    panic!("unit tests should assert reload requests directly")
                }
                EditorRequest::WriteBuffer(write) => {
                    app::execute_deferred_write(editor, &mut lsp_manager, write)
                }
                EditorRequest::SaveSession(_)
                | EditorRequest::OpenSession(_)
                | EditorRequest::DeleteSession(_) => {
                    panic!("unit tests should assert session requests directly")
                }
                EditorRequest::LspNavigation(_)
                | EditorRequest::LspHover
                | EditorRequest::LspRename(_)
                | EditorRequest::LspCodeAction => {
                    panic!("unit tests should assert LSP requests directly")
                }
            }
        }
        panic!("flush_pending_requests exceeded 64 chained requests");
    }

    /// Apply ordered diagnostics to the active test buffer at `path`.
    fn apply_test_diagnostics(
        editor: &mut EditorState,
        path: &str,
        diagnostics: Vec<(usize, usize, usize, LspDiagnosticSeverity, &str)>,
    ) {
        editor.set_startup_path(path);
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            PathBuf::from(path),
            Some(editor.lsp_document_version),
            diagnostics
                .into_iter()
                .map(
                    |(line, start, end, severity, message)| crate::lsp::LspDiagnostic {
                        range: LspRange {
                            start: LspPosition {
                                line,
                                character: start,
                            },
                            end: LspPosition {
                                line,
                                character: end,
                            },
                        },
                        severity,
                        message: message.to_string(),
                        source: None,
                        code: None,
                    },
                )
                .collect(),
        ));
    }

    /// Build one editor with syntax detection enabled for `path`.
    fn create_syntax_editor(content: &str, path: &str) -> EditorState {
        let mut editor = create_editor_with_content(content);
        editor.file_path = PathBuf::from(path);
        editor.refresh_syntax();
        editor
    }

    /// The buffer switcher should surface the alternate file immediately after the active row.
    #[test]
    fn test_buffer_switcher_prefers_recent_named_buffers_after_active_entry() {
        let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
        let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
        let third = TempFile::with_suffix("_third.txt").expect("create third temp file");
        let mut editor = EditorState::new(24);

        editor.set_startup_path(first.path());
        let first_id = editor.active_buffer_id();
        editor
            .open_buffer(second.path())
            .expect("open second buffer");
        let second_id = editor.active_buffer_id();
        editor.open_buffer(third.path()).expect("open third buffer");
        let third_id = editor.active_buffer_id();
        editor.activate_buffer(first_id);
        editor.open_buffer_switcher();

        // Opening the picker after jumping back to the first buffer should leave
        // the alternate file at the top of the selectable rows.
        let picker = editor
            .buffer_switch
            .as_ref()
            .expect("buffer switcher should be open");
        let popup = picker.popup("", 0, 10);
        let labels = popup
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                first.path().display().to_string(),
                third.path().display().to_string(),
                second.path().display().to_string(),
            ]
        );
        assert_eq!(picker.selected_buffer_id(), Some(third_id));
        assert_ne!(third_id, second_id);
    }

    /// The buffer switcher should keep empty-query rows in MRU order even when path lengths differ.
    #[test]
    fn test_buffer_switcher_empty_query_uses_reported_mru_order() {
        let mut editor = EditorState::new(24);

        // Open the reported sequence so the latest file becomes the active buffer.
        editor.set_startup_path("src/render.rs");
        editor
            .open_buffer("src/syntax/profiles/c.rs")
            .expect("open c buffer");
        editor
            .open_buffer("src/syntax/profiles/d.rs")
            .expect("open d buffer");
        editor
            .open_buffer("src/syntax/profiles/r.rs")
            .expect("open r buffer");
        editor
            .open_buffer("src/syntax/profiles/go.rs")
            .expect("open go buffer");
        editor
            .open_buffer("src/syntax/profiles/sh.rs")
            .expect("open sh buffer");
        editor.open_buffer_switcher();

        // Empty-query picker rows should follow the same last-access history as `ga`.
        let popup = editor
            .buffer_switch
            .as_ref()
            .expect("buffer switcher should be open")
            .popup("", 0, 10);

        assert_eq!(
            popup
                .entries
                .into_iter()
                .map(|entry| entry.label)
                .collect::<Vec<_>>(),
            vec![
                "src/syntax/profiles/sh.rs".to_string(),
                "src/syntax/profiles/go.rs".to_string(),
                "src/syntax/profiles/r.rs".to_string(),
                "src/syntax/profiles/d.rs".to_string(),
                "src/syntax/profiles/c.rs".to_string(),
                "src/render.rs".to_string(),
            ]
        );
    }

    /// Newer versioned diagnostics should ignore older clearing snapshots.
    #[test]
    fn test_apply_lsp_file_diagnostics_ignores_stale_empty_update() {
        let mut editor = create_editor_with_content("fn main() {}\n");
        editor.set_startup_path("/tmp/main.rs");
        let path = PathBuf::from("/tmp/main.rs");
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            path.clone(),
            Some(3),
            vec![crate::lsp::LspDiagnostic {
                range: LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 3,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 7,
                    },
                },
                severity: LspDiagnosticSeverity::Error,
                message: "broken".to_string(),
                source: None,
                code: None,
            }],
        ));

        let changed =
            editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(path, Some(2), Vec::new()));

        assert!(!changed);
        assert_eq!(editor.active_diagnostic_counts().errors, 1);
    }

    /// Empty pull diagnostics should not clear a non-empty push snapshot for the file.
    #[test]
    fn test_apply_lsp_file_diagnostics_ignores_empty_pull_over_push_snapshot() {
        let mut editor = create_editor_with_content("fn main() {}\n");
        editor.set_startup_path("/tmp/main.rs");
        let path = PathBuf::from("/tmp/main.rs");
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            path.clone(),
            Some(3),
            vec![crate::lsp::LspDiagnostic {
                range: LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 3,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 7,
                    },
                },
                severity: LspDiagnosticSeverity::Error,
                message: "broken".to_string(),
                source: None,
                code: None,
            }],
        ));

        // rust-analyzer mixes push and pull diagnostics, so an empty pull result
        // must not wipe out a valid push snapshot that still belongs to this file.
        let changed = editor.apply_lsp_file_diagnostics(LspFileDiagnostics::with_transport(
            path,
            Some(3),
            Vec::new(),
            crate::lsp::diagnostics::DiagnosticTransport::Pull,
        ));

        assert!(!changed);
        assert_eq!(editor.active_diagnostic_counts().errors, 1);
    }

    /// Newer empty pull diagnostics should clear an older push snapshot.
    #[test]
    fn test_apply_lsp_file_diagnostics_allows_newer_empty_pull_clear() {
        let mut editor = create_editor_with_content("fn main() {}\n");
        editor.set_startup_path("/tmp/main.rs");
        let path = PathBuf::from("/tmp/main.rs");
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            path.clone(),
            Some(3),
            vec![crate::lsp::LspDiagnostic {
                range: LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 3,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 7,
                    },
                },
                severity: LspDiagnosticSeverity::Error,
                message: "broken".to_string(),
                source: None,
                code: None,
            }],
        ));

        // A newer pull clear belongs to a later saved document version, so keep it.
        let changed = editor.apply_lsp_file_diagnostics(LspFileDiagnostics::with_transport(
            path,
            Some(4),
            Vec::new(),
            crate::lsp::diagnostics::DiagnosticTransport::Pull,
        ));

        assert!(changed);
        assert_eq!(editor.active_diagnostic_counts().errors, 0);
    }

    /// Unversioned empty diagnostics should not clear a newer versioned snapshot.
    #[test]
    fn test_apply_lsp_file_diagnostics_ignores_unversioned_empty_update_after_versioned_snapshot() {
        let mut editor = create_editor_with_content("fn main() {}\n");
        editor.set_startup_path("/tmp/main.rs");
        let path = PathBuf::from("/tmp/main.rs");
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            path.clone(),
            Some(3),
            vec![crate::lsp::LspDiagnostic {
                range: LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 3,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 7,
                    },
                },
                severity: LspDiagnosticSeverity::Error,
                message: "broken".to_string(),
                source: None,
                code: None,
            }],
        ));

        // Simulate a server clearing diagnostics without a version after a newer save.
        let changed =
            editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(path, None, Vec::new()));

        assert!(!changed);
        assert_eq!(editor.active_diagnostic_counts().errors, 1);
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
    /// Confirm async-only completion popups narrow in place while typing.
    fn test_async_completion_popup_stays_open_while_typing_more_prefix() {
        let mut editor = create_editor_with_content("use std::");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 9);
        let popup_anchor_char_idx = editor.cursor.to_char_index(&editor.buffer);
        let request = CompletionRequest::new(
            editor.active_buffer_id,
            0,
            build_lsp_trigger_request_identity(popup_anchor_char_idx),
        );
        editor.completion_session = Some(CompletionSession::new(
            request,
            vec![CompletionCandidate {
                source_id: CompletionSourceId::Lsp,
                insert_text: "alloc".to_string(),
                popup_label: "alloc".to_string(),
                popup_detail: Some("module"),
                normalized_match_text: "alloc".to_string(),
                replace_start_char_idx: popup_anchor_char_idx,
                replace_end_char_idx: popup_anchor_char_idx,
                rank: 0,
            }],
            popup_anchor_char_idx,
        ));

        // Typing one more character should locally narrow the async results
        // instead of dismissing the popup until the next LSP batch arrives.
        handle_key_and_flush_requests(&mut editor, Key::Char('a'));
        let session = editor
            .completion_session
            .as_ref()
            .expect("completion popup should stay open while typing");

        assert_eq!(session.popup_anchor_char_idx, popup_anchor_char_idx);
        assert_eq!(session.request().match_prefix(), "a");
        assert_eq!(session.candidates.len(), 1);
        assert_eq!(session.candidates[0].popup_label, "alloc");
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
        assert!(session.request().cursor_char_idx() > session.popup_anchor_char_idx);
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
    fn test_counted_insert_repeats_typed_text_on_escape() {
        let mut editor = create_editor_with_content("xy");

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('b'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "abababxy");
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 5);
    }

    #[test]
    fn test_counted_append_repeats_typed_text_on_escape() {
        let mut editor = create_editor_with_content("xy");

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('!'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "x!!y");
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_counted_insert_at_first_non_blank_repeats_typed_text() {
        let mut editor = create_editor_with_content("  word");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('I'));
        editor.handle_key(Key::Char('*'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "  ***word");
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_counted_append_at_line_end_repeats_typed_text() {
        let mut editor = create_editor_with_content("abc");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('A'));
        editor.handle_key(Key::Char('!'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "abc!!");
        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.column(), 4);
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
    fn test_insert_newline_auto_indents_supported_language() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n    \n}\n");
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_insert_closing_brace_auto_dedents_supported_language() {
        let mut editor = create_syntax_editor("fn main() {\n", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n    \n");
        assert_eq!(editor.cursor, Cursor::new(1, 4));

        editor.handle_key(Key::Char('}'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n}\n");
        assert_eq!(editor.cursor, Cursor::new(1, 1));
    }

    #[test]
    fn test_insert_newline_skips_auto_indent_for_unsupported_language() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "notes.txt");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n\n}\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_open_line_below_auto_indents_supported_language() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n    \n}\n");
        assert_eq!(editor.cursor, Cursor::new(1, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_above_auto_indents_supported_language() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('O'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n    \n}\n");
        assert_eq!(editor.cursor, Cursor::new(1, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_insert_newline_repeat_enter_cleans_up_empty_auto_indent() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n\n    \n}\n");
        assert_eq!(editor.cursor, Cursor::new(2, 4));
    }

    #[test]
    fn test_insert_newline_escape_cleans_up_empty_auto_indent() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "fn main() {\n\n}\n");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_dot_repeats_auto_indented_open_line_insert_session() {
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n    x\n    x\n}\n");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor, Cursor::new(2, 4));
    }

    #[test]
    fn test_insert_python_dedent_keyword_auto_dedents_supported_language() {
        let mut editor = create_syntax_editor("if cond:\n", "main.py");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "if cond:\n    \n");
        assert_eq!(editor.cursor, Cursor::new(1, 4));

        for ch in "else".chars() {
            editor.handle_key(Key::Char(ch));
        }

        assert_eq!(editor.buffer.to_string(), "if cond:\n    else\n");
        assert_eq!(editor.cursor, Cursor::new(1, 8));

        editor.handle_key(Key::Char(':'));

        assert_eq!(editor.buffer.to_string(), "if cond:\nelse:\n");
        assert_eq!(editor.cursor, Cursor::new(1, 5));
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
        assert_eq!(editor.buffer.to_string(), "Xhello\n");
        assert!(!editor.buffer.is_modified());

        editor.handle_key(Key::Ctrl('r'));
        assert_eq!(editor.buffer.to_string(), "XYhello\n");
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
    /// `<Space>l` should hide committed highlights without clearing repeat-search state.
    fn test_space_l_hides_search_highlights_without_clearing_last_search() {
        let mut editor = create_editor_with_content("alpha\nbeta\nalpha");
        editor.execute_search("alpha");

        assert_eq!(editor.search_highlight_snapshot(), vec![(0, 5), (11, 16)]);
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('l'));
        assert_eq!(editor.search_highlight_snapshot(), Vec::new());

        // Repeating the search should reveal highlights again because the last query still exists.
        editor.handle_key(Key::Char('n'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.search_highlight_snapshot(), vec![(0, 5), (11, 16)]);
    }

    #[test]
    /// Command-mode `:{number}` should move to the requested line.
    fn test_goto_line() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4\nline5");

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.cursor.line(), 2); // 0-indexed
    }

    #[test]
    /// Jump history should replay command-mode line jumps in both directions.
    fn test_jump_history_replays_goto_line_backward_and_forward() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4\nline5");

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('4'));
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.cursor.line(), 3);

        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.cursor.line(), 0);

        editor.handle_key(Key::Ctrl('i'));
        assert_eq!(editor.cursor.line(), 3);
    }

    #[test]
    /// Plain local motions should not create jump-history entries.
    fn test_jump_history_ignores_plain_character_motions() {
        let mut editor = create_editor_with_content("line1\nline2\nline3");

        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 1);

        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Already at oldest jump")
        );
    }

    #[test]
    /// A fresh jump should discard forward history created by moving backward.
    fn test_jump_history_clears_forward_entries_after_fresh_jump() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4\nline5");

        editor.handle_key(Key::Char('G'));
        assert_eq!(editor.cursor.line(), 4);

        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.cursor.line(), 0);

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.cursor.line(), 2);

        editor.handle_key(Key::Ctrl('i'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Already at newest jump")
        );
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
    /// Regex search patterns should use the configured regex syntax.
    fn test_search_uses_regex_syntax() {
        let mut editor = create_editor_with_content("abc\naxc\n");

        editor.handle_key(Key::Char('/'));
        for c in "a.c\n".chars() {
            editor.handle_key(Key::Char(c));
        }

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
        editor.handle_key(Key::Char('n'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Repeated regex searches should keep overlapping matches reachable.
    fn test_search_repeat_supports_overlapping_matches() {
        let mut editor = create_editor_with_content("banana");

        editor.handle_key(Key::Char('/'));
        for c in "ana\n".chars() {
            editor.handle_key(Key::Char(c));
        }

        assert_eq!(editor.cursor.column(), 1);
        editor.handle_key(Key::Char('n'));
        assert_eq!(editor.cursor.column(), 3);
        editor.handle_key(Key::Char('N'));
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    /// Invalid regex input should surface a search error instead of falling back.
    fn test_search_invalid_regex_sets_status_message() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('/'));
        for c in "(?=beta)\n".chars() {
            editor.handle_key(Key::Char(c));
        }

        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Invalid regex:"))
        );
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
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new\n");
        assert!(
            editor
                .status_message
                .as_deref()
                .unwrap()
                .contains("written")
        );
    }

    #[test]
    /// Saving should normalize the live buffer to the trailing newline written to disk.
    fn test_w_current_file_normalizes_live_buffer_after_save() {
        let target = TempFile::with_suffix("_normalize_write").unwrap();
        fs::write(target.path(), "old").unwrap();

        let mut editor = create_editor_with_content("new");
        editor.file_path = target.path().to_path_buf();
        editor.mode = Mode::command_with_text("w");
        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "new\n");
        assert!(!editor.buffer.is_modified());
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new\n");
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
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new\n");
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
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "old!\n");
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
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new\n");
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
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "new\n");
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
        assert_eq!(fs::read_to_string(target.path()).unwrap(), "old!\n");
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
    fn test_q_starts_pending_macro_record_prefix() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('q'));

        assert_eq!(editor.pending_prefix_label(), Some("q".to_string()));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_pending_macro_playback_prefix_keeps_count() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('@'));

        assert_eq!(editor.pending_prefix_label(), Some("3@".to_string()));
    }

    #[test]
    fn test_macro_recording_indicator_appears_after_register_selection() {
        let mut editor = create_editor_with_content("hello");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.pending_prefix_label(), None);
        assert_eq!(
            editor.macro_recording_label(),
            Some("recording @a".to_string())
        );

        editor.handle_key(Key::Char('q'));
        assert_eq!(editor.pending_prefix_label(), None);
        assert_eq!(editor.macro_recording_label(), None);
    }

    #[test]
    fn test_macro_replay_repeats_recorded_insert_session() {
        let mut editor = create_editor_with_content("ab");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('q'));
        assert_eq!(editor.buffer.to_string(), "Xab");

        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.buffer.to_string(), "XXab");
    }

    #[test]
    fn test_double_at_replays_last_played_macro() {
        let mut editor = create_editor_with_content("ab");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('q'));

        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('@'));

        assert_eq!(editor.buffer.to_string(), "XXXab");
    }

    #[test]
    fn test_macro_replay_captures_operator_sequences() {
        let mut editor = create_editor_with_content("one two three");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('w'));
        editor.handle_key(Key::Char('q'));
        assert_eq!(editor.buffer.to_string(), "two three");

        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.buffer.to_string(), "three");
    }

    #[test]
    fn test_macro_replay_captures_command_mode_input() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('q'));
        assert_eq!(editor.cursor.line(), 2);

        editor.cursor = Cursor::new(0, 0);
        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor.line(), 2);
    }

    #[test]
    fn test_macro_replay_captures_search_mode_input() {
        let mut editor = create_editor_with_content("alpha\nbeta\ngamma");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('/'));
        editor.handle_key(Key::Char('b'));
        editor.handle_key(Key::Char('e'));
        editor.handle_key(Key::Char('t'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('q'));
        assert_eq!(editor.cursor, Cursor::new(1, 0));

        editor.cursor = Cursor::new(0, 0);
        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_empty_macro_shows_error_on_playback() {
        let mut editor = create_editor_with_content("ab");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));

        assert_eq!(editor.status_message, Some("Macro @a is empty".to_string()));
    }

    #[test]
    fn test_macro_replay_is_blocked_while_recording() {
        let mut editor = create_editor_with_content("ab");

        editor.handle_key(Key::Char('q'));
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Char('@'));
        editor.handle_key(Key::Char('a'));

        assert_eq!(
            editor.status_message,
            Some("Cannot replay a macro while recording".to_string())
        );
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
                        keys: "e".to_string(),
                        action: "Move word end backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "E".to_string(),
                        action: "Move WORD end backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "v".to_string(),
                        action: "Recreate last selection".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "d".to_string(),
                        action: "Go to definition".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "r".to_string(),
                        action: "Go to references".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "f".to_string(),
                        action: "Go to file under cursor".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "F".to_string(),
                        action: "Go to file under cursor at position".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "a".to_string(),
                        action: "Go to alternate file".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: ".".to_string(),
                        action: "Go to last modification".to_string(),
                    },
                ],
            })
        );
    }

    #[test]
    fn test_sequence_discovery_popup_shows_built_in_space_continuations() {
        let mut editor = create_editor_with_content("line1\nline2");

        editor.handle_key(Key::Char(' '));

        assert_eq!(
            editor.sequence_discovery_popup(),
            Some(SequenceDiscoveryPopup {
                prefix: " ".to_string(),
                entries: vec![
                    SequenceDiscoveryEntry {
                        keys: "a".to_string(),
                        action: "Open code actions".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "d".to_string(),
                        action: "Open diagnostics".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "w".to_string(),
                        action: "Save current file".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "q".to_string(),
                        action: "Update current file and quit".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "b".to_string(),
                        action: "Open buffer switcher".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "f".to_string(),
                        action: "Open file picker".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "l".to_string(),
                        action: "Hide search highlighting".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "r".to_string(),
                        action: "Rename symbol".to_string(),
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
    fn test_cc_keeps_changed_line_in_place() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");
        editor.cursor = Cursor::new(1, 1);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "one\n\nthree");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_cc_preserves_existing_empty_line() {
        let mut editor = create_editor_with_content("one\n\nthree");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "one\n\nthree");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
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
    fn test_apply_config_can_set_indent_settings() {
        let mut editor = create_editor_with_content("alpha");

        editor.apply_config(&ConfigSettings {
            indent_width: Some(2),
            indent_with_tabs: Some(true),
            ..ConfigSettings::default()
        });

        assert_eq!(editor.settings.indent_width, 2);
        assert!(editor.settings.indent_with_tabs);
    }

    #[test]
    fn test_replace_config_resets_indent_settings_to_defaults() {
        let mut editor = create_editor_with_content("alpha");
        editor.apply_config(&ConfigSettings {
            indent_width: Some(2),
            indent_with_tabs: Some(true),
            ..ConfigSettings::default()
        });

        editor.replace_config(&ConfigSettings::default());

        assert_eq!(editor.settings.indent_width, DEFAULT_INDENT_WIDTH);
        assert!(!editor.settings.indent_with_tabs);
    }

    #[test]
    fn test_equal_equal_reindents_c_like_line() {
        let mut editor =
            create_syntax_editor("fn main() {\nprintln!(\"hi\");\n}\n", "/tmp/main.rs");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    println!(\"hi\");\n}\n"
        );
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_equal_percent_reindents_matching_block() {
        let mut editor = create_syntax_editor(
            "fn main() {\nif ready {\nprintln!(\"hi\");\n}\n}\n",
            "/tmp/main.rs",
        );
        editor.cursor = Cursor::new(1, 9);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('%'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    if ready {\n        println!(\"hi\");\n    }\n}\n"
        );
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_equal_equal_inherits_previous_indent_for_markdown() {
        let mut editor = create_syntax_editor("  alpha\nbeta\n", "/tmp/notes.md");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), "  alpha\n  beta\n");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_indent_text_object_reindents_current_line() {
        let mut editor =
            create_syntax_editor("fn main() {\nprintln!(\"hi\");\n}\n", "/tmp/main.rs");
        editor.cursor = Cursor::new(1, 3);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    println!(\"hi\");\n}\n"
        );
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_visual_equal_reindents_python_lines_with_tabs() {
        let mut editor =
            create_syntax_editor("if cond:\nprint('a')\nelse:\nprint('b')\n", "/tmp/main.py");
        editor.apply_config(&ConfigSettings {
            indent_width: Some(4),
            indent_with_tabs: Some(true),
            ..ConfigSettings::default()
        });
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "if cond:\n\tprint('a')\nelse:\n\tprint('b')\n"
        );
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_greater_greater_indents_current_line_by_indent_width() {
        let mut editor = create_syntax_editor("alpha\nbeta\n", "/tmp/notes.txt");

        editor.handle_key(Key::Char('>'));
        editor.handle_key(Key::Char('>'));

        assert_eq!(editor.buffer.to_string(), "    alpha\nbeta\n");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_less_less_dedents_current_line_by_indent_width() {
        let mut editor = create_syntax_editor("    alpha\nbeta\n", "/tmp/notes.txt");

        editor.handle_key(Key::Char('<'));
        editor.handle_key(Key::Char('<'));

        assert_eq!(editor.buffer.to_string(), "alpha\nbeta\n");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_greater_percent_indents_matching_block_by_indent_width() {
        let mut editor = create_syntax_editor("{\nalpha\nbeta\n}\n", "/tmp/notes.txt");

        editor.handle_key(Key::Char('>'));
        editor.handle_key(Key::Char('%'));

        assert_eq!(
            editor.buffer.to_string(),
            "    {\n    alpha\n    beta\n    }\n"
        );
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_visual_greater_indents_selection_by_indent_width() {
        let mut editor = create_syntax_editor("alpha\nbeta\ngamma\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('>'));

        assert_eq!(editor.buffer.to_string(), "    alpha\n    beta\ngamma\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    fn test_visual_less_dedents_selection_by_indent_width() {
        let mut editor = create_syntax_editor("    alpha\n    beta\ngamma\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('<'));

        assert_eq!(editor.buffer.to_string(), "alpha\nbeta\ngamma\n");
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_indent_reports_unsupported_language() {
        let mut editor = create_syntax_editor("alpha\nbeta\n", "/tmp/notes.txt");

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), "alpha\nbeta\n");
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No manual indent rule for current language")
        );
    }

    #[test]
    fn test_indent_works_without_language_rules() {
        let mut editor = create_syntax_editor("alpha\n", "/tmp/notes.txt");

        editor.handle_key(Key::Char('>'));
        editor.handle_key(Key::Char('>'));

        assert_eq!(editor.buffer.to_string(), "    alpha\n");
        assert_eq!(editor.status_message, None);
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
    fn test_count_before_command_mode_prefills_line_number() {
        let mut editor = create_editor_with_content("one\ntwo\nthree\nfour\nfive");
        editor.handle_key(Key::Char('5'));
        editor.handle_key(Key::Char(':'));
        assert!(matches!(editor.mode, Mode::Command(_)));
        assert_eq!(editor.pending_prefix_label(), None);
        if let Mode::Command(ref input) = editor.mode {
            assert_eq!(input.text(), "5");
        } else {
            panic!("expected command mode");
        }
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.cursor.line(), 4);
    }

    #[test]
    fn test_count_before_search_mode_repeats_initial_search() {
        let mut editor = create_editor_with_content("target\nx\ntarget\ny\ntarget\nz\ntarget");

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('/'));
        for c in "target\n".chars() {
            editor.handle_key(Key::Char(c));
        }

        assert!(matches!(editor.mode, Mode::Normal));
        assert_eq!(editor.cursor.line(), 4);
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
    /// Regression test for `gv` recreating a full-file selection after `>` indents it.
    fn test_gv_recreates_full_characterwise_selection_after_indent() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    println!(\"Hello, world!\");\n}",
            "/tmp/main.rs",
        );

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('G'));
        editor.handle_key(Key::Char('$'));
        editor.handle_key(Key::Char('>'));

        assert!(editor.mode.is_normal());
        assert_eq!(
            editor.buffer.to_string(),
            "    fn main() {\n        println!(\"Hello, world!\");\n    }"
        );

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('v'));

        assert_eq!(editor.mode, Mode::Visual(VisualKind::Character));
        assert_eq!(
            editor.selection_range(),
            Some((0, editor.buffer.chars_count()))
        );
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
    fn test_visual_block_mode_tracks_rectangular_selection_on_ragged_lines() {
        let mut editor = create_editor_with_content("abcd\na\nabc");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('l'));

        assert_eq!(editor.mode, Mode::Visual(VisualKind::Block));
        assert_eq!(editor.selection_range(), None);
        assert!(editor.selection_contains_cell(0, 1));
        assert!(editor.selection_contains_cell(0, 2));
        assert!(!editor.selection_contains_cell(1, 1));
        assert!(editor.selection_contains_cell(2, 1));
        assert!(editor.selection_contains_cell(2, 2));
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
    fn test_visual_block_delete_then_paste_before_restores_ragged_block() {
        let mut editor = create_editor_with_content("abcd\na\nabc");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('d'));

        assert_eq!(editor.buffer.to_string(), "ad\na\na");
        assert!(editor.mode.is_normal());

        editor.handle_key(Key::Char('P'));

        assert_eq!(editor.buffer.to_string(), "abcd\na\nabc");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
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
    fn test_visual_line_i_leaves_non_block_selection_unchanged() {
        let mut editor = create_editor_with_content("one\ntwo");

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('I'));

        assert_eq!(editor.buffer.to_string(), "one\ntwo");
        assert_eq!(editor.mode, Mode::Visual(VisualKind::Line));
        assert_eq!(editor.status_message.as_deref(), None);
    }

    #[test]
    fn test_visual_character_a_leaves_non_block_selection_unchanged() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('A'));

        assert_eq!(editor.buffer.to_string(), "abcd");
        assert_eq!(editor.mode, Mode::Visual(VisualKind::Character));
        assert_eq!(editor.status_message.as_deref(), None);
    }

    #[test]
    fn test_visual_block_i_inserts_at_block_start_on_each_selected_line() {
        let mut editor =
            create_editor_with_content("fn main() {\n    println!(\"Hello, world!\");\n}");
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        for _ in 0..3 {
            editor.handle_key(Key::Char('l'));
        }
        editor.handle_key(Key::Char('I'));
        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Esc);

        assert_eq!(
            editor.buffer.to_string(),
            "fn 123main() {\n   123 println!(\"Hello, world!\");\n}"
        );
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_visual_block_a_appends_at_block_end_on_each_selected_line() {
        let mut editor =
            create_editor_with_content("fn main() {\n    println!(\"Hello, world!\");\n}");
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        for _ in 0..3 {
            editor.handle_key(Key::Char('l'));
        }
        editor.handle_key(Key::Char('A'));
        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Esc);

        assert_eq!(
            editor.buffer.to_string(),
            "fn main123() {\n    pri123ntln!(\"Hello, world!\");\n}"
        );
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_visual_block_a_pads_short_last_line_to_block_end() {
        let mut editor =
            create_editor_with_content("fn main() {\n    println!(\"Hello, world!\");\n}");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('A'));
        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Esc);

        assert_eq!(
            editor.buffer.to_string(),
            "fn main123() {\n    pri123ntln!(\"Hello, world!\");\n}      123"
        );
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_visual_block_insert_preserves_typed_order_on_every_line() {
        let mut editor = create_editor_with_content("abcd\nabcd\nabcd");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('A'));
        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "abc123d\nabc123d\nabc123d");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_visual_block_i_uses_block_start_even_when_last_line_is_short() {
        let mut editor =
            create_editor_with_content("fn main() {\n    println!(\"Hello, world!\");\n}");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('I'));
        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Esc);

        assert_eq!(
            editor.buffer.to_string(),
            "123fn main() {\n123    println!(\"Hello, world!\");\n123}"
        );
        assert!(editor.mode.is_normal());
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
    fn test_gv_recreates_last_block_selection() {
        let mut editor = create_editor_with_content("abcd\na\nabc");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Esc);

        assert!(editor.mode.is_normal());
        assert_eq!(editor.selection_range(), None);

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('v'));

        assert_eq!(editor.mode, Mode::Visual(VisualKind::Block));
        assert!(editor.selection_contains_cell(0, 1));
        assert!(editor.selection_contains_cell(0, 2));
        assert!(!editor.selection_contains_cell(1, 1));
        assert!(editor.selection_contains_cell(2, 1));
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
    /// Stale navigation results should be ignored without clearing the live lookup.
    fn test_apply_navigation_lookup_result_rejects_stale_token() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind: NavigationKind::Definition,
            token: 7,
            document_version: 3,
        });

        let changed = editor.apply_navigation_lookup_result(NavigationLookupResult {
            kind: NavigationKind::Definition,
            buffer_id: editor.active_buffer_id,
            lookup_token: 8,
            document_version: 3,
            outcome: NavigationLookupOutcome::NotFound,
        });

        assert!(!changed);
        assert_eq!(
            editor.active_navigation_lookup,
            Some(ActiveNavigationLookup {
                kind: NavigationKind::Definition,
                token: 7,
                document_version: 3,
            })
        );
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// Debounced sync snapshots should stay pending until the debounce delay expires.
    fn test_take_due_document_sync_snapshot_waits_for_debounce() {
        let mut editor = create_editor_with_content("ab");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.insert_buffer_text(2, "c");

        assert!(
            editor
                .take_due_document_sync_snapshot(Instant::now())
                .is_none()
        );
    }

    #[test]
    /// Insert edits should queue one incremental LSP change at the pre-edit position.
    fn test_insert_buffer_text_queues_incremental_lsp_change() {
        let mut editor = create_editor_with_content("ab\ncd");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.insert_buffer_text(4, "xy");

        let snapshot = editor
            .take_due_document_sync_snapshot(Instant::now() + EditorState::LSP_SYNC_DEBOUNCE_DELAY)
            .expect("pending sync snapshot");

        assert_eq!(snapshot.document_version, 1);
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(
            snapshot.changes[0].range,
            Some(LspRange {
                start: LspPosition {
                    line: 1,
                    character: 1,
                },
                end: LspPosition {
                    line: 1,
                    character: 1,
                },
            })
        );
        assert_eq!(snapshot.changes[0].text, "xy");
    }

    #[test]
    /// Save snapshots should preserve the active file path when the URI stays unchanged.
    fn test_document_save_snapshot_uses_current_path_for_normal_write() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.insert_buffer_text(5, "!");
        let expected_path = std::env::current_dir()
            .expect("current directory")
            .join("src/main.rs");

        let snapshot = editor
            .document_save_snapshot(Path::new("src/main.rs"), false)
            .expect("save snapshot");

        assert_eq!(snapshot.document_version, 1);
        assert_eq!(snapshot.previous_file_path, None);
        assert_eq!(snapshot.file_path, expected_path);
        assert_eq!(snapshot.changes.len(), 1);
    }

    #[test]
    /// Save snapshots should retain the old URI when writing the buffer to a new path.
    fn test_document_save_snapshot_tracks_previous_path_for_save_as() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.insert_buffer_text(5, "!");
        let current_dir = std::env::current_dir().expect("current directory");

        let snapshot = editor
            .document_save_snapshot(Path::new("src/lib.rs"), true)
            .expect("save-as snapshot");

        assert_eq!(
            snapshot.previous_file_path,
            Some(current_dir.join("src/main.rs"))
        );
        assert_eq!(snapshot.file_path, current_dir.join("src/lib.rs"));
        assert_eq!(snapshot.changes.len(), 1);
    }

    #[test]
    /// Save snapshots should match the trailing newline written to disk.
    fn test_document_save_snapshot_appends_trailing_newline_when_missing() {
        let editor = create_editor_with_content("alpha");

        let snapshot = editor
            .document_save_snapshot(Path::new("src/main.rs"), false)
            .expect("save snapshot");

        assert_eq!(snapshot.text.to_string(), "alpha\n");
    }

    #[test]
    /// Remove edits should queue one incremental LSP change spanning the deleted text.
    fn test_remove_buffer_range_queues_incremental_lsp_change() {
        let mut editor = create_editor_with_content("ab\ncd");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.remove_buffer_range(1, 4);

        let snapshot = editor
            .take_due_document_sync_snapshot(Instant::now() + EditorState::LSP_SYNC_DEBOUNCE_DELAY)
            .expect("pending sync snapshot");

        assert_eq!(snapshot.document_version, 1);
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(
            snapshot.changes[0].range,
            Some(LspRange {
                start: LspPosition {
                    line: 0,
                    character: 1,
                },
                end: LspPosition {
                    line: 1,
                    character: 1,
                },
            })
        );
        assert_eq!(snapshot.changes[0].text, "");
    }

    #[test]
    /// Removing text before the popup anchor should move visible and queued anchors left.
    fn test_remove_buffer_range_shifts_completion_popup_anchors() {
        let mut editor = create_editor_with_content("use std::alloc\nnext");
        let anchor_char_idx = 14;
        let request = CompletionRequest::new(
            editor.active_buffer_id,
            0,
            build_lsp_trigger_request_identity(anchor_char_idx),
        );
        editor.completion_session = Some(CompletionSession::new(
            request.clone(),
            vec![CompletionCandidate {
                source_id: CompletionSourceId::Lsp,
                insert_text: "alloc".to_string(),
                popup_label: "alloc".to_string(),
                popup_detail: Some("module"),
                normalized_match_text: "alloc".to_string(),
                replace_start_char_idx: anchor_char_idx,
                replace_end_char_idx: anchor_char_idx,
                rank: 0,
            }],
            anchor_char_idx,
        ));
        editor.pending_lsp_completion = Some(PendingLspCompletion {
            request,
            popup_anchor_char_idx: anchor_char_idx,
            document_version: editor.lsp_document_version,
            due_at: Instant::now(),
            trigger_text: None,
        });

        // Backspacing over the final two letters must pull the saved popup anchor
        // left with the shortened line instead of leaving it past the newline.
        editor.remove_buffer_range(12, 14);

        assert_eq!(
            editor
                .completion_session
                .as_ref()
                .expect("completion session")
                .popup_anchor_char_idx,
            12
        );
        assert_eq!(
            editor
                .pending_lsp_completion
                .as_ref()
                .expect("pending LSP completion")
                .popup_anchor_char_idx,
            12
        );
    }

    #[test]
    fn test_goto_next_and_prev_diagnostic_move_cursor() {
        let mut editor = create_editor_with_content("alpha\nbeta\ngamma");
        apply_test_diagnostics(
            &mut editor,
            "/tmp/diagnostics.rs",
            vec![
                (0, 1, 3, LspDiagnosticSeverity::Warning, "first"),
                (1, 2, 4, LspDiagnosticSeverity::Error, "second"),
            ],
        );

        editor.goto_next_diagnostic();
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);

        editor.goto_next_diagnostic();
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 2);

        editor.goto_prev_diagnostic();
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_open_diagnostics_picker_uses_active_buffer_diagnostics() {
        let mut editor = create_editor_with_content("alpha\nbeta");
        apply_test_diagnostics(
            &mut editor,
            "/tmp/diagnostics.rs",
            vec![
                (0, 0, 5, LspDiagnosticSeverity::Error, "alpha error"),
                (1, 0, 4, LspDiagnosticSeverity::Warning, "beta warning"),
            ],
        );

        editor.open_diagnostics_picker();

        let popup = editor.picker_popup().expect("diagnostics popup");
        assert_eq!(popup.title, "Diagnostics");
        assert_eq!(popup.query_suffix, "2 ");
        assert_eq!(popup.entries.len(), 2);
        assert!(popup.entries[0].label.contains("alpha error"));
        assert!(popup.entries[1].label.contains("beta warning"));
    }

    #[test]
    fn test_active_buffer_diagnostics_hide_when_local_edits_advance_document_version() {
        let mut editor = create_editor_with_content("alpha");
        apply_test_diagnostics(
            &mut editor,
            "/tmp/diagnostics.rs",
            vec![(0, 0, 5, LspDiagnosticSeverity::Error, "alpha error")],
        );

        assert_eq!(
            editor.line_diagnostic_severity(0),
            Some(LspDiagnosticSeverity::Error)
        );

        editor.insert_buffer_text(5, "!");

        assert_eq!(editor.line_diagnostic_severity(0), None);
        assert_eq!(editor.cursor_diagnostic(), None);
    }

    #[test]
    /// Sync completion should trigger a redraw when versionless diagnostics become visible again.
    fn test_document_sync_outcome_reports_visible_versionless_diagnostics() {
        let mut editor = create_editor_with_content("alpha");
        editor.set_startup_path("/tmp/diagnostics.rs");
        editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
            PathBuf::from("/tmp/diagnostics.rs"),
            None,
            vec![crate::lsp::LspDiagnostic {
                range: LspRange {
                    start: LspPosition {
                        line: 0,
                        character: 0,
                    },
                    end: LspPosition {
                        line: 0,
                        character: 5,
                    },
                },
                severity: LspDiagnosticSeverity::Error,
                message: "alpha error".to_string(),
                source: None,
                code: None,
            }],
        ));
        editor.insert_buffer_text(5, "!");

        assert_eq!(editor.line_diagnostic_severity(0), None);
        assert!(
            editor.apply_document_sync_outcome(DocumentSyncOutcome::Synced {
                buffer_id: editor.active_buffer_id,
                document_version: editor.lsp_document_version,
            })
        );
        assert_eq!(
            editor.line_diagnostic_severity(0),
            Some(LspDiagnosticSeverity::Error)
        );
    }

    #[test]
    /// Go-to-definition should force one full sync while keeping the current LSP version.
    fn test_request_goto_definition_uses_current_lsp_version() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.insert_buffer_text(5, "!");
        editor.request_navigation(NavigationKind::Definition);

        let snapshot = editor
            .navigation_request_snapshot()
            .expect("definition request snapshot");

        assert_eq!(snapshot.document_version, 1);
        assert!(snapshot.force_full_sync);
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(
            editor.active_navigation_lookup,
            Some(ActiveNavigationLookup {
                kind: NavigationKind::Definition,
                token: 1,
                document_version: 1,
            })
        );
    }

    #[test]
    /// Go-to-references should queue one references lookup with the current document version.
    fn test_request_goto_references_sets_navigation_kind() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.request_navigation(NavigationKind::References);

        assert_eq!(
            editor.pending_request,
            Some(EditorRequest::LspNavigation(NavigationKind::References))
        );
        assert_eq!(
            editor.active_navigation_lookup,
            Some(ActiveNavigationLookup {
                kind: NavigationKind::References,
                token: 1,
                document_version: 0,
            })
        );
    }

    #[test]
    /// Hover should queue one deferred LSP request with the current buffer version.
    fn test_request_hover_sets_pending_request() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.request_hover();

        assert_eq!(editor.pending_request, Some(EditorRequest::LspHover));
        assert_eq!(
            editor.active_hover_lookup,
            Some(ActiveHoverLookup {
                token: 1,
                document_version: 0,
            })
        );
        assert_eq!(editor.status_message.as_deref(), Some("Resolving hover..."));
    }

    #[test]
    /// Queued app-layer requests should keep background polling active until handled.
    fn test_pending_request_keeps_background_poll_active() {
        let mut editor = create_editor_with_content("alpha");
        editor.pending_request = Some(EditorRequest::ReloadConfig);

        assert!(editor.needs_background_poll());

        let _request = editor.take_pending_request();
        assert!(!editor.needs_background_poll());
    }

    #[test]
    /// Hover requests should force one full sync while keeping the current LSP version.
    fn test_hover_request_snapshot_uses_current_lsp_version() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.insert_buffer_text(5, "!");
        editor.request_hover();

        let snapshot = editor
            .hover_request_snapshot()
            .expect("hover request snapshot");

        assert_eq!(snapshot.document_version, 1);
        assert!(snapshot.force_full_sync);
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(
            editor.active_hover_lookup,
            Some(ActiveHoverLookup {
                token: 1,
                document_version: 1,
            })
        );
    }

    #[test]
    /// Rename requests should queue one deferred LSP request with the current buffer version.
    fn test_request_rename_sets_pending_request() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.request_rename("beta".to_string());

        assert_eq!(
            editor.pending_request,
            Some(EditorRequest::LspRename("beta".to_string()))
        );
        assert_eq!(
            editor.active_rename_lookup,
            Some(ActiveRenameLookup {
                token: 1,
                document_version: 0,
                request_edit_generation: 0,
                new_name: "beta".to_string(),
            })
        );
        assert_eq!(editor.status_message.as_deref(), Some("Renaming symbol..."));
    }

    #[test]
    /// Code-action requests should queue one deferred LSP request with the current buffer version.
    fn test_request_code_actions_sets_pending_request() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.request_code_actions();

        assert_eq!(editor.pending_request, Some(EditorRequest::LspCodeAction));
        assert_eq!(
            editor.active_code_action_lookup,
            Some(ActiveCodeActionLookup {
                token: 1,
                document_version: 0,
                request_edit_generation: 0,
            })
        );
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Loading code actions...")
        );
    }

    #[test]
    /// Lookup requests should share one monotonic token sequence across request kinds.
    fn test_lookup_requests_share_one_token_sequence() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        editor.request_navigation(NavigationKind::Definition);
        let navigation = editor
            .active_navigation_lookup
            .expect("navigation lookup token");
        editor.request_hover();
        let hover = editor.active_hover_lookup.expect("hover lookup token");
        // Rename clears older lookup state, so capture the earlier tokens first.
        editor.request_rename("beta".to_string());
        let rename = editor
            .active_rename_lookup
            .clone()
            .expect("rename lookup token");

        assert_eq!(navigation.token, 1);
        assert_eq!(hover.token, 2);
        assert_eq!(rename.token, 3);
    }

    #[test]
    /// The built-in rename shortcut should prefill command mode with the current symbol name.
    fn test_prompt_rename_symbol_prefills_command_mode() {
        let mut editor = create_editor_with_content("alpha");

        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('r'));

        assert_eq!(editor.mode.command_string(), Some("rename alpha"));
    }

    #[test]
    /// Rename prefill should follow syntax-profile identifier rules for dashed identifiers.
    fn test_prompt_rename_symbol_uses_syntax_profile_identifier_rules() {
        let mut editor = create_editor_with_content("image-tag = true");
        editor.file_path = PathBuf::from("config.cfg");

        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('r'));

        assert_eq!(editor.mode.command_string(), Some("rename image-tag"));
    }

    #[test]
    /// Rename prefill should stay empty when the active syntax profile has no identifiers.
    fn test_prompt_rename_symbol_skips_profiles_without_identifier_rules() {
        let mut editor = create_editor_with_content("project-name");
        editor.file_path = PathBuf::from("notes.md");

        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('r'));

        assert_eq!(editor.mode.command_string(), Some("rename "));
    }

    #[test]
    /// Rename results should update the active buffer and open unopened targets as dirty buffers.
    fn test_apply_rename_lookup_result_opens_unopened_targets_as_buffers() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.rs", "fn main() { helper_value(); }\n")
            .expect("write main");
        tree.write_file("src/lib.rs", "pub fn helper_value() {}\n")
            .expect("write lib");

        let mut editor = EditorState::new(24);
        editor
            .load_file(tree.path().join("src/main.rs"))
            .expect("load main");
        editor.active_rename_lookup = Some(ActiveRenameLookup {
            token: 3,
            document_version: 0,
            request_edit_generation: 0,
            new_name: "helper_total".to_string(),
        });

        let changed = editor.apply_rename_lookup_result(RenameLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 3,
            document_version: 0,
            outcome: RenameLookupOutcome::Applied(LspWorkspaceEdit {
                document_edits: vec![
                    crate::lsp::protocol::LspDocumentEdit {
                        path: tree.path().join("src/main.rs"),
                        edits: vec![crate::lsp::protocol::LspTextEdit {
                            range: LspRange {
                                start: LspPosition {
                                    line: 0,
                                    character: 12,
                                },
                                end: LspPosition {
                                    line: 0,
                                    character: 24,
                                },
                            },
                            new_text: "helper_total".to_string(),
                        }],
                    },
                    crate::lsp::protocol::LspDocumentEdit {
                        path: tree.path().join("src/lib.rs"),
                        edits: vec![crate::lsp::protocol::LspTextEdit {
                            range: LspRange {
                                start: LspPosition {
                                    line: 0,
                                    character: 7,
                                },
                                end: LspPosition {
                                    line: 0,
                                    character: 19,
                                },
                            },
                            new_text: "helper_total".to_string(),
                        }],
                    },
                ],
            }),
        });

        assert!(changed);
        assert_eq!(
            editor.buffer.to_string(),
            "fn main() { helper_total(); }\n".to_string()
        );
        let summaries = editor.buffer_summaries();
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().all(|summary| summary.modified));
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Renamed symbol across 2 file(s)")
        );
        editor.activate_buffer(summaries[1].id);
        assert_eq!(editor.buffer.to_string(), "pub fn helper_total() {}\n");
        assert_eq!(
            fs::read_to_string(tree.path().join("src/lib.rs")).expect("read lib"),
            "pub fn helper_value() {}\n".to_string()
        );
    }

    #[test]
    /// Rename results should abort before applying edits into an open unsynced target buffer.
    fn test_apply_rename_lookup_result_aborts_for_unsynced_open_target() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.rs", "fn main() { helper_value(); }\n")
            .expect("write main");
        tree.write_file("src/lib.rs", "pub fn helper_value() {}\n")
            .expect("write lib");
        let _guard = CurrentDirectoryGuard::change_to(tree.path());

        let mut editor = EditorState::new(24);
        editor
            .load_file(tree.path().join("src/main.rs"))
            .expect("load main");
        let main_id = editor.active_buffer_id;
        editor
            .open_startup_buffer(tree.path().join("src/lib.rs"))
            .expect("open lib");
        let lib_id = editor.active_buffer_id;
        // Leave the target buffer dirty with pending sync work so rename must stop.
        editor.buffer.set_modified(true);
        editor.activate_buffer(main_id);
        editor.active_rename_lookup = Some(ActiveRenameLookup {
            token: 6,
            document_version: 0,
            request_edit_generation: 0,
            new_name: "helper_total".to_string(),
        });

        let changed = editor.apply_rename_lookup_result(RenameLookupResult {
            buffer_id: main_id,
            lookup_token: 6,
            document_version: 0,
            outcome: RenameLookupOutcome::Applied(LspWorkspaceEdit {
                document_edits: vec![
                    crate::lsp::protocol::LspDocumentEdit {
                        path: tree.path().join("src/main.rs"),
                        edits: vec![crate::lsp::protocol::LspTextEdit {
                            range: LspRange {
                                start: LspPosition {
                                    line: 0,
                                    character: 12,
                                },
                                end: LspPosition {
                                    line: 0,
                                    character: 24,
                                },
                            },
                            new_text: "helper_total".to_string(),
                        }],
                    },
                    crate::lsp::protocol::LspDocumentEdit {
                        path: tree.path().join("src/lib.rs"),
                        edits: vec![crate::lsp::protocol::LspTextEdit {
                            range: LspRange {
                                start: LspPosition {
                                    line: 0,
                                    character: 7,
                                },
                                end: LspPosition {
                                    line: 0,
                                    character: 19,
                                },
                            },
                            new_text: "helper_total".to_string(),
                        }],
                    },
                ],
            }),
        });

        assert!(changed);
        assert_eq!(editor.active_rename_lookup, None);
        assert_eq!(editor.buffer.to_string(), "fn main() { helper_value(); }\n");
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Rename aborted because open target buffer \"src/lib.rs\" has unsynced changes")
        );
        editor.activate_buffer(lib_id);
        assert_eq!(editor.buffer.to_string(), "pub fn helper_value() {}\n");
    }

    #[test]
    /// Hover results should materialize a popup and clear the transient status line.
    fn test_apply_hover_lookup_result_sets_popup() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_hover_lookup = Some(ActiveHoverLookup {
            token: 4,
            document_version: 2,
        });
        editor.status_message = Some("Resolving hover...".to_string());

        let changed = editor.apply_hover_lookup_result(HoverLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 4,
            document_version: 2,
            outcome: HoverLookupOutcome::Found("fn helper_value() -> i32".to_string()),
        });

        assert!(changed);
        assert_eq!(editor.active_hover_lookup, None);
        assert_eq!(editor.status_message, None);
        assert_eq!(
            editor.hover_popup,
            Some(HoverPopup::new("fn helper_value() -> i32"))
        );
    }

    #[test]
    /// Stale hover results should be ignored without clearing the live lookup.
    fn test_apply_hover_lookup_result_rejects_stale_token() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_hover_lookup = Some(ActiveHoverLookup {
            token: 7,
            document_version: 3,
        });

        let changed = editor.apply_hover_lookup_result(HoverLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 8,
            document_version: 3,
            outcome: HoverLookupOutcome::NotFound,
        });

        assert!(!changed);
        assert_eq!(
            editor.active_hover_lookup,
            Some(ActiveHoverLookup {
                token: 7,
                document_version: 3,
            })
        );
        assert_eq!(editor.hover_popup, None);
    }

    #[test]
    /// Missing server binaries should not interrupt automatic signature help with status noise.
    fn test_apply_signature_help_lookup_result_suppresses_missing_binary_message() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_signature_help_lookup = Some(ActiveSignatureHelpLookup {
            token: 8,
            document_version: 2,
            cursor_char_idx: 5,
            anchor_char_idx: 5,
        });
        editor.status_message = Some("keep typing".to_string());

        let changed = editor.apply_signature_help_lookup_result(SignatureHelpLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 8,
            document_version: 2,
            missing_server_binary: true,
            outcome: SignatureHelpLookupOutcome::Unavailable(
                "language server \"rust-analyzer\" is not in PATH; install \"rust-analyzer\" or add it to PATH"
                    .to_string(),
            ),
        });

        assert!(changed);
        assert_eq!(editor.active_signature_help_lookup, None);
        assert_eq!(editor.signature_help_popup, None);
        assert_eq!(editor.status_message.as_deref(), Some("keep typing"));
    }

    /// Go-to-definition should force a full sync whenever the buffer is modified.
    #[test]
    fn test_request_goto_definition_forces_full_sync_without_pending_changes() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.buffer.set_modified(true);
        editor.request_navigation(NavigationKind::Definition);

        let snapshot = editor
            .navigation_request_snapshot()
            .expect("definition request snapshot");

        assert!(snapshot.force_full_sync);
        assert!(snapshot.changes.is_empty());
    }

    #[test]
    /// Navigation results should still apply after switching to another open buffer.
    fn test_apply_navigation_lookup_result_considers_inactive_origin_buffer() {
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
        editor.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind: NavigationKind::Definition,
            token: 11,
            document_version: 4,
        });
        let first_id = editor.active_buffer_id;
        editor
            .open_startup_buffer(second.path())
            .expect("open second buffer");
        let second_id = editor.active_buffer_id;
        editor.activate_buffer(second_id);

        let changed = editor.apply_navigation_lookup_result(NavigationLookupResult {
            kind: NavigationKind::Definition,
            buffer_id: first_id,
            lookup_token: 11,
            document_version: 4,
            outcome: NavigationLookupOutcome::Single(NavigationTarget {
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
    /// Multiple navigation targets should open the picker instead of jumping immediately.
    fn test_apply_navigation_lookup_result_opens_location_picker() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind: NavigationKind::References,
            token: 5,
            document_version: 2,
        });

        let changed = editor.apply_navigation_lookup_result(NavigationLookupResult {
            kind: NavigationKind::References,
            buffer_id: editor.active_buffer_id,
            lookup_token: 5,
            document_version: 2,
            outcome: NavigationLookupOutcome::Multiple(vec![
                NavigationTarget {
                    file_path: PathBuf::from("src/lib.rs"),
                    line: 0,
                    character: 7,
                    display_label: "src/lib.rs:1:8".to_string(),
                },
                NavigationTarget {
                    file_path: PathBuf::from("src/other.rs"),
                    line: 1,
                    character: 2,
                    display_label: "src/other.rs:2:3".to_string(),
                },
            ]),
        });

        assert!(changed);
        assert!(matches!(editor.mode, Mode::LocationPicker(_)));
        assert!(editor.location_picker.is_some());
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// One returned code action should still open the picker so Escape can cancel it.
    fn test_apply_code_action_lookup_result_opens_picker_for_single_action() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_code_action_lookup = Some(ActiveCodeActionLookup {
            token: 11,
            document_version: 2,
            request_edit_generation: 4,
        });

        let changed = editor.apply_code_action_lookup_result(CodeActionLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 11,
            document_version: 2,
            outcome: CodeActionLookupOutcome::Found(vec![LspCodeAction {
                title: "Apply quick fix".to_string(),
                edit: LspWorkspaceEdit {
                    document_edits: Vec::new(),
                },
            }]),
        });

        assert!(changed);
        assert!(matches!(editor.mode, Mode::CodeActionPicker(_)));
        assert!(editor.code_action_picker.is_some());
        assert_eq!(editor.active_code_action_lookup, None);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// A not-found lookup should clear the pending request and surface feedback.
    fn test_apply_navigation_lookup_result_sets_not_found_message() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind: NavigationKind::Definition,
            token: 9,
            document_version: 6,
        });

        let changed = editor.apply_navigation_lookup_result(NavigationLookupResult {
            kind: NavigationKind::Definition,
            buffer_id: editor.active_buffer_id,
            lookup_token: 9,
            document_version: 6,
            outcome: NavigationLookupOutcome::NotFound,
        });

        assert!(changed);
        assert_eq!(editor.active_navigation_lookup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No definition found")
        );
    }

    #[test]
    /// A references not-found lookup should report the references-specific message.
    fn test_apply_navigation_lookup_result_sets_references_not_found_message() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind: NavigationKind::References,
            token: 10,
            document_version: 2,
        });

        let changed = editor.apply_navigation_lookup_result(NavigationLookupResult {
            kind: NavigationKind::References,
            buffer_id: editor.active_buffer_id,
            lookup_token: 10,
            document_version: 2,
            outcome: NavigationLookupOutcome::NotFound,
        });

        assert!(changed);
        assert_eq!(editor.active_navigation_lookup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No references found")
        );
    }

    #[test]
    /// Definition jumps should keep buffer paths relative when they stay under cwd.
    fn test_goto_navigation_target_opens_relative_path_within_current_directory() {
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

        editor.goto_navigation_target(&NavigationTarget {
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
    /// Jump history should replay cross-file navigation targets in both directions.
    fn test_jump_history_replays_navigation_targets_across_files() {
        let first = TempFile::with_suffix("_first.rs").expect("create first file");
        first
            .write_all(b"fn first() {}\n")
            .expect("seed first file");
        let target = TempFile::with_suffix("_target.rs").expect("create target file");
        target
            .write_all(b"fn target() {}\n")
            .expect("seed target file");

        let mut editor = EditorState::new(24);
        editor.load_file(first.path()).expect("load first file");
        editor.cursor = Cursor::new(0, 3);

        editor.goto_navigation_target(&NavigationTarget {
            file_path: target.path().to_path_buf(),
            line: 0,
            character: 7,
            display_label: "target.rs:1:8".to_string(),
        });
        assert_eq!(editor.file_path, target.path());
        assert_eq!(editor.cursor.column(), 7);

        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.file_path, first.path());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 3);

        editor.handle_key(Key::Ctrl('i'));
        assert_eq!(editor.file_path, target.path());
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
    fn test_big_word_motions_and_underscore_binding() {
        let mut editor = create_editor_with_content("  one\nalpha::beta gamma");

        editor.handle_key(Key::Char('_'));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('_'));
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('W'));
        assert_eq!(editor.cursor.column(), 12);

        editor.handle_key(Key::Char('B'));
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Char('E'));
        assert_eq!(editor.cursor.column(), 10);
    }

    #[test]
    fn test_tilde_toggles_case_and_dot_repeats_it() {
        let mut editor = create_editor_with_content("Ab");

        editor.handle_key(Key::Char('~'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "aB");
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    fn test_visual_tilde_toggles_selection_and_exits_visual_mode() {
        let mut editor = create_editor_with_content("AbC");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('~'));

        assert_eq!(editor.buffer.to_string(), "aBC");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_d_and_c_aliases_use_line_end_editing() {
        let mut editor = create_editor_with_content("alpha beta\nz");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('D'));
        assert_eq!(editor.buffer.to_string(), "alpha \nz");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 5);

        editor.handle_key(Key::Char('u'));
        editor.handle_key(Key::Char('C'));
        editor.handle_key(Key::Char('Z'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "alpha Z\nz");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_ctrl_a_and_ctrl_x_adjust_next_number() {
        let mut editor = create_editor_with_content("x=-12 y=9");

        editor.handle_key(Key::Ctrl('a'));
        assert_eq!(editor.buffer.to_string(), "x=-11 y=9");
        assert_eq!(editor.cursor.column(), 2);

        editor.handle_key(Key::Ctrl('x'));
        assert_eq!(editor.buffer.to_string(), "x=-12 y=9");
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_next_number_range_includes_sign_after_separator() {
        let mut editor = create_editor_with_content("x=-12 y=9");
        editor.cursor = Cursor::new(0, 0);

        assert_eq!(editor.next_number_range_on_current_line(), Some((2, 5)));
    }

    #[test]
    fn test_next_number_range_skips_sign_after_previous_digit() {
        let mut editor = create_editor_with_content("v1-23");
        editor.cursor = Cursor::new(0, 2);

        assert_eq!(editor.next_number_range_on_current_line(), Some((3, 5)));
    }

    #[test]
    fn test_next_number_range_skips_sign_without_digits() {
        let mut editor = create_editor_with_content("x=- y=42");
        editor.cursor = Cursor::new(0, 0);

        assert_eq!(editor.next_number_range_on_current_line(), Some((6, 8)));
    }

    #[test]
    fn test_next_number_range_includes_sign_at_line_start() {
        let mut editor = create_editor_with_content("-9");
        editor.cursor = Cursor::new(0, 0);

        assert_eq!(editor.next_number_range_on_current_line(), Some((0, 2)));
    }

    #[test]
    fn test_join_lines_and_replace_char_repeat() {
        let mut editor = create_editor_with_content("one\n  two\nthree");

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('J'));
        assert_eq!(editor.buffer.to_string(), "one two three");
        assert!(editor.mode.is_normal());

        let mut replace_editor = create_editor_with_content("abcd\nabyd");
        replace_editor.cursor = Cursor::new(0, 1);
        replace_editor.handle_key(Key::Char('r'));
        replace_editor.handle_key(Key::Char('X'));
        replace_editor.cursor = Cursor::new(1, 1);
        replace_editor.handle_key(Key::Char('.'));

        assert_eq!(replace_editor.buffer.to_string(), "aXcd\naXyd");
    }

    #[test]
    fn test_star_searches_word_under_cursor() {
        let mut editor = create_editor_with_content("word test word");

        editor.handle_key(Key::Char('*'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 10);
    }

    #[test]
    fn test_ctrl_l_requests_full_redraw() {
        let mut editor = create_editor_with_content("hello");

        assert!(!editor.redraw_requested());
        editor.handle_key(Key::Ctrl('l'));
        assert!(editor.redraw_requested());
        editor.finish_full_render();
        assert!(!editor.redraw_requested());
    }

    #[test]
    fn test_insert_ctrl_t_and_ctrl_d_adjust_current_line_indentation() {
        let mut editor = create_editor_with_content("    value");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Ctrl('d'));
        assert_eq!(editor.buffer.to_string(), "value");
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Ctrl('t'));
        assert_eq!(editor.buffer.to_string(), "    value");
        assert_eq!(editor.cursor.column(), 4);
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
    fn test_dot_repeats_visual_delete_at_current_cursor() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_dot_repeats_visual_change_insert_session_at_current_cursor() {
        let mut editor = create_editor_with_content("abcd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('X'));
        editor.handle_key(Key::Esc);
        editor.cursor = Cursor::new(0, 1);
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "XX");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_dot_repeats_visual_tilde_at_current_cursor() {
        let mut editor = create_editor_with_content("AbCd");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('~'));
        editor.cursor = Cursor::new(0, 2);
        editor.handle_key(Key::Char('.'));

        assert_eq!(editor.buffer.to_string(), "aBcD");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_dot_repeats_visual_indent_over_same_line_count() {
        let mut editor = create_syntax_editor(
            "fn alpha() {\nprintln!(\"a\");\n}\nfn beta() {\nprintln!(\"b\");\n}\n",
            "main.rs",
        );

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('>'));
        editor.cursor = Cursor::new(3, 0);
        editor.handle_key(Key::Char('.'));

        assert_eq!(
            editor.buffer.to_string(),
            "    fn alpha() {\n    println!(\"a\");\n}\n    fn beta() {\n    println!(\"b\");\n}\n"
        );
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_dot_repeats_visual_reindent_over_same_line_count() {
        let mut editor = create_syntax_editor(
            "fn alpha() {\nprintln!(\"a\");\n}\nfn beta() {\nprintln!(\"b\");\n}\n",
            "main.rs",
        );

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('='));
        editor.cursor = Cursor::new(3, 0);
        editor.handle_key(Key::Char('.'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn alpha() {\n    println!(\"a\");\n}\nfn beta() {\n    println!(\"b\");\n}\n"
        );
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_dot_repeats_indent_match_delimiter_operator() {
        let mut editor = create_editor_with_content("if foo {\nbar();\n}\nif bar {\nbaz();\n}\n");

        editor.cursor = Cursor::new(0, 7);
        editor.handle_key(Key::Char('>'));
        editor.handle_key(Key::Char('%'));
        editor.cursor = Cursor::new(3, 7);
        editor.handle_key(Key::Char('.'));

        assert_eq!(
            editor.buffer.to_string(),
            "    if foo {\n    bar();\n    }\n    if bar {\n    baz();\n    }\n"
        );
        assert!(editor.mode.is_normal());
    }

    #[test]
    /// Code actions should clamp the cursor when their edits delete the old cursor line.
    fn test_apply_selected_code_action_clamps_cursor_after_deleted_lines() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "src/main.rs",
            "use std::fmt::Debug;\nuse std::io::Read;\n\nfn main() {}",
        )
        .expect("write main");

        let mut editor = EditorState::new(24);
        editor
            .load_file(tree.path().join("src/main.rs"))
            .expect("load main");
        editor.cursor = Cursor::new(3, 3);

        // Simulate a "remove all unused imports" code action that deletes the
        // import block and blank separator above the cursor.
        editor.apply_selected_code_action(
            &LspCodeAction {
                title: "Remove all unused imports".to_string(),
                edit: LspWorkspaceEdit {
                    document_edits: vec![crate::lsp::protocol::LspDocumentEdit {
                        path: tree.path().join("src/main.rs"),
                        edits: vec![crate::lsp::protocol::LspTextEdit {
                            range: LspRange {
                                start: LspPosition {
                                    line: 0,
                                    character: 0,
                                },
                                end: LspPosition {
                                    line: 3,
                                    character: 0,
                                },
                            },
                            new_text: String::new(),
                        }],
                    }],
                },
            },
            editor.active_buffer_id,
            0,
        );

        assert_eq!(editor.buffer.to_string(), "fn main() {}");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 3);
    }

    #[test]
    /// Code actions should clamp the cursor left when the current line gets shorter.
    fn test_apply_selected_code_action_clamps_cursor_after_shorter_line_edit() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.rs", "let value = helper_value();")
            .expect("write main");

        let mut editor = EditorState::new(24);
        editor
            .load_file(tree.path().join("src/main.rs"))
            .expect("load main");
        editor.cursor = Cursor::new(0, 20);

        // Simulate a code action that replaces a long expression on the current
        // line with a shorter one while leaving the cursor on that line.
        editor.apply_selected_code_action(
            &LspCodeAction {
                title: "Inline constant".to_string(),
                edit: LspWorkspaceEdit {
                    document_edits: vec![crate::lsp::protocol::LspDocumentEdit {
                        path: tree.path().join("src/main.rs"),
                        edits: vec![crate::lsp::protocol::LspTextEdit {
                            range: LspRange {
                                start: LspPosition {
                                    line: 0,
                                    character: 12,
                                },
                                end: LspPosition {
                                    line: 0,
                                    character: 26,
                                },
                            },
                            new_text: "1".to_string(),
                        }],
                    }],
                },
            },
            editor.active_buffer_id,
            0,
        );

        assert_eq!(editor.buffer.to_string(), "let value = 1;");
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 13);
    }
}
