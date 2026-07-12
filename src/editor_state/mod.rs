//! Editor state management
//!
//! The EditorState struct holds all the state for the editor session,
//! including the text buffer, cursor, mode, viewport, and status messages.

use crate::clipboard::{ClipboardPasteRequest, ClipboardRegister, ClipboardWriteRequest};
use crate::command_completion::{
    CommandCompletionDirection, CommandCompletionSession, PendingCommandCompletion,
    build_command_completion_request, build_command_completion_session_for_request,
    build_command_completion_session_from_candidates, retained_async_command_completion_session,
};
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
    PickerPreviewFocus, PickerPreviewState, SearchPickerPollResult, SearchPickerState,
    SearchPickerTarget, SignatureHelpPopup, build_preview_popup,
};
use crate::keybindings::{Action, ActionBinding, Binding, KeyBindings, KeyInput, SequenceMatch};
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
    find_next_paragraph_line, find_next_word_start_with_style, find_prev_paragraph_line,
    find_prev_word_end, find_prev_word_end_with_style, find_prev_word_start_insert_mode,
    find_prev_word_start_with_style, find_word_end_with_style,
};
use crate::path_utils::{current_dir_relative_path, display_path_for_ui};
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
use crate::visible_whitespace::VisibleWhitespace;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use termion::event::Key;

mod actions;
mod auto_insert;
mod buffers;
mod commands;
mod commenting;
mod editing;
pub(crate) mod ex_commands;
mod file_monitor;
mod go_to;
mod history;
mod jump_history;
mod lookup;
mod lsp_edits;
mod macros;
mod matching;
mod operator;
mod prompt_history;
mod registers;
mod repeat;
mod search_count;
mod search_highlighting;
mod substitute_preview;
mod view;

pub(crate) use buffers::BufferSummary;
use buffers::{
    BufferManager, BufferState, OrderedBufferState, absolute_lookup_path, display_file_name,
    normalize_lookup_path, paths_match,
};
use file_monitor::{
    CompletedFingerprint, ExternalFileState, FileFingerprint, FileFingerprintWorker, FileMonitor,
    read_fingerprint_from_disk,
};
use jump_history::JumpHistory;
use lookup::{
    ActiveCodeActionLookup, ActiveHoverLookup, ActiveNavigationLookup, ActiveRenameLookup,
    ActiveSignatureHelpLookup, LookupTokenSource,
};
use macros::{MacroState, PendingMacro};
pub(crate) use matching::VisibleMatchRole;
use operator::{ExecutedOperatorCommand, OperatorKind, PendingOperator, TextObjectPrefix};
use prompt_history::{PromptHistory, PromptHistoryKind, PromptHistoryScope};
use registers::PendingRegister;

const DEFAULT_INDENT_WIDTH: usize = 4;
const DEFAULT_TAB_WIDTH: usize = 8;

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
    SearchPicker,
    LocationPicker,
    DiagnosticPicker,
    CodeActionPicker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOverwrite {
    target_path: PathBuf,
    update_file_path: bool,
    after_write_action: AfterWriteAction,
    reason: OverwritePromptKind,
}

/// Deferred save flow waiting for one asynchronous save-conflict fingerprint result.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSaveConflictCheck {
    request_id: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverwritePromptKind {
    DifferentTargetPath,
    ExternalChange,
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
    /// Whether the prompt corresponds to an unnamed buffer and supports "ignore".
    supports_ignore: bool,
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
        binding: Binding,
        count: Option<usize>,
        register: Option<ClipboardRegister>,
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
    /// Explicit clipboard register targeted by the original visual command, if any.
    register: Option<ClipboardRegister>,
}

/// Distinguish the selection-shaped changes that `.` can replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionRepeatAction {
    Delete,
    Change,
    ToggleCase,
    ToggleLineComment,
    ToggleBlockComment,
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

/// One untouched auto-inserted prefix that may still be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAutoInsertLine {
    /// Logical line index that received the auto-inserted prefix.
    line: usize,
    /// Exact prefix text inserted for that line.
    prefix: String,
    /// Whether leaving Insert mode should preserve the line instead of cleaning it up.
    cleanup_on_exit: bool,
    /// Whether user edits touched the line after insertion.
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

/// Visual severity of a transient status-bar message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StatusMessageKind {
    /// Neutral informational message rendered with the default message-line style.
    #[default]
    Info,
    /// Warning message rendered with a yellow background.
    Warning,
    /// Error message rendered with a red background.
    Error,
}

/// Direction for Vim-style before/after paste placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PastePosition {
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
    ReloadCurrentBuffer,
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
    WriteClipboard(ClipboardWriteRequest),
    PasteClipboard(ClipboardPasteRequest),
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
    auto_reload_external_changes: bool,
    indent_width: usize,
    indent_with_tabs: bool,
    tab_width: usize,
    file_picker_max_files: usize,
    sequence_discovery_popup: bool,
    long_line_column: Option<usize>,
    visible_whitespace: VisibleWhitespace,
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
            auto_reload_external_changes: true,
            indent_width: DEFAULT_INDENT_WIDTH,
            indent_with_tabs: false,
            tab_width: DEFAULT_TAB_WIDTH,
            file_picker_max_files: DEFAULT_FILE_PICKER_MAX_FILES,
            sequence_discovery_popup: true,
            long_line_column: None,
            visible_whitespace: VisibleWhitespace::none(),
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
    /// Last synced disk fingerprint plus any unresolved external-change state.
    external_file: ExternalFileState,
    /// Derived syntax-highlighting state for the current document.
    syntax: SyntaxEngine,
    /// Inactive buffers plus navigation order for all open buffers.
    buffer_manager: BufferManager,
    /// Status message to display on the next render pass.
    status_message: Option<String>,
    /// Whether the current single-line status should stay visible until input arrives.
    status_message_persistent_until_input: bool,
    /// Visual severity of the current status message.
    status_message_kind: StatusMessageKind,
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
    /// Pending visual-mode `i`/`a` text-object prefix waiting for the delimiter key.
    pending_visual_text_object: Option<TextObjectPrefix>,
    /// Pending macro action waiting for one register key.
    pending_macro: Option<PendingMacro>,
    /// Pending Vim-style clipboard register prefix introduced by `"`.
    pending_register: Option<PendingRegister>,
    /// Pending find/till motion waiting for a target character.
    pending_find: Option<FindMotion>,
    /// Pending `r` replacement waiting for the typed replacement character.
    pending_replace: Option<PendingReplace>,
    /// Pending Insert-mode literal insert waiting for the next key after `Ctrl+V`.
    pending_insert_literal: bool,
    /// Last attempted character find/till motion used by ';' and ','.
    last_find: Option<LastFind>,
    /// Last visual selection that can be recreated via normal-mode `gv`.
    last_visual_selection: Option<LastVisualSelection>,
    /// Editor-owned unnamed register used by yank, delete, and paste actions.
    yank_buffer: Option<YankBuffer>,
    /// Session-local macro registers plus active recording/playback state.
    macro_state: MacroState,
    /// Active config replay ids used to detect direct and indirect recursion.
    active_config_replays: Vec<String>,
    /// Pending overwrite confirmation for save commands targeting an existing file.
    pending_overwrite: Option<PendingOverwrite>,
    /// Deferred save flow waiting for one asynchronous save-conflict result.
    pending_save_conflict_check: Option<PendingSaveConflictCheck>,
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
    /// Pending reload confirmation for `:edit` without arguments on a dirty buffer.
    pending_reload_confirmation: bool,
    /// Active buffer-switch picker state while the overlay is open.
    buffer_switch: Option<BufferSwitchState>,
    /// Active file-picker state while the overlay is open.
    file_picker: Option<FilePickerState>,
    /// Active search-results picker state while the overlay is open.
    search_picker: Option<SearchPickerState>,
    /// Active navigation-target picker state while the overlay is open.
    location_picker: Option<LocationPickerState>,
    /// Active diagnostics picker state while the overlay is open.
    diagnostic_picker: Option<DiagnosticPickerState>,
    /// Active code-action picker state while the overlay is open.
    code_action_picker: Option<CodeActionPickerState>,
    /// Shared preview state for picker dialogs that show file content.
    picker_preview: PickerPreviewState,
    /// Registered completion sources available to the insert-mode popup flow.
    completion_sources: CompletionSourceRegistry,
    /// Monotonic generation used to discard stale completion refreshes.
    completion_generation: usize,
    /// Active inline completion session for Insert mode, if any.
    completion_session: Option<CompletionSession>,
    /// Active command-mode completion session for the prompt, if any.
    command_completion_session: Option<CommandCompletionSession>,
    /// In-flight asynchronous command-mode completion request, if any.
    pending_command_completion: Option<PendingCommandCompletion>,
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
    /// Background search-match count for the message bar.
    search_count: search_count::SearchCountState,
    /// Transient `:s` preview state rendered without mutating the committed buffer.
    substitute_preview: Option<substitute_preview::SubstitutePreviewState>,
    /// Wrapping redraw token that forces full redraws when substitute preview changes.
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
    /// Whether swap state has been initialized for the active buffer.
    ///
    /// When `false`, the next buffer activation must run
    /// `load_swap_state_for_active_buffer()` to establish swap ownership
    /// and surface any pending recovery prompts.
    swap_loaded: bool,
    /// Most recent working directory successfully resolved by this process.
    last_known_working_directory: Option<PathBuf>,
    /// Whether one missing-cwd unnamed-swap warning has already been shown.
    missing_working_directory_swap_warning_emitted: bool,
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
    /// Most-recent-first history of buffers visited during this session.
    recent_buffers: VecDeque<usize>,
    /// One untouched auto-inserted prefix that may still be cleaned up.
    pending_auto_insert: Option<PendingAutoInsertLine>,
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
    /// Session-wide external file monitor backend.
    file_monitor: FileMonitor,
    /// Session-wide asynchronous file fingerprint worker.
    file_fingerprint_worker: FileFingerprintWorker,
    /// Latest queued external-change fingerprint request id per monitored path.
    latest_external_fingerprint_request_ids: HashMap<PathBuf, u64>,
    /// Next unique token used to distinguish one on-disk change from the next.
    next_external_change_generation: u64,
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
            external_file: ExternalFileState::default(),
            syntax: SyntaxEngine::new(),
            buffer_manager: BufferManager::new(0),
            status_message: None,
            message_line_needs_clear: false,
            status_message_persistent_until_input: false,
            status_message_kind: StatusMessageKind::Info,
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
            pending_visual_text_object: None,
            pending_macro: None,
            pending_register: None,
            pending_find: None,
            pending_replace: None,
            pending_insert_literal: false,
            last_find: None,
            last_visual_selection: None,
            yank_buffer: None,
            macro_state: MacroState::default(),
            active_config_replays: Vec::new(),
            pending_overwrite: None,
            pending_save_conflict_check: None,
            pending_soft_read_only_save: None,
            pending_quit_confirmation: None,
            pending_session_open_confirmation: None,
            pending_swap_recovery: None,
            pending_buffer_close_confirmation: false,
            pending_reload_confirmation: false,
            buffer_switch: None,
            file_picker: None,
            search_picker: None,
            location_picker: None,
            diagnostic_picker: None,
            code_action_picker: None,
            picker_preview: PickerPreviewState::new(),
            completion_sources: CompletionSourceRegistry::new(),
            completion_generation: 0,
            completion_session: None,
            command_completion_session: None,
            pending_command_completion: None,
            pending_async_completion: None,
            pending_lsp_completion: None,
            pending_lsp_signature_help: None,
            active_lsp_completion: None,
            matching: matching::MatchingState::new(),
            search_highlighting: search_highlighting::SearchHighlightState::new(),
            search_count: search_count::SearchCountState::new(),
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
            swap_loaded: false,
            last_known_working_directory: std::env::current_dir().ok(),
            missing_working_directory_swap_warning_emitted: false,
            last_repeatable_change: None,
            pending_visual_repeat: None,
            last_committed_change_char_idx: None,
            active_insert_repeat: None,
            visual_insert_session: None,
            recent_buffers: VecDeque::new(),
            pending_auto_insert: None,
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
            file_monitor: FileMonitor::new(),
            file_fingerprint_worker: FileFingerprintWorker::new(),
            latest_external_fingerprint_request_ids: HashMap::new(),
            next_external_change_generation: 1,
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

        if let Some(enabled) = settings.auto_reload_external_changes {
            self.settings.auto_reload_external_changes = enabled;
        }

        if let Some(width) = settings.indent_width {
            self.settings.indent_width = width.max(1);
        }

        if let Some(enabled) = settings.indent_with_tabs {
            self.settings.indent_with_tabs = enabled;
        }

        if let Some(width) = settings.tab_width {
            self.settings.tab_width = width.clamp(1, 9_999);
        }

        if let Some(limit) = settings.file_picker_max_files {
            self.settings.file_picker_max_files = limit.max(1);
        }

        if let Some(enabled) = settings.sequence_discovery_popup {
            self.settings.sequence_discovery_popup = enabled;
        }

        if let Some(column) = settings.long_line_column {
            self.settings.long_line_column = Some(column.max(1));
        }

        if let Some(markers) = settings.visible_whitespace {
            self.settings.visible_whitespace = markers;
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
            self.keybindings.set_binding(
                binding.mode,
                binding.key.clone(),
                binding.binding.clone(),
            );
        }
        for binding in &settings.sequence_bindings {
            self.keybindings.set_sequence_binding(
                binding.mode,
                binding.keys.clone(),
                binding.binding.clone(),
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
        self.viewport.set_tab_width(self.settings.tab_width);
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
        BufferState::clamped_buffer_cursor(&self.buffer, line, column)
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
        self.buffer = BufferState::read_named_buffer_from_disk(path, &mut self.external_file)?;
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
        self.record_active_buffer();
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

        let buffer = self.create_buffer_state(path)?;
        self.buffer_manager.push_new_id(buffer.id);
        self.activate_inactive_buffer(buffer);
        Ok(())
    }

    /// Open one buffer from `:edit`, replacing the default startup buffer when safe.
    pub(crate) fn open_buffer_from_edit(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if paths_match(&self.file_path, path) {
            return Ok(());
        }
        if self.should_replace_default_unnamed_buffer_on_edit() {
            // Reuse the initial active slot so `:edit` does not leave a redundant
            // unnamed buffer behind when the session still has only startup state.
            if path.exists() {
                self.load_file(path)?;
            } else {
                self.set_startup_path(path);
            }
            // Replacing the active buffer in-place does not go through
            // `activate_inactive_buffer`, so swap must be loaded explicitly.
            self.load_swap_state_for_active_buffer();
            return Ok(());
        }
        self.open_buffer(path)
    }

    /// Return whether `:edit` should replace the default unnamed startup buffer.
    ///
    /// Returns `true` when the active buffer is unnamed, empty, unmodified, and
    /// the only open buffer, and `false` for every other editor state.
    fn should_replace_default_unnamed_buffer_on_edit(&self) -> bool {
        self.file_path.as_os_str().is_empty()
            && self.buffer.chars_count() == 0
            && !self.buffer.is_modified()
            && self.buffer_manager.has_single_buffer()
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
        self.external_file.sync_to_missing_file();
        self.refresh_syntax();
        self.lsp_document_version = 0;
        self.pending_lsp_changes.clear();
        self.pending_lsp_sync_at = (!self.file_path.as_os_str().is_empty()).then(Instant::now);
        self.clear_active_lookup_state();
        self.hover_popup = None;
        self.dismiss_signature_help();
        self.record_active_buffer();
    }

    /// Build one `BufferState` for `path` without registering it in the buffer manager.
    ///
    /// Reads the file from disk when it exists, or produces a named-empty buffer
    /// for paths that do not yet exist. The caller is responsible for assigning
    /// the buffer id, pushing it into the buffer manager, and either activating
    /// or parking the result.
    fn create_buffer_state(&mut self, path: &Path) -> io::Result<BufferState> {
        let buffer_id = self.buffer_manager.allocate_id();
        if path.exists() {
            BufferState::from_file(
                buffer_id,
                self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
                path,
            )
        } else {
            Ok(BufferState::new_named_empty(
                buffer_id,
                self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
                path,
            ))
        }
    }

    /// Create one buffer for `path` and park it as inactive without activating.
    ///
    /// Skips creation when the path already matches the active buffer or an
    /// existing inactive buffer, so duplicate CLI arguments do not produce
    /// redundant buffer entries. Swap files are only created when the user
    /// actually switches to each buffer.
    pub(crate) fn park_startup_buffer(&mut self, path: &Path) -> io::Result<()> {
        if paths_match(&self.file_path, path) {
            return Ok(());
        }
        if self.buffer_manager.has_inactive_path(path) {
            return Ok(());
        }
        let buffer = self.create_buffer_state(path)?;
        self.buffer_manager.push_new_id(buffer.id);
        self.buffer_manager.store_inactive(buffer);
        Ok(())
    }

    /// Return the normalized named file paths that should be observed for external changes.
    fn named_file_paths_for_monitor(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(path) = absolute_lookup_path(&self.file_path) {
            paths.push(path);
        }
        for buffer in self.buffer_manager.inactive_buffers() {
            if let Some(path) = absolute_lookup_path(&buffer.file_path) {
                paths.push(path);
            }
        }
        paths
    }

    /// Return whether the editor currently tracks at least one named file path.
    ///
    /// Returns `true` when the active buffer or any inactive buffer has a named
    /// path that should keep external-file monitoring alive, and `false` when
    /// every open buffer is unnamed.
    fn has_named_file_paths(&self) -> bool {
        if !self.file_path.as_os_str().is_empty() {
            return true;
        }
        // Inactive buffers are checked lazily so the common unnamed-startup path
        // can short-circuit without iterating over the parked buffer list.
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .any(|buffer| !buffer.file_path.as_os_str().is_empty())
    }

    /// Return whether the active buffer currently shows an unresolved external-change prompt.
    ///
    /// Returns `true` when the active buffer still needs a reload-or-ignore
    /// decision, and `false` when no external-change prompt is active.
    fn active_external_change_prompt_active(&self) -> bool {
        self.external_file.prompt_is_active()
    }

    /// Return whether a clean buffer should auto-reload for the given fingerprint.
    ///
    /// Returns `true` when the buffer has no local edits, auto-reload is enabled,
    /// and the changed path still exists on disk, and `false` otherwise.
    fn should_auto_reload_clean_buffer(
        &self,
        fingerprint: &FileFingerprint,
        modified: bool,
    ) -> bool {
        !modified
            && self.settings.auto_reload_external_changes
            && matches!(fingerprint, FileFingerprint::Present(_))
    }

    /// Synchronize the active monitored path set and process any queued external file changes.
    fn poll_external_file_changes(&mut self) {
        self.file_monitor
            .sync_paths(&self.named_file_paths_for_monitor());
        if let Some(warning) = self.file_monitor.take_warning() {
            self.show_warning_message(warning);
        }

        // The monitor only reports candidate paths. Fingerprints are queued for
        // asynchronous computation before mutating any buffer state.
        for path in self.file_monitor.poll_changed_paths() {
            self.queue_external_fingerprint_request(&path);
        }
    }

    /// Queue one external-change fingerprint request for asynchronous processing.
    fn queue_external_fingerprint_request(&mut self, changed_path: &Path) {
        let request_id = self.file_fingerprint_worker.queue_request(changed_path);
        self.latest_external_fingerprint_request_ids
            .insert(changed_path.to_path_buf(), request_id);
    }

    /// Drain completed asynchronous fingerprint requests and apply accepted results.
    fn poll_file_fingerprint_results(&mut self) {
        if let Some(warning) = self.file_fingerprint_worker.take_warning() {
            self.show_warning_message(warning);
        }

        for completed in self.file_fingerprint_worker.poll_completed() {
            self.handle_completed_fingerprint(completed);
        }
    }

    /// Route one completed fingerprint to save-conflict or external-change handling.
    fn handle_completed_fingerprint(&mut self, completed: CompletedFingerprint) {
        // Save-conflict checks consume their exact request id so external-change
        // filtering cannot accidentally steal save-specific responses.
        if self
            .pending_save_conflict_check
            .as_ref()
            .is_some_and(|pending| pending.request_id == completed.request_id)
        {
            let pending = self.pending_save_conflict_check.take();
            if let Some(pending) = pending {
                self.handle_save_conflict_fingerprint(pending, completed.result);
            }
            return;
        }

        let latest_request_id = self
            .latest_external_fingerprint_request_ids
            .get(&completed.path)
            .copied();
        // Newer queued requests for the same path supersede stale completions.
        if latest_request_id != Some(completed.request_id) {
            return;
        }
        self.latest_external_fingerprint_request_ids
            .remove(&completed.path);

        match completed.result {
            Ok(fingerprint) => {
                self.apply_external_fingerprint_for_path(&completed.path, fingerprint)
            }
            Err(error) => self.show_error_message(format!(
                "Failed to inspect external changes for {}: {error}",
                display_path_for_ui(&completed.path)
            )),
        }
    }

    /// Apply one changed on-disk path to the matching open buffer, if any.
    #[cfg(test)]
    fn apply_external_path_change(&mut self, changed_path: &Path) {
        let fingerprint = match read_fingerprint_from_disk(changed_path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.show_error_message(format!(
                    "Failed to inspect external changes for {}: {error}",
                    display_path_for_ui(changed_path)
                ));
                return;
            }
        };

        self.apply_external_fingerprint_for_path(changed_path, fingerprint);
    }

    /// Apply one changed-path fingerprint to the matching open buffer, if any.
    fn apply_external_fingerprint_for_path(
        &mut self,
        changed_path: &Path,
        fingerprint: FileFingerprint,
    ) {
        if self
            .active_named_file_path()
            .is_some_and(|path| paths_match(path, changed_path))
        {
            self.apply_active_external_fingerprint(fingerprint);
            return;
        }

        for buffer in self.buffer_manager.inactive_buffers_mut() {
            if buffer
                .named_file_path()
                .is_some_and(|path| paths_match(path, changed_path))
            {
                Self::apply_inactive_external_fingerprint(
                    buffer,
                    fingerprint.clone(),
                    self.settings.auto_reload_external_changes,
                    &mut self.next_external_change_generation,
                );
                return;
            }
        }
    }

    /// Apply one newly observed fingerprint to the active buffer's external-change state.
    fn apply_active_external_fingerprint(&mut self, fingerprint: FileFingerprint) {
        if self.external_file.synced.is_none() {
            return;
        }
        if self
            .external_file
            .synced
            .as_ref()
            .is_some_and(|synced| synced == &fingerprint)
        {
            self.external_file.pending_change = None;
            return;
        }

        // Clean buffers can safely reload in place when the setting is enabled
        // and the changed path still points at readable file contents.
        if self.should_auto_reload_clean_buffer(&fingerprint, self.buffer.is_modified()) {
            self.reload_active_buffer_after_external_change();
            return;
        }

        self.external_file
            .update_pending_change(fingerprint, &mut self.next_external_change_generation);
    }

    /// Apply one newly observed fingerprint to an inactive buffer's external-change state.
    fn apply_inactive_external_fingerprint(
        buffer: &mut BufferState,
        fingerprint: FileFingerprint,
        auto_reload_external_changes: bool,
        next_external_change_generation: &mut u64,
    ) {
        if buffer.external_file.synced.is_none() {
            return;
        }
        if buffer
            .external_file
            .synced
            .as_ref()
            .is_some_and(|synced| synced == &fingerprint)
        {
            buffer.external_file.pending_change = None;
            return;
        }

        // Hidden clean buffers reload immediately so they are up to date when the
        // user returns, but the user-facing notice is deferred until activation.
        if !buffer.buffer.is_modified()
            && auto_reload_external_changes
            && matches!(fingerprint, FileFingerprint::Present(_))
        {
            match buffer.reload_from_disk() {
                Ok(Some(warning)) => buffer.external_file.deferred_notice = Some(warning),
                Ok(None) => {
                    buffer.external_file.deferred_notice = Some(format!(
                        "\"{}\" reloaded after external change",
                        display_path_for_ui(&buffer.file_path)
                    ))
                }
                Err(error) => {
                    buffer.external_file.deferred_notice = Some(format!(
                        "Failed to reload {} after external change: {error}",
                        display_path_for_ui(&buffer.file_path)
                    ))
                }
            }
            return;
        }

        buffer
            .external_file
            .update_pending_change(fingerprint, next_external_change_generation);
    }

    /// Reload the active buffer after an external change and surface the result.
    fn reload_active_buffer_after_external_change(&mut self) {
        match self.reload_active_buffer_from_disk() {
            Ok(Some(warning)) => self.show_warning_message(warning),
            Ok(None) => self.show_status_message(format!(
                "\"{}\" reloaded after external change",
                display_path_for_ui(&self.file_path)
            )),
            Err(error) => self.show_error_message(format!(
                "Failed to reload {} after external change: {error}",
                display_path_for_ui(&self.file_path)
            )),
        }
    }

    /// Reload the active buffer from disk and show a success message.
    pub(crate) fn reload_active_buffer_manually(&mut self) {
        match self.reload_active_buffer_from_disk() {
            Ok(Some(warning)) => self.show_warning_message(warning),
            Ok(None) => self.show_status_message(format!(
                "\"{}\" reloaded",
                display_path_for_ui(&self.file_path)
            )),
            Err(error) => self.show_error_message(format!(
                "Failed to reload {}: {error}",
                display_path_for_ui(&self.file_path)
            )),
        }
    }

    /// Reload the active buffer from disk while preserving its viewport focus as much as possible.
    ///
    /// Returns `Ok(Some(message))` when the reload succeeded but follow-up work
    /// such as swap refresh still needs one warning, `Ok(None)` when the reload
    /// completed without any extra status message, and `Err(error)` when reading
    /// the backing file failed.
    fn reload_active_buffer_from_disk(&mut self) -> io::Result<Option<String>> {
        let terminal_height = self.viewport.height() + Self::RESERVED_SCREEN_ROWS;
        let placeholder = BufferState::new_empty(self.active_buffer_id, terminal_height);
        let mut active_state = self.replace_active_buffer_state(placeholder);
        let reload_result = active_state.reload_from_disk();
        let _ = self.replace_active_buffer_state(active_state);
        self.refresh_active_read_only_state();
        self.clear_active_lookup_state();
        self.hover_popup = None;
        self.dismiss_signature_help();
        reload_result
    }

    /// Show any deferred hidden-buffer auto-reload notice after the buffer becomes active.
    fn present_active_external_notice(&mut self) {
        if let Some(message) = self.external_file.take_deferred_notice() {
            self.show_status_message(message);
        }
    }

    /// Queue one deferred save-conflict fingerprint check for `target_path`.
    ///
    /// Returns `true` when a deferred check was queued and write completion must
    /// wait for a background fingerprint result, and `false` when no deferred
    /// save-conflict check is needed for this write.
    fn enqueue_save_conflict_check(
        &mut self,
        target_path: PathBuf,
        update_file_path: bool,
        after_write_action: AfterWriteAction,
    ) -> bool {
        if !paths_match(&self.file_path, &target_path) || self.external_file.synced.is_none() {
            return false;
        }
        if self.pending_save_conflict_check.is_some() {
            self.show_error_message("Write already waiting for external change check");
            return true;
        }

        // Save completion resumes after this fingerprint result arrives.
        let request_id = self.file_fingerprint_worker.queue_request(&target_path);
        self.pending_save_conflict_check = Some(PendingSaveConflictCheck {
            request_id,
            target_path,
            update_file_path,
            after_write_action,
        });
        self.show_status_message("Checking external changes before write...");
        true
    }

    /// Handle one completed asynchronous save-conflict fingerprint result.
    fn handle_save_conflict_fingerprint(
        &mut self,
        pending: PendingSaveConflictCheck,
        result: io::Result<FileFingerprint>,
    ) {
        // Buffer switches can invalidate the deferred check before it completes.
        if !paths_match(&self.file_path, &pending.target_path)
            || self.external_file.synced.is_none()
        {
            return;
        }

        let fingerprint = match result {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.show_error_message(format!(
                    "Failed to verify external changes for {}: {error}",
                    display_path_for_ui(&pending.target_path)
                ));
                return;
            }
        };

        if self.resolve_save_conflict_from_fingerprint(fingerprint) {
            self.pending_overwrite = Some(PendingOverwrite {
                target_path: pending.target_path,
                update_file_path: pending.update_file_path,
                after_write_action: pending.after_write_action,
                reason: OverwritePromptKind::ExternalChange,
            });
            self.clear_status_message();
            return;
        }

        self.queue_write_request(
            pending.target_path,
            pending.update_file_path,
            pending.after_write_action,
        );
    }

    /// Resolve one save-conflict check from a completed disk fingerprint.
    ///
    /// Returns `true` when the save must show overwrite confirmation because the
    /// on-disk content differs from the synced baseline, and `false` when no
    /// overwrite prompt is needed and the save may continue.
    fn resolve_save_conflict_from_fingerprint(&mut self, fingerprint: FileFingerprint) -> bool {
        if self
            .external_file
            .synced
            .as_ref()
            .is_some_and(|synced| synced == &fingerprint)
        {
            self.external_file.pending_change = None;
            return false;
        }

        self.external_file
            .update_pending_change(fingerprint, &mut self.next_external_change_generation);
        true
    }

    /// Refresh the active current-file conflict state before queuing one in-place save.
    ///
    /// Returns `Ok(true)` when the save target has changed on disk relative to the
    /// synced baseline and the caller must trigger overwrite confirmation. Returns
    /// `Ok(false)` when no overwrite prompt is required because the save target is
    /// not the active synced file or the on-disk fingerprint still matches.
    fn check_external_save_conflict_sync(&mut self, target_path: &Path) -> io::Result<bool> {
        if !paths_match(&self.file_path, target_path) || self.external_file.synced.is_none() {
            return Ok(false);
        }

        let fingerprint = read_fingerprint_from_disk(target_path)?;
        Ok(self.resolve_save_conflict_from_fingerprint(fingerprint))
    }

    /// Build a serializable snapshot of the current project session.
    pub(crate) fn build_project_session(&self, working_directory: PathBuf) -> ProjectSession {
        let ordered_buffers = self.ordered_project_buffers();
        let active_buffer = ordered_buffers
            .iter()
            .position(|buffer| buffer.active)
            .unwrap_or(0);
        // The alternate buffer is the most recently visited buffer that is not
        // the active one, matching the runtime `goto_alternate_file` target.
        let alternate_buffer = self
            .recent_buffers
            .iter()
            .find(|&&buffer_id| buffer_id != self.active_buffer_id)
            .and_then(|&alternate_id| {
                ordered_buffers
                    .iter()
                    .position(|buffer| buffer.id == alternate_id)
            });
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
            alternate_buffer,
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
            external_file,
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
            pending_swap_recovery,
            swap_loaded,
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
            external_file: std::mem::replace(&mut self.external_file, external_file),
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
            pending_swap_recovery: std::mem::replace(
                &mut self.pending_swap_recovery,
                pending_swap_recovery,
            ),
            swap_loaded: std::mem::replace(&mut self.swap_loaded, swap_loaded),
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
        self.viewport.set_tab_width(self.settings.tab_width);
        self.viewport
            .set_horizontal_scroll_margin(self.settings.horizontal_scroll_margin);
        previous
    }

    /// Park the current active buffer and activate one inactive buffer in its place.
    fn activate_inactive_buffer(&mut self, target: BufferState) {
        let previous = self.replace_active_buffer_state(target);
        self.buffer_manager.store_inactive(previous);
        self.record_active_buffer();
        self.reset_mode_for_buffer_switch();
        self.present_active_external_notice();
        // Deferred swap initialization: load swap state when a buffer becomes
        // active for the first time so inactive buffers never create swap files.
        if !self.swap_loaded {
            self.load_swap_state_for_active_buffer();
        }
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
            // Re-activating the current buffer is a no-op unless swap state
            // has not been initialized yet (e.g. the first startup buffer).
            if !self.swap_loaded {
                self.load_swap_state_for_active_buffer();
            }
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
        self.pending_buffer_close_confirmation = false;
        self.buffer_switch = None;
        self.clear_picker_and_hover_state();
        self.clear_status_message();
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
            let buffer_id = self.restore_project_session_buffer(buffer, index == 0)?;
            buffer_ids.push(buffer_id);
        }

        if let Some(&active_id) = buffer_ids.get(session.active_buffer) {
            self.activate_buffer(active_id);
        }

        // Seed the recent-buffer history with the saved alternate so
        // `goto_alternate_file` works immediately after session restore.
        // `push_back` places the alternate after the active entry, matching
        // the most-recent-first order that `record_active_buffer` maintains
        // during normal buffer switching (active first, alternate second).
        if let Some(alternate_index) = session.alternate_buffer
            && let Some(&alternate_id) = buffer_ids.get(alternate_index)
            && alternate_id != self.active_buffer_id
        {
            self.recent_buffers
                .retain(|&buffer_id| buffer_id != alternate_id);
            self.recent_buffers.push_back(alternate_id);
        }
        Ok(())
    }

    /// Restore one saved buffer entry and return its buffer id.
    fn restore_project_session_buffer(
        &mut self,
        buffer: &SessionBuffer,
        first_buffer: bool,
    ) -> io::Result<usize> {
        if first_buffer {
            self.restore_first_project_session_buffer(buffer)?;
            // First buffer is loaded into the active slot, so the cursor
            // can be restored directly on the active buffer fields.
            self.restore_active_project_session_cursor(&buffer.cursor);
            return Ok(self.active_buffer_id);
        }

        self.restore_additional_project_session_buffer(buffer)
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

    /// Restore one additional saved buffer after the first entry and return its id.
    fn restore_additional_project_session_buffer(
        &mut self,
        session_buffer: &SessionBuffer,
    ) -> io::Result<usize> {
        if session_buffer.path.as_os_str().is_empty() {
            // Session restore must preserve unnamed buffers as distinct entries in
            // the buffer list instead of collapsing them into the current buffer.
            // Park as inactive so swap files are only created on activation.
            let buffer_id = self.buffer_manager.allocate_id();
            let buffer = BufferState::new_empty(
                buffer_id,
                self.viewport.height() + Self::RESERVED_SCREEN_ROWS,
            );
            self.buffer_manager.push_new_id(buffer_id);
            self.buffer_manager.store_inactive(buffer);
            return Ok(buffer_id);
        }

        let mut buffer = self.create_buffer_state(&session_buffer.path)?;
        // Set the restored cursor on the BufferState before parking so the
        // inactive snapshot carries the correct position without needing a
        // post-hoc fixup through the buffer manager.
        buffer.cursor = BufferState::clamped_buffer_cursor(
            &buffer.buffer,
            session_buffer.cursor.line(),
            session_buffer.cursor.column(),
        );
        buffer
            .viewport
            .ensure_cursor_visible(&buffer.cursor, &buffer.buffer);
        let buffer_id = buffer.id;
        self.buffer_manager.push_new_id(buffer_id);
        self.buffer_manager.store_inactive(buffer);
        Ok(buffer_id)
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
            Mode::SearchPicker(_) => Some(PickerKind::SearchPicker),
            Mode::LocationPicker(_) => Some(PickerKind::LocationPicker),
            Mode::DiagnosticPicker(_) => Some(PickerKind::DiagnosticPicker),
            Mode::CodeActionPicker(_) => Some(PickerKind::CodeActionPicker),
            _ => None,
        }
    }

    /// Return the user-facing path label shown in the preview pane.
    fn picker_preview_display_path(path: &Path) -> String {
        display_path_for_ui(path)
    }

    /// Build one preview popup for an open buffer identified by `buffer_id`.
    fn picker_preview_popup_for_buffer_id(
        &self,
        buffer_id: usize,
        focus: PickerPreviewFocus,
    ) -> Option<crate::dialogs::PickerPreviewPopup> {
        // Open-buffer previews reuse the live in-memory text and syntax state so
        // unsaved edits appear immediately without touching the filesystem.
        if buffer_id == self.active_buffer_id {
            return Some(build_preview_popup(
                &self.buffer,
                &self.syntax,
                buffers::display_buffer_path(&self.file_path, buffer_id),
                focus,
            ));
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| buffer.id == buffer_id)
            .map(|buffer| {
                build_preview_popup(&buffer.buffer, &buffer.syntax, buffer.display_path(), focus)
            })
    }

    /// Build one preview popup for an already-open path, if a live buffer exists.
    fn picker_preview_popup_for_open_path(
        &self,
        path: &Path,
        focus: PickerPreviewFocus,
    ) -> Option<crate::dialogs::PickerPreviewPopup> {
        // Path lookups prefer active or inactive open buffers so preview content
        // stays aligned with the editor's unsaved state instead of stale disk data.
        if paths_match(&self.file_path, path) {
            return Some(build_preview_popup(
                &self.buffer,
                &self.syntax,
                Self::picker_preview_display_path(path),
                focus,
            ));
        }
        self.buffer_manager
            .inactive_buffers()
            .iter()
            .find(|buffer| paths_match(&buffer.file_path, path))
            .map(|buffer| {
                build_preview_popup(
                    &buffer.buffer,
                    &buffer.syntax,
                    Self::picker_preview_display_path(path),
                    focus,
                )
            })
    }

    /// Refresh the picker preview so it matches the currently selected row.
    fn refresh_picker_preview(&mut self) {
        let Some(picker) = self.active_picker_kind() else {
            self.picker_preview.clear();
            return;
        };

        match picker {
            PickerKind::BufferSwitch => self.refresh_buffer_switch_picker_preview(),
            PickerKind::FilePicker => self.refresh_file_picker_preview(),
            PickerKind::SearchPicker => self.refresh_search_picker_preview(),
            PickerKind::LocationPicker => self.refresh_location_picker_preview(),
            PickerKind::DiagnosticPicker | PickerKind::CodeActionPicker => {
                self.clear_picker_preview();
            }
        }
    }

    /// Clear the shared picker preview when the active picker has no preview support.
    fn clear_picker_preview(&mut self) {
        self.picker_preview.clear();
    }

    /// Refresh the buffer-switch preview for the selected open buffer.
    fn refresh_buffer_switch_picker_preview(&mut self) {
        // Buffer previews always come from live in-memory state so unsaved edits
        // stay visible without touching the filesystem.
        let Some(buffer_id) = self
            .buffer_switch
            .as_ref()
            .and_then(BufferSwitchState::selected_buffer_id)
        else {
            // No buffer is selected, so show an empty preview pane to prevent
            // the picker layout from shifting.
            self.picker_preview.show_empty("none".to_string());
            return;
        };
        if let Some(popup) =
            self.picker_preview_popup_for_buffer_id(buffer_id, PickerPreviewFocus::Top)
        {
            self.picker_preview
                .show_sync(format!("buffer:{buffer_id}"), popup);
        } else {
            self.picker_preview.show_empty("none".to_string());
        }
    }

    /// Refresh the file-picker preview for the selected filesystem path.
    fn refresh_file_picker_preview(&mut self) {
        // File-picker rows point at filesystem paths, so preview resolution can
        // delegate straight to the shared path-backed preview helper.
        let Some(path) = self
            .file_picker
            .as_ref()
            .and_then(FilePickerState::selected_path)
            .map(PathBuf::from)
        else {
            // No file is selected, so show an empty preview pane to prevent the
            // picker layout from shifting.
            self.picker_preview.show_empty("none".to_string());
            return;
        };
        self.refresh_path_picker_preview(
            format!("file:{}", path.display()),
            path,
            PickerPreviewFocus::Top,
        );
    }

    /// Refresh the search-results preview for the selected match target.
    fn refresh_search_picker_preview(&mut self) {
        // Search results preview the matched file and center the selected hit so
        // the surrounding context stays visible while moving between matches.
        let Some(target) = self
            .search_picker
            .as_ref()
            .and_then(SearchPickerState::selected_target)
        else {
            // No search result is selected, so show an empty preview pane to
            // prevent the picker layout from shifting.
            self.picker_preview.show_empty("none".to_string());
            return;
        };
        self.refresh_path_picker_preview(
            format!("search:{}:{}", target.file_path.display(), target.line),
            target.file_path,
            PickerPreviewFocus::Center(target.line),
        );
    }

    /// Refresh the location-picker preview for the selected navigation target.
    fn refresh_location_picker_preview(&mut self) {
        // Location pickers share the same centered-target behavior as search
        // results, but their targets come from LSP navigation rows.
        let Some(target) = self
            .location_picker
            .as_ref()
            .and_then(LocationPickerState::selected_target)
            .cloned()
        else {
            // No location is selected, so show an empty preview pane to prevent
            // the picker layout from shifting.
            self.picker_preview.show_empty("none".to_string());
            return;
        };
        self.refresh_path_picker_preview(
            format!("location:{}:{}", target.file_path.display(), target.line),
            target.file_path,
            PickerPreviewFocus::Center(target.line),
        );
    }

    /// Refresh one path-backed picker preview from live buffers or disk.
    fn refresh_path_picker_preview(
        &mut self,
        key: String,
        path: PathBuf,
        focus: PickerPreviewFocus,
    ) {
        // Path-backed pickers share the same precedence: prefer a live open
        // buffer, otherwise fall back to an async disk-backed preview load.
        if let Some(popup) = self.picker_preview_popup_for_open_path(&path, focus) {
            self.picker_preview.show_sync(key, popup);
            return;
        }
        self.picker_preview.load_file(
            key,
            path.clone(),
            Self::picker_preview_display_path(&path),
            focus,
        );
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
    fn current_buffer_identifier_pattern(&self) -> IdentifierPattern {
        if self.file_path.as_os_str().is_empty() {
            return ascii_identifier();
        }
        let path = self.file_path.as_path();
        match detect_language_details(Some(path)) {
            Some((profile, _)) => profile.identifier,
            None => ascii_identifier(),
        }
    }

    /// Return whether `ch` belongs to an identifier-like word in this buffer.
    ///
    /// Returns `true` for characters that the active syntax profile allows in a
    /// buffer-specific identifier, and `false` for separators or punctuation.
    fn is_identifier_char_in_current_buffer(&self, ch: char) -> bool {
        identifier_can_continue(self.current_buffer_identifier_pattern(), ch)
    }

    /// Open the buffer-switch picker with the current ordered buffer list.
    fn open_buffer_switcher(&mut self) {
        self.prepare_picker_open();
        self.buffer_switch = Some(BufferSwitchState::new(self.buffer_switch_items()));
        self.mode = Mode::buffer_switch_empty();
        self.refresh_picker_preview();
    }

    /// Build buffer-switch picker rows with the active buffer pinned first.
    fn buffer_switch_items(&self) -> Vec<BufferSwitchItem> {
        let recent_ranks = self
            .recent_buffers
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
                // Keep the active buffer pinned, then sort all other buffers by session recency
                // regardless of whether they are named so the alternate file stays near the top.
                let sort_group = if summary.active {
                    0
                } else if let Some(rank) = recent_ranks.get(&summary.id) {
                    return (1, *rank, open_order, summary);
                } else {
                    2
                };
                (sort_group, open_order, open_order, summary)
            })
            .collect::<Vec<_>>();

        // Preserve stable open-buffer order inside each fallback group.
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
        self.picker_preview.clear();
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
                self.show_error_message(format!("Failed to read working directory: {error}"));
                return;
            }
        };

        self.prepare_picker_open();
        self.file_picker = Some(FilePickerState::new(
            root,
            self.settings.file_picker_max_files,
        ));
        self.mode = Mode::file_picker_empty();
        self.refresh_picker_preview();
    }

    /// Open the async search-results picker for one regex pattern.
    fn open_search_picker(&mut self, pattern: String) {
        let query = match SearchQuery::compile(&pattern) {
            Ok(query) => query,
            Err(error) => {
                self.show_error_message(format!("Invalid regex:\n{error}"));
                return;
            }
        };
        let root = match std::env::current_dir() {
            Ok(root) => root,
            Err(error) => {
                self.show_error_message(format!("Failed to read working directory: {error}"));
                return;
            }
        };

        // The picker owns only the fuzzy-filter query while the command-supplied regex remains fixed.
        self.prepare_picker_open();
        self.search_picker = Some(SearchPickerState::new(root, pattern, query));
        self.mode = Mode::search_picker_empty();
    }

    /// Close the file picker without opening a selection.
    fn close_file_picker(&mut self) {
        if let Some(picker) = &mut self.file_picker {
            picker.cancel();
        }
        self.file_picker = None;
        self.picker_preview.clear();
        self.mode = Mode::Normal;
    }

    /// Close the search-results picker without opening a selection.
    fn close_search_picker(&mut self) {
        if let Some(picker) = &mut self.search_picker {
            picker.cancel();
        }
        self.search_picker = None;
        self.picker_preview.clear();
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
        self.refresh_picker_preview();
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
        self.picker_preview.clear();
        self.mode = Mode::Normal;
    }

    /// Close the diagnostics picker without applying a selection.
    fn close_diagnostics_picker(&mut self) {
        self.diagnostic_picker = None;
        self.picker_preview.clear();
        self.mode = Mode::Normal;
    }

    /// Close the code-action picker without applying a selection.
    fn close_code_action_picker(&mut self) {
        self.code_action_picker = None;
        self.picker_preview.clear();
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
        if self.goto_navigation_target(&target) {
            self.center_cursor_after_picker_jump();
        }
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
        if self.goto_active_buffer_diagnostic(selected_index) {
            self.center_cursor_after_picker_jump();
        }
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
    ///
    /// Opens the selected path with `:edit` semantics: the default unnamed
    /// startup buffer is replaced in place when it is still pristine, and
    /// every other state adds or reactivates a buffer.
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
        if let Err(error) = self.open_buffer_from_edit(&path) {
            self.show_error_message(format!("Failed to open \"{path}\": {error}"));
        }
    }

    /// Confirm the current search-picker selection, if one is available.
    fn confirm_search_picker_selection(&mut self) {
        let Some(target) = self
            .search_picker
            .as_ref()
            .and_then(SearchPickerState::selected_target)
        else {
            return;
        };

        self.close_search_picker();
        self.goto_search_picker_target(&target);
    }

    /// Center the active cursor after one picker-confirmed jump.
    fn center_cursor_after_picker_jump(&mut self) {
        self.viewport
            .align_cursor_center(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
    }

    /// Poll background picker and completion work plus any due swap refreshes.
    pub(crate) fn poll_background_tasks(&mut self) {
        if let Some(query) = self.mode.file_picker_string().map(str::to_string)
            && let Some(picker) = &mut self.file_picker
        {
            let selected_before_poll = picker.selected_path().map(str::to_string);
            let FilePickerPollResult {
                changed: picker_changed,
                status_message,
            } = picker.poll(&query);
            let selected_after_poll = picker.selected_path().map(str::to_string);
            if let Some(status_message) = status_message {
                self.show_status_message(status_message);
            }
            // File-scan batches often append rows that are off-screen; preview work only
            // needs to run when the actively previewed selection actually changes.
            if picker_changed && selected_before_poll != selected_after_poll {
                self.refresh_picker_preview();
            }
        }
        if let Some(query) = self.mode.picker_string().map(str::to_string)
            && let Some(picker) = &mut self.search_picker
            && matches!(self.mode, Mode::SearchPicker(_))
        {
            let SearchPickerPollResult {
                changed: picker_changed,
                status_message,
            } = picker.poll(&query);
            if let Some(status_message) = status_message {
                self.show_status_message(status_message);
            }
            if picker_changed {
                self.refresh_picker_preview();
            }
        }
        let _ = self.picker_preview.poll();

        self.poll_command_completion_background_tasks();
        self.poll_completion_background_tasks();
        self.flush_due_swap_refresh();
        self.poll_external_file_changes();
        self.poll_file_fingerprint_results();
        self.search_count.poll();
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
        self.clear_status_message();
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
    ///
    /// Returns `true` when the destination diagnostic exists and the cursor jump
    /// is applied, and `false` when no matching diagnostic could be resolved.
    fn goto_active_buffer_diagnostic(&mut self, diagnostic_index: usize) -> bool {
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
            return false;
        };
        if !self.record_jump_origin_for_destination(&self.file_path.clone(), line, character) {
            self.show_status_message(label);
            return false;
        }
        self.move_cursor_to_lsp_position(line, character);
        self.show_status_message(label);
        true
    }

    /// Return whether the app loop should poll for asynchronous picker updates.
    ///
    /// Returns `true` when picker work, a queued app-layer request, or a pending
    /// swap flush needs a timed wakeup, and `false` when the editor can stay on
    /// the blocking input path.
    pub(crate) fn needs_background_poll(&self) -> bool {
        self.file_picker
            .as_ref()
            .is_some_and(FilePickerState::is_scanning)
            || self
                .search_picker
                .as_ref()
                .is_some_and(SearchPickerState::is_searching)
            || self.picker_preview.is_loading()
            || self.pending_command_completion.is_some()
            || self.pending_request.is_some()
            || self.pending_save_conflict_check.is_some()
            || self.pending_async_completion.is_some()
            || self.pending_lsp_completion.is_some()
            || self.pending_lsp_signature_help.is_some()
            || self.active_lsp_completion.is_some()
            || self.active_signature_help_lookup.is_some()
            || self.pending_swap_refresh_at.is_some()
            || self.pending_lsp_sync_at.is_some()
            || self.has_named_file_paths()
            || self.search_count.should_background_poll()
    }

    /// Return whether file-picker scan or deferred filter work is currently active.
    ///
    /// Returns `true` when the file picker still has background scan/filter work
    /// in flight, and `false` when no file-picker background work remains.
    pub(crate) fn file_picker_background_active(&self) -> bool {
        self.file_picker
            .as_ref()
            .is_some_and(FilePickerState::is_scanning)
    }

    /// Queue one navigation lookup for the current cursor position.
    fn request_navigation(&mut self, kind: NavigationKind) {
        if self.file_path.as_os_str().is_empty() {
            self.show_error_message(kind.unavailable_file_message());
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
        self.show_transient_status_message(kind.resolving_message());
    }

    /// Queue one hover lookup for the current cursor position.
    fn request_hover(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            self.show_error_message("No file is open for hover");
            return;
        }
        self.clear_hover_and_rename_state();
        let token = self.lookup_tokens.next();
        self.active_hover_lookup = Some(ActiveHoverLookup {
            token,
            document_version: self.lsp_document_version,
        });
        self.pending_request = Some(EditorRequest::LspHover);
        self.show_transient_status_message("Resolving hover...");
    }

    /// Queue one rename lookup for the current cursor position.
    fn request_rename(&mut self, new_name: String) {
        if self.file_path.as_os_str().is_empty() {
            self.show_error_message("No file is open for rename");
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
        self.show_transient_status_message("Renaming symbol...");
    }

    /// Queue one code-action lookup for the current cursor context.
    fn request_code_actions(&mut self) {
        if self.file_path.as_os_str().is_empty() {
            self.show_error_message("No file is open for code actions");
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
        self.show_transient_status_message("Loading code actions...");
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
            NavigationLookupOutcome::Single(target) => {
                // When the destination equals the current position the cursor does
                // not move, but the transient resolving message must still be
                // cleared so it does not linger on the terminal message row.
                if !self.goto_navigation_target(&target) {
                    self.clear_status_message();
                }
            }
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
            | NavigationLookupOutcome::Error(message) => self.show_error_message(message),
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
                self.clear_status_message();
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
                self.show_error_message(message);
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
                self.clear_status_message();
            }
            SignatureHelpLookupOutcome::NotFound => {
                self.signature_help_popup = None;
            }
            SignatureHelpLookupOutcome::UnsupportedFile(message)
            | SignatureHelpLookupOutcome::UnsupportedProject(message)
            | SignatureHelpLookupOutcome::Error(message) => {
                self.signature_help_popup = None;
                self.show_error_message(message);
            }
            SignatureHelpLookupOutcome::Unavailable(message) => {
                self.signature_help_popup = None;
                if !missing_server_binary {
                    self.show_error_message(message);
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
    ///
    /// Returns `true` when the destination file opens and the cursor is placed
    /// on the requested location, and `false` when the destination cannot be
    /// resolved or opened.
    fn goto_navigation_target(&mut self, target: &NavigationTarget) -> bool {
        self.goto_picker_target(
            &target.file_path,
            target.line,
            target.character,
            "navigation target",
        )
    }

    /// Open one search-result target and move the cursor to the matched position.
    fn goto_search_picker_target(&mut self, target: &SearchPickerTarget) {
        if self.goto_picker_target(
            &target.file_path,
            target.line,
            target.column,
            "search result",
        ) {
            self.center_cursor_after_picker_jump();
        }
    }

    /// Open one picker-owned file target and move the cursor to the requested position.
    ///
    /// Returns `true` when the picker target opens and updates the active cursor,
    /// and `false` when the jump cannot be completed.
    fn goto_picker_target(
        &mut self,
        file_path: &Path,
        line: usize,
        column: usize,
        target_kind: &str,
    ) -> bool {
        if !self.record_jump_origin_for_destination(file_path, line, column) {
            return false;
        }
        let open_path = current_dir_relative_path(file_path);
        // Open the destination file first so every later cursor calculation uses the target buffer.
        if let Err(error) = self.open_buffer(open_path.as_ref()) {
            self.show_error_message(format!(
                "Failed to open {target_kind} \"{}\": {error}",
                open_path.display()
            ));
            return false;
        }
        // Successful target resolution hands feedback over to the cursor jump itself,
        // so the transient lookup message must not remain on the message line.
        self.clear_status_message();
        // Clamp the destination because match and LSP locations may target EOF or short lines.
        self.cursor = self.clamped_normal_cursor(line, column);
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        true
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

    /// Clear transient picker state together with any hover overlay.
    fn clear_picker_and_hover_state(&mut self) {
        if let Some(picker) = &mut self.file_picker {
            picker.cancel();
        }
        if let Some(picker) = &mut self.search_picker {
            picker.cancel();
        }
        self.file_picker = None;
        self.search_picker = None;
        self.location_picker = None;
        self.diagnostic_picker = None;
        self.code_action_picker = None;
        self.picker_preview.clear();
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

    /// Clear the active command-completion popup without mutating the prompt text.
    fn dismiss_command_completion_session(&mut self) {
        self.command_completion_session = None;
        self.cancel_pending_command_completion();
    }

    /// Refresh command-mode completion from the current prompt contents.
    fn refresh_command_completion(&mut self, explicit: bool) {
        let request = match &self.mode {
            Mode::Command(input) => build_command_completion_request(
                input.text(),
                input.cursor(),
                explicit,
                ex_commands::command_specs(),
            ),
            _ => None,
        };

        // Rebuild from the current prompt snapshot so cursor moves and prompt edits
        // always recompute against the active token instead of stale preview state.
        let Some(request) = request else {
            self.dismiss_command_completion_session();
            return;
        };
        if self
            .pending_command_completion
            .as_ref()
            .is_some_and(|pending| {
                pending.request().matches_prompt_state(
                    self.mode.command_string().unwrap_or_default(),
                    self.mode.input_cursor().unwrap_or(0),
                    ex_commands::command_specs(),
                )
            })
        {
            return;
        }
        if !request.requires_async_scan() {
            self.cancel_pending_command_completion();
            self.command_completion_session = build_command_completion_session_for_request(
                &request,
                ex_commands::command_specs(),
            );
            return;
        }

        self.command_completion_session = self
            .command_completion_session
            .as_ref()
            .and_then(|session| retained_async_command_completion_session(session, &request));
        self.cancel_pending_command_completion();
        self.pending_command_completion = PendingCommandCompletion::spawn(request);
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

        let refreshed_session = refresh_session(
            &self.completion_sources,
            &self.buffer,
            request.clone(),
            popup_anchor_char_idx,
            &retained_async_candidates,
        );
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

    /// Cancel any in-flight asynchronous command-mode completion request.
    fn cancel_pending_command_completion(&mut self) {
        if let Some(pending) = &mut self.pending_command_completion {
            pending.cancel();
        }
        self.pending_command_completion = None;
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
    fn poll_command_completion_background_tasks(&mut self) {
        let Some(mut pending) = self.pending_command_completion.take() else {
            return;
        };
        let poll_result = pending.poll();
        if !poll_result.finished {
            self.pending_command_completion = Some(pending);
            return;
        }

        let (input, cursor_column) = match &self.mode {
            Mode::Command(input) => (input.text(), input.cursor()),
            _ => return,
        };
        if !pending.request().matches_prompt_state(
            input,
            cursor_column,
            ex_commands::command_specs(),
        ) {
            return;
        }

        self.command_completion_session = poll_result.candidates.and_then(|candidates| {
            build_command_completion_session_from_candidates(
                pending.request().context(),
                candidates,
            )
        });
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

    /// Cycle the command-mode completion selection when one prompt session is active.
    ///
    /// Returns `true` when a visible command-completion session handled the
    /// request and updated the prompt preview, and `false` when no completion
    /// session was available for the requested direction.
    fn move_command_completion_selection(&mut self, direction: CommandCompletionDirection) -> bool {
        if self.command_completion_session.is_none() {
            self.refresh_command_completion(true);
        }
        let Some(mut session) = self.command_completion_session.take() else {
            return false;
        };
        let start_column = session.replace_start_column();
        let end_column = session.replacement_end_column();
        session.move_selection(direction);
        let replacement = session.current_text();
        self.replace_command_completion_range(start_column, end_column, replacement);
        self.command_completion_session = Some(session);
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
            | Action::PromptGrep
            | Action::GrepWordUnderCursor
            | Action::EnterSearchMode
            | Action::OpenBufferSwitcher
            | Action::OpenFilePicker
            | Action::GotoDefinition
            | Action::GotoReferences
            | Action::GotoFileUnderCursor
            | Action::GotoFileUnderCursorAtPosition
            | Action::GotoAlternateFile
            | Action::GotoCorrespondingFile
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
            | Action::ToggleLineComment
            | Action::ToggleBlockComment
            | Action::DeleteToLineEnd
            | Action::ChangeToLineEnd
            | Action::IncrementNextNumber
            | Action::DecrementNextNumber
            | Action::JoinLines
            | Action::BeginReplaceChar
            | Action::BeginInsertLiteral
            | Action::SearchWordUnderCursor
            | Action::DeleteCharAtCursor
            | Action::DeleteSelection
            | Action::IndentSelection
            | Action::ReindentSelection
            | Action::DedentSelection
            | Action::ChangeSelection
            | Action::YankSelection
            | Action::YankCurrentLine
            | Action::YankToLineEnd
            | Action::YankClipboard
            | Action::PasteAfterCursor
            | Action::PasteBeforeCursor
            | Action::PasteClipboardAfterCursor
            | Action::PasteClipboardBeforeCursor
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
            | Action::MoveInputWordRight
            | Action::CommandCompletionNext
            | Action::CommandCompletionPrev => self.dismiss_completion_if_not_insert(),
        }
    }

    /// Sync command-completion visibility after one action updates the prompt state.
    fn sync_command_completion_after_action(&mut self, action: Action) {
        match action {
            Action::CommandCompletionNext | Action::CommandCompletionPrev => {}
            Action::PromptHistoryPrev
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
            | Action::MoveInputWordRight => self.refresh_command_completion(false),
            _ => {
                if !matches!(self.mode, Mode::Command(_)) {
                    self.dismiss_command_completion_session();
                }
            }
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
            | Action::PromptGrep
            | Action::GrepWordUnderCursor
            | Action::EnterSearchMode
            | Action::OpenBufferSwitcher
            | Action::OpenFilePicker
            | Action::GotoDefinition
            | Action::GotoReferences
            | Action::GotoFileUnderCursor
            | Action::GotoFileUnderCursorAtPosition
            | Action::GotoAlternateFile
            | Action::GotoCorrespondingFile
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
            | Action::ToggleLineComment
            | Action::ToggleBlockComment
            | Action::DeleteToLineEnd
            | Action::ChangeToLineEnd
            | Action::IncrementNextNumber
            | Action::DecrementNextNumber
            | Action::JoinLines
            | Action::BeginReplaceChar
            | Action::BeginInsertLiteral
            | Action::SearchWordUnderCursor
            | Action::DeleteCharAtCursor
            | Action::DeleteSelection
            | Action::IndentSelection
            | Action::ReindentSelection
            | Action::DedentSelection
            | Action::ChangeSelection
            | Action::YankSelection
            | Action::YankCurrentLine
            | Action::YankToLineEnd
            | Action::YankClipboard
            | Action::PasteAfterCursor
            | Action::PasteBeforeCursor
            | Action::PasteClipboardAfterCursor
            | Action::PasteClipboardBeforeCursor
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
            | Action::MoveInputWordRight
            | Action::CommandCompletionNext
            | Action::CommandCompletionPrev => {
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
        self.search_count.invalidate();
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
        let old_tail_exit_mode =
            self.syntax
                .exit_mode_for_range(&self.buffer, start_line, start_line);
        // Popup anchors are stored as absolute buffer indices, so they must move
        // with any text inserted before the saved anchor position.
        self.shift_completion_popup_anchors_for_insert(char_idx, inserted_char_count);
        if let Some(selection) = self.last_visual_selection.as_mut() {
            selection.shift_for_insert(char_idx, inserted_char_count);
        }
        self.buffer.insert(char_idx, text);
        let new_end_line = start_line + text.chars().filter(|&c| c == '\n' || c == '\r').count();
        let may_change_later_line_state = old_tail_exit_mode
            != self
                .syntax
                .exit_mode_for_range(&self.buffer, start_line, new_end_line);
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
            new_end_line,
            may_change_later_line_state,
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
        let old_tail_exit_mode =
            self.syntax
                .exit_mode_for_range(&self.buffer, start_line, old_end_line);
        // Removing text before the popup anchor would otherwise leave it pointing
        // at a later buffer position, potentially even beyond the current line.
        self.shift_completion_popup_anchors_for_removal(start_char, end_char);
        if let Some(selection) = self.last_visual_selection.as_mut() {
            selection.shift_for_removal(start_char, end_char);
        }
        self.buffer.remove(start_char, end_char);
        let may_change_later_line_state = old_tail_exit_mode
            != self
                .syntax
                .exit_mode_for_range(&self.buffer, start_line, start_line);
        // Deletions send the pre-edit span with an empty replacement string.
        self.queue_lsp_change(LspTextChange {
            range: Some(LspRange { start, end }),
            text: String::new(),
        });
        self.syntax.apply_edit(BufferEdit {
            start_line,
            old_end_line,
            new_end_line: start_line,
            may_change_later_line_state,
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
    ///
    /// Unnamed buffers additionally offer an `[i] ignore` choice that leaves the
    /// swap file on disk and starts a fresh empty buffer.
    fn build_recovery_swap_prompt(
        &self,
        recovery: swap::SwapRecovery,
        recreate_handle_on_discard: bool,
    ) -> PendingSwapPrompt {
        let is_unnamed = self.file_path.as_os_str().is_empty();
        let prompt = if is_unnamed {
            "Recovery swap found. [r] recover [d] discard [i] ignore [c] cancel".to_string()
        } else {
            "Recovery swap found. [r] recover [d] discard [c] cancel".to_string()
        };
        PendingSwapPrompt {
            prompt,
            recovered_buffer: recovery.buffer,
            swap_path: recovery.swap_path,
            kind: PendingSwapPromptKind::Recovery,
            cancel_action: self.pending_swap_cancel_action(),
            recreate_handle_on_discard,
            supports_ignore: is_unnamed,
        }
    }

    /// Build one prompt for a swap file that likely belongs to another instance.
    ///
    /// Unnamed buffers additionally offer an `[i] ignore` choice that leaves the
    /// swap file on disk and starts a fresh empty buffer.
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
        let is_unnamed = self.file_path.as_os_str().is_empty();
        let choice_tail = if is_unnamed {
            " [o] read-only [e] edit [r] recover [d] discard [i] ignore [c] cancel"
        } else {
            " [o] read-only [e] edit [r] recover [d] discard [c] cancel"
        };

        PendingSwapPrompt {
            prompt: format!("{explanation}{choice_tail}"),
            recovered_buffer: conflict.buffer,
            swap_path: conflict.swap_path,
            kind: PendingSwapPromptKind::Conflict,
            cancel_action: self.pending_swap_cancel_action(),
            recreate_handle_on_discard: self.file_path.as_os_str().is_empty()
                || !self.active_path_is_swap_excluded(),
            supports_ignore: is_unnamed,
        }
    }

    /// Resolve one working directory path used by unnamed-buffer swap handling.
    fn resolve_working_directory_for_unnamed_swap(&mut self) -> io::Result<Option<PathBuf>> {
        match std::env::current_dir() {
            Ok(cwd) => {
                self.last_known_working_directory = Some(cwd.clone());
                self.missing_working_directory_swap_warning_emitted = false;
                Ok(Some(cwd))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Keep a single warning visible while cwd is missing so normal
                // edit loops do not spam the message line on each swap probe.
                if !self.missing_working_directory_swap_warning_emitted {
                    self.show_warning_message(
                        "Unnamed swap protection is degraded because the working directory no longer exists",
                    );
                    self.missing_working_directory_swap_warning_emitted = true;
                }
                Ok(self.last_known_working_directory.clone())
            }
            Err(error) => Err(io::Error::other(format!(
                "failed to read working directory: {error}"
            ))),
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
        self.swap_loaded = true;
        let active_path = normalize_lookup_path(&self.file_path);
        let is_excluded = active_path
            .as_ref()
            .is_some_and(|path| self.path_is_swap_excluded(path));
        let existing_swap = if let Some(path) = active_path.as_ref() {
            swap::inspect_existing_swap(path)
        } else if self.file_path.as_os_str().is_empty() {
            match self.resolve_working_directory_for_unnamed_swap() {
                Ok(Some(cwd)) => swap::inspect_unnamed_swap_for_cwd(&cwd),
                Ok(None) => {
                    self.suppress_swap_creation = true;
                    return;
                }
                Err(error) => Err(error),
            }
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
                self.show_error_message(format!("Swap recovery unavailable: {error}"));
            }
        }
    }

    /// Create a fresh swap file handle for the active buffer path.
    fn create_active_swap_handle(&mut self) -> io::Result<()> {
        let handle = if let Some(path) = normalize_lookup_path(&self.file_path) {
            SwapHandle::create_from_buffer(&path, &self.buffer)?
        } else if self.file_path.as_os_str().is_empty() {
            let Some(cwd) = self.resolve_working_directory_for_unnamed_swap()? else {
                self.suppress_swap_creation = true;
                return Ok(());
            };
            SwapHandle::create_for_unnamed_buffer_in_cwd(&self.buffer, &cwd)?
        } else {
            return Ok(());
        };
        self.suppress_swap_creation = false;
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
            debug_assert!(self.swap.is_some() || self.suppress_swap_creation);
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
        self.show_error_message(format!(
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
                editor.show_error_message("Nothing to paste");
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
                editor.show_error_message("Nothing to paste");
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
        self.touch_pending_auto_insert();
        let char_idx = self.adjusted_insert_char_idx(c);
        self.insert_buffer_text(char_idx, &c.to_string());
        self.cursor = Cursor::from_char_index(&self.buffer, char_idx + 1);
        self.auto_dedent_current_line_after_insert();
    }

    /// Apply one terminal bracketed-paste payload according to the active mode.
    pub(crate) fn handle_paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.pending_insert_literal = false;
        if let Some(picker) = self.active_picker_kind() {
            self.paste_into_picker(picker, text);
            return;
        }
        match self.mode {
            Mode::Insert => self.paste_into_insert_mode(text),
            Mode::Command(_) | Mode::Search(_) => self.paste_into_prompt(text),
            Mode::Visual(_) => self.paste_into_visual_mode(text),
            _ if self.mode_uses_modal_bindings() => self.paste_into_normal_mode(text),
            _ => {}
        }
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

    /// Insert one pasted payload at the Insert-mode cursor without replaying key-by-key edits.
    fn paste_into_insert_mode(&mut self, text: &str) {
        self.touch_pending_auto_insert();
        let char_idx = self.cursor.to_char_index(&self.buffer);
        let insertion = self.materialized_bracketed_paste_text(char_idx, text);

        // Keep the cursor at the end of the user-provided payload so a
        // synthesized EOF newline only materializes the blank line behind it.
        self.insert_buffer_text(char_idx, &insertion);
        self.cursor = Cursor::from_char_index(&self.buffer, char_idx + text.chars().count());
        self.viewport
            .ensure_cursor_visible(&self.cursor, &self.buffer);
        self.sync_visible_match_for_viewport();
        self.dismiss_completion_session(false);
        self.dismiss_signature_help();
        self.clear_pending_auto_insert_if_cursor_left_line();
    }

    /// Insert only the first pasted line into the active single-line prompt.
    fn paste_into_prompt(&mut self, text: &str) {
        let line = Self::first_pasted_line(text);
        if line.is_empty() {
            return;
        }
        let cursor = self.mode.input_cursor().unwrap_or_default();
        self.edit_prompt_input(|mode| mode.replace_input_range(cursor, cursor, line));
    }

    /// Insert one flattened pasted payload into the active picker filter.
    fn paste_into_picker(&mut self, picker: PickerKind, text: &str) {
        let line = Self::flattened_picker_paste_text(text);
        if line.is_empty() {
            return;
        }
        let cursor = self.mode.input_cursor().unwrap_or_default();
        self.mode.replace_input_range(cursor, cursor, &line);
        self.refresh_picker_matches(picker);
    }

    /// Replace the active Visual selection, then insert the pasted payload.
    fn paste_into_visual_mode(&mut self, text: &str) {
        let Some(saved_selection) = self.current_visual_selection() else {
            return;
        };
        let Some(selection) = self.visual_selection() else {
            return;
        };
        self.last_visual_selection = Some(saved_selection);
        // Visual bracketed paste behaves like a replace: delete the selection
        // first, route the delete through the shared Visual delete path without
        // touching the unnamed register, then insert the payload as one change.
        self.apply_delete_visual_selection(selection, true, false);
        self.paste_into_insert_mode(text);
        self.exit_to_normal_mode();
        self.last_visual_selection = Some(saved_selection);
    }

    /// Paste one payload as characterwise text after the cursor in Normal mode.
    fn paste_into_normal_mode(&mut self, text: &str) {
        self.with_history_transaction(|editor| {
            let char_idx = editor.cursor.to_char_index(&editor.buffer);
            let insert_idx = if editor.buffer.line_len(editor.cursor.line()) > 0 {
                char_idx + 1
            } else {
                char_idx
            };
            let insertion = editor.materialized_bracketed_paste_text(insert_idx, text);

            // Normal-mode bracketed paste inserts terminal data literally rather
            // than routing through Vim register semantics, so preserve the typed
            // payload and only synthesize the extra EOF newline when needed.
            editor.insert_buffer_text(insert_idx, &insertion);
            let cursor_char_idx = if Self::text_ends_with_line_break(text) {
                insert_idx + text.chars().count()
            } else {
                // Non-newline payloads should land on the final inserted
                // character because Normal mode never places the cursor after
                // the end of the pasted text.
                insert_idx + text.chars().count().saturating_sub(1)
            };
            editor.cursor = Cursor::from_char_index(&editor.buffer, cursor_char_idx);
            editor
                .viewport
                .ensure_cursor_visible(&editor.cursor, &editor.buffer);
            editor.sync_visible_match_for_viewport();
        });
    }

    /// Return the first logical line from one normalized bracketed-paste payload.
    fn first_pasted_line(text: &str) -> &str {
        text.split('\n').next().unwrap_or("")
    }

    /// Return one single-line picker query from a normalized bracketed-paste payload.
    fn flattened_picker_paste_text(text: &str) -> String {
        text.split('\n')
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Return the text stored for one Insert-mode bracketed paste.
    fn materialized_bracketed_paste_text<'a>(
        &self,
        insert_idx: usize,
        text: &'a str,
    ) -> Cow<'a, str> {
        if insert_idx != self.buffer.chars_count() || !Self::text_ends_with_line_break(text) {
            return Cow::Borrowed(text);
        }
        let mut materialized = String::with_capacity(text.len() + 1);
        materialized.push_str(text);
        materialized.push('\n');
        Cow::Owned(materialized)
    }

    /// Delete one character backward in insert mode.
    fn delete_char_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx > 0 {
            self.touch_pending_auto_insert();
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
            self.touch_pending_auto_insert();
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
    ///
    /// Stops at the current line boundary: if the cursor is at column 0 the
    /// preceding newline is removed (joining with the previous line); otherwise
    /// the deletion never crosses a newline. Whitespace-only content before
    /// the cursor on the current line is deleted in its entirety.
    fn delete_word_backward(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }

        let char_idx = self.cursor.to_char_index(&self.buffer);
        if char_idx == 0 {
            return;
        }

        self.touch_pending_auto_insert();
        let word_start = find_prev_word_start_insert_mode(&self.buffer, char_idx);
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

        self.touch_pending_auto_insert();
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

    /// Delete one picker word backward using whitespace-only boundaries.
    ///
    /// Forwards to `Mode::delete_input_word_backward_picker` so that Ctrl-w in
    /// picker dialogs deletes across punctuation characters such as `-`, `/`, and
    /// `.` in a single keystroke.
    fn delete_input_word_backward_picker(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_word_backward_picker());
    }

    /// Delete one picker word backward using emacs-style boundaries.
    ///
    /// Forwards to `Mode::delete_input_word_backward_picker_alt` so that
    /// Alt-Backspace in picker dialogs skips trailing punctuation then deletes
    /// the preceding alphanumeric word, e.g. `foo-bar-` becomes `foo-`.
    fn delete_input_word_backward_picker_alt(&mut self) {
        self.edit_prompt_input(|mode| mode.delete_input_word_backward_picker_alt());
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

    /// Replace the current command-completion span while keeping prompt previews in sync.
    fn replace_command_completion_range(&mut self, start: usize, end: usize, text: &str) {
        let prompt_changed = self.active_prompt_text().is_some_and(|before| {
            let before_char_count = before.chars().count();
            let start = start.min(before_char_count);
            let end = end.min(before_char_count).max(start);
            before
                .chars()
                .skip(start)
                .take(end - start)
                .ne(text.chars())
        });
        self.mode.replace_input_range(start, end, text);
        if prompt_changed {
            self.reset_active_prompt_history();
        }
        self.sync_prompt_auxiliary_previews();
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
        // Save current viewport and cursor before entering search mode
        self.search_highlighting
            .save_original_viewport(self.viewport);
        self.search_highlighting
            .save_original_cursor(self.cursor.clone());
        self.mode = Mode::search_empty();
        self.prompt_history
            .reset_traversal(PromptHistoryKind::Search);
        self.sync_prompt_previews();
    }

    /// Refresh prompt side effects that do not own command-completion state.
    fn sync_prompt_auxiliary_previews(&mut self) {
        self.refresh_substitute_preview();
        self.sync_search_highlights_for_viewport();
    }

    /// Refresh every prompt-scoped preview surface after one prompt edit.
    fn sync_prompt_previews(&mut self) {
        self.sync_prompt_auxiliary_previews();
        self.refresh_command_completion(false);
    }

    /// Leave command or search mode while clearing transient prompt-only UI state.
    fn cancel_prompt_input(&mut self) {
        self.pending_search_count = None;
        self.reset_active_prompt_history();
        self.mode = Mode::Normal;
        self.dismiss_command_completion_session();
        self.clear_substitute_preview(true);

        // Restore original viewport and cursor if search preview had scrolled
        if let Some(original_viewport) = self.search_highlighting.take_original_viewport() {
            self.viewport = original_viewport;
        }
        if let Some(original_cursor) = self.search_highlighting.take_original_cursor() {
            self.cursor = original_cursor;
        }

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
        self.apply_delete_visual_selection(selection, enter_insert, true);
    }

    /// Delete one explicit Visual selection and optionally enter Insert mode afterward.
    fn apply_delete_visual_selection(
        &mut self,
        selection: VisualSelection,
        enter_insert: bool,
        yank_into_register: bool,
    ) {
        match selection {
            VisualSelection::Character(selection) => {
                self.apply_delete_selection(
                    selection,
                    VisualKind::Character,
                    enter_insert,
                    yank_into_register,
                );
            }
            VisualSelection::Line(selection) => {
                self.apply_delete_selection(
                    selection,
                    VisualKind::Line,
                    enter_insert,
                    yank_into_register,
                );
            }
            VisualSelection::Block(selection) => {
                self.apply_delete_block_selection(selection, enter_insert, yank_into_register);
            }
        }
    }

    /// Delete one explicit selection and optionally enter Insert mode afterward.
    fn apply_delete_selection(
        &mut self,
        selection: SelectionRange,
        kind: VisualKind,
        enter_insert: bool,
        yank_into_register: bool,
    ) {
        // Capture the target line index before deletion so it remains valid
        // regardless of how the buffer shrinks afterward.
        let line_idx = self.buffer.char_to_line(selection.start);

        self.begin_history_transaction();
        if yank_into_register {
            self.delete_range_into_yank_buffer(selection, Self::yank_kind_for_visual(kind));
        } else if selection.end > selection.start {
            self.remove_buffer_range(selection.start, selection.end);
        }

        // A linewise change keeps one empty line in place so the user has a
        // line to type on, matching vim's behaviour.  The blank line slot is
        // always inserted so the indentation prefix has a line to land on and
        // following content stays on separate lines.
        if enter_insert && kind == VisualKind::Line {
            self.insert_buffer_text(selection.start, "\n");
        }

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

        // Re-indent the blank replacement line for linewise changes using the
        // same auto-indent algorithm as `o`/`O`/Enter and `cc`.
        if enter_insert && kind == VisualKind::Line {
            self.apply_indent_prefix_to_line(selection.start, line_idx);
        }

        if enter_insert {
            self.clear_visual_mode(Mode::Insert);
        } else {
            self.clear_visual_mode(Mode::Normal);
            self.finish_history_transaction();
        }
    }

    /// Delete one explicit block selection and optionally enter Insert mode afterward.
    fn apply_delete_block_selection(
        &mut self,
        selection: BlockSelection,
        enter_insert: bool,
        yank_into_register: bool,
    ) {
        self.begin_history_transaction();
        if yank_into_register {
            self.delete_block_into_yank_buffer(selection);
        } else {
            let segments = selection.segments(&self.buffer);
            // Delete bottom-up so earlier block rows keep their captured indices.
            for segment in segments.iter().rev() {
                self.remove_buffer_range(segment.start, segment.end);
            }
        }

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
    use std::thread;
    use test_utils::{
        CurrentDirectoryGuard, EnvVarGuard, TempFile, TempTree, lock_process_environment,
    };

    fn create_editor_with_content(content: &str) -> EditorState {
        let mut editor = EditorState::new(24);
        editor.buffer = TextBuffer::from_str(content);
        editor
    }

    #[test]
    /// Verify picker previews compact home paths outside the current directory.
    fn test_picker_preview_display_path_compacts_home_relative_path() {
        let lock = lock_process_environment();
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home");
        let project = tree.path().join("project");
        std::fs::create_dir_all(home.join("workspace")).expect("create home workspace");
        std::fs::create_dir_all(&project).expect("create project");
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());
        let _cwd_guard = CurrentDirectoryGuard::change_to(&project);

        assert_eq!(
            EditorState::picker_preview_display_path(&home.join("workspace/main.rs")),
            "~/workspace/main.rs"
        );
    }

    #[test]
    /// Verify overwrite prompts compact home-directory target paths.
    fn test_overwrite_prompt_compacts_home_relative_target_path() {
        let lock = lock_process_environment();
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home");
        std::fs::create_dir_all(home.join("workspace")).expect("create home workspace");
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());
        let mut editor = EditorState::new(24);

        editor.pending_overwrite = Some(PendingOverwrite {
            target_path: home.join("workspace/main.rs"),
            update_file_path: false,
            after_write_action: AfterWriteAction::StayOpen,
            reason: OverwritePromptKind::DifferentTargetPath,
        });

        assert_eq!(
            editor.overwrite_prompt(),
            Some("Overwrite \"~/workspace/main.rs\"? [y/N]".to_string())
        );
    }

    #[test]
    /// Verify successful write status messages compact home-directory target paths.
    fn test_write_status_message_compacts_home_relative_target_path() {
        let lock = lock_process_environment();
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home");
        std::fs::create_dir_all(&home).expect("create home");
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());
        let target = home.join("written.txt");
        let mut editor = create_editor_with_content("test content");
        editor.mode = Mode::command_with_text(format!("w {}", target.display()));

        handle_key_and_flush_requests(&mut editor, Key::Char('\n'));

        assert_eq!(
            editor.status_message,
            Some("\"~/written.txt\" written".to_string())
        );
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
        let mut clipboard = crate::clipboard::ClipboardState::new();
        for _ in 0..128 {
            editor.poll_background_tasks();
            let Some(request) = editor.take_pending_request() else {
                if editor.pending_save_conflict_check.is_none() {
                    return;
                }
                // Save-conflict checks can complete on another thread, so yielding
                // lets that worker run before this loop polls again.
                thread::yield_now();
                continue;
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
                EditorRequest::WriteClipboard(write) => {
                    app::execute_deferred_clipboard_write(editor, &mut clipboard, &write)
                }
                EditorRequest::PasteClipboard(paste) => {
                    app::execute_deferred_clipboard_paste(editor, &mut clipboard, &paste)
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
            // Deferred writes can enqueue follow-up requests, so yield once to
            // avoid starving producers before the next drain iteration.
            thread::yield_now();
        }
        panic!("flush_pending_requests exceeded 128 chained requests");
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

    #[test]
    /// Clean active buffers should reload immediately after an external disk change.
    fn test_active_clean_buffer_auto_reloads_after_external_change() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");

        // Updating the on-disk file while the buffer stays clean should refresh
        // the active buffer contents instead of prompting.
        fs::write(file.path(), "beta\n").expect("rewrite file");
        editor.apply_external_path_change(file.path());

        assert_eq!(editor.buffer.to_string(), "beta\n");
        assert_eq!(editor.external_change_prompt(), None);
        assert_eq!(
            editor.status_message,
            Some(format!(
                "\"{}\" reloaded after external change",
                file.path().display()
            ))
        );
    }

    #[test]
    /// Undo after an external reload should restore the pre-reload buffer contents.
    fn test_undo_restores_buffer_contents_before_external_reload() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");

        // Reload inserts one history entry whose undo path must restore the
        // exact text that was visible before the external change landed.
        fs::write(file.path(), "beta\n").expect("rewrite file");
        editor.apply_external_path_change(file.path());
        assert_eq!(editor.buffer.to_string(), "beta\n");
        editor.undo_changes(1);

        assert_eq!(editor.buffer.to_string(), "alpha\n");
        assert!(editor.buffer.is_modified());
    }

    #[test]
    /// Hidden clean buffers should reload in the background and report that on activation.
    fn test_hidden_clean_buffer_auto_reload_defers_notice_until_activation() {
        let first = TempFile::with_suffix(".txt").expect("first temp file");
        first.write_all(b"first\n").expect("write first file");
        let second = TempFile::with_suffix(".txt").expect("second temp file");
        second.write_all(b"second\n").expect("write second file");
        let mut editor = EditorState::new(24);
        editor.load_file(first.path()).expect("load first file");
        editor
            .open_buffer(second.path())
            .expect("open second buffer");
        let first_id = editor
            .buffer_summaries()
            .into_iter()
            .find(|summary| {
                !summary.active && summary.display_path == first.path().display().to_string()
            })
            .expect("first buffer summary")
            .id;

        // The hidden buffer should refresh immediately, but the user-facing
        // notice must wait until the buffer becomes active again.
        fs::write(first.path(), "first updated\n").expect("rewrite first file");
        editor.apply_external_path_change(first.path());

        assert_eq!(editor.file_path, second.path());
        assert_eq!(editor.buffer.to_string(), "second\n");
        assert_eq!(editor.status_message, None);

        editor.activate_buffer(first_id);

        assert_eq!(editor.file_path, first.path());
        assert_eq!(editor.buffer.to_string(), "first updated\n");
        assert_eq!(
            editor.status_message,
            Some(format!(
                "\"{}\" reloaded after external change",
                first.path().display()
            ))
        );
    }

    #[test]
    /// Ignored external-change prompts should stay suppressed until the file changes again.
    fn test_ignored_external_change_stays_suppressed_until_new_disk_version() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('!'));
        editor.handle_key(Key::Esc);

        // The first external edit should prompt, and ignoring it should keep the
        // same on-disk fingerprint from prompting again.
        fs::write(file.path(), "beta\n").expect("rewrite file");
        editor.apply_external_path_change(file.path());
        assert!(editor.external_change_prompt().is_some());

        assert!(editor.handle_pending_external_change_key(Key::Char('i')));
        assert_eq!(editor.external_change_prompt(), None);

        editor.apply_external_path_change(file.path());
        assert_eq!(editor.external_change_prompt(), None);

        fs::write(file.path(), "gamma\n").expect("rewrite file again");
        editor.apply_external_path_change(file.path());

        assert!(editor.external_change_prompt().is_some());
    }

    #[test]
    /// Save requests should still ask before overwriting disk changes that were previously ignored.
    fn test_save_after_ignored_external_change_prompts_for_overwrite() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('!'));
        editor.handle_key(Key::Esc);
        fs::write(file.path(), "beta\n").expect("rewrite file");
        editor.apply_external_path_change(file.path());
        assert!(editor.handle_pending_external_change_key(Key::Char('i')));

        // Ignoring the reload prompt suppresses that prompt only; saving the file
        // must still require an explicit overwrite confirmation.
        editor.request_save_current(
            OverwriteBehavior::ConfirmIfDifferentPath,
            PostSaveAction::StayOpen,
        );
        flush_pending_requests(&mut editor);

        assert_eq!(
            editor.overwrite_prompt(),
            Some(format!(
                "\"{}\" changed on disk. Overwrite anyway? [y/N]",
                file.path().display()
            ))
        );
        assert_eq!(editor.pending_request, None);
    }

    #[test]
    /// Save requests should defer completion while async save-conflict checks are pending.
    fn test_save_conflict_check_defers_write_until_fingerprint_arrives() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('!'));
        editor.handle_key(Key::Esc);
        fs::write(file.path(), "beta\n").expect("rewrite file");

        editor.request_save_current(
            OverwriteBehavior::ConfirmIfDifferentPath,
            PostSaveAction::StayOpen,
        );

        assert!(editor.pending_save_conflict_check.is_some());
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Checking external changes before write...")
        );

        flush_pending_requests(&mut editor);
        assert!(editor.pending_save_conflict_check.is_none());
        assert_eq!(
            editor.overwrite_prompt(),
            Some(format!(
                "\"{}\" changed on disk. Overwrite anyway? [y/N]",
                file.path().display()
            ))
        );
    }

    #[test]
    /// Repeated fingerprint worker disconnects should trigger synchronous fallback with warning.
    fn test_fingerprint_worker_disconnect_falls_back_to_sync_with_warning() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");
        editor.settings.auto_reload_external_changes = false;
        fs::write(file.path(), "beta\n").expect("rewrite file");

        editor.queue_external_fingerprint_request(file.path());
        editor
            .file_fingerprint_worker
            .simulate_disconnect_for_test();
        editor
            .file_fingerprint_worker
            .simulate_disconnect_for_test();
        editor.poll_file_fingerprint_results();

        assert_eq!(
            editor.status_message.as_deref(),
            Some("Fingerprint worker unavailable; using synchronous fingerprint checks")
        );
        assert!(editor.external_change_prompt().is_some());
    }

    #[test]
    /// Disabling clean-buffer auto-reload should surface a reload-or-ignore prompt instead.
    fn test_clean_buffer_external_change_prompts_when_auto_reload_disabled() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut editor = EditorState::new(24);
        editor.load_file(file.path()).expect("load file");
        editor.settings.auto_reload_external_changes = false;

        // Clean buffers should stop auto-reloading once the config disables that behavior.
        fs::write(file.path(), "beta\n").expect("rewrite file");
        editor.apply_external_path_change(file.path());

        assert_eq!(editor.buffer.to_string(), "alpha\n");
        assert_eq!(editor.status_message, None);
        assert_eq!(
            editor.external_change_prompt(),
            Some(format!(
                "\"{}\" changed on disk. Reload from disk? [r]eload/[i]gnore",
                editor.file_name()
            ))
        );
    }

    #[test]
    /// External-change prompt visibility should only require a message-line render update.
    fn test_external_change_prompt_is_message_only_render_change() {
        let file = TempFile::with_suffix(".txt").expect("temp file");
        file.write_all(b"alpha\n").expect("write initial file");
        let mut before = EditorState::new(24);
        before.load_file(file.path()).expect("load file");
        let mut after = EditorState::new(24);
        after.load_file(file.path()).expect("load file");
        after.settings.auto_reload_external_changes = false;

        // The prompt only changes message-line content, so render diffing should
        // stay on the incremental message-only path.
        fs::write(file.path(), "beta\n").expect("rewrite file");
        after.apply_external_path_change(file.path());

        let decision = crate::render::RenderSnapshot::decide(
            &crate::render::RenderSnapshot::capture(&before),
            &crate::render::RenderSnapshot::capture(&after),
        );
        assert_eq!(decision, crate::render::RenderDecision::MessageOnly);
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

    #[test]
    fn test_buffer_switcher_popup_includes_preview_for_selected_buffer() {
        let first = TempFile::with_suffix("_first.txt").expect("create first temp file");
        first
            .write_all(b"first buffer\n")
            .expect("seed first temp file");
        let second = TempFile::with_suffix("_second.txt").expect("create second temp file");
        second
            .write_all(b"second buffer\n")
            .expect("seed second temp file");
        let mut editor = EditorState::new(24);

        editor.load_file(first.path()).expect("load first file");
        let first_id = editor.active_buffer_id();
        editor
            .open_buffer(second.path())
            .expect("open second buffer");
        editor.buffer = TextBuffer::from_str("unsaved second buffer\n");
        editor.buffer.set_modified(true);
        editor
            .syntax
            .open_document(Some(second.path()), &editor.buffer);
        editor.activate_buffer(first_id);
        editor.open_buffer_switcher();

        let popup = editor.picker_popup().expect("buffer switch popup");
        let preview = popup.preview.expect("preview pane");
        assert!(
            preview
                .lines
                .iter()
                .any(|line| line.text.contains("unsaved second buffer"))
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
                alternate_buffer: None,
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

    /// Restoring a session with an alternate buffer should make `ga` jump to it.
    #[test]
    fn test_restore_project_session_populates_alternate_buffer() {
        let session_dir =
            std::env::temp_dir().join(format!("ordex_restore_session_alt_{}", std::process::id()));
        let main_path = session_dir.join("main.rs");
        let lib_path = session_dir.join("lib.rs");
        let _ = fs::remove_dir_all(&session_dir);
        fs::create_dir_all(&session_dir).expect("create session dir");
        fs::write(&main_path, "fn main() {}\n").expect("write main file");
        fs::write(&lib_path, "fn lib() {}\n").expect("write lib file");

        let mut editor = create_editor_with_content("kept");
        editor.file_path = PathBuf::from("kept.txt");
        editor
            .restore_project_session(&crate::session::ProjectSession {
                working_directory: session_dir.clone(),
                active_buffer: 0,
                alternate_buffer: Some(1),
                buffers: vec![
                    crate::session::SessionBuffer {
                        path: main_path.clone(),
                        cursor: Cursor::new(0, 0),
                    },
                    crate::session::SessionBuffer {
                        path: lib_path.clone(),
                        cursor: Cursor::new(0, 0),
                    },
                ],
            })
            .expect("restore project session");

        assert_eq!(editor.file_name(), "main.rs");
        editor.goto_alternate_file();
        assert_eq!(editor.file_name(), "lib.rs");

        let _ = fs::remove_dir_all(session_dir);
    }

    /// Restoring a session without an alternate buffer should show a status message.
    #[test]
    fn test_restore_project_session_without_alternate_buffer_shows_message() {
        let session_dir = std::env::temp_dir().join(format!(
            "ordex_restore_session_no_alt_{}",
            std::process::id()
        ));
        let main_path = session_dir.join("main.rs");
        let lib_path = session_dir.join("lib.rs");
        let _ = fs::remove_dir_all(&session_dir);
        fs::create_dir_all(&session_dir).expect("create session dir");
        fs::write(&main_path, "fn main() {}\n").expect("write main file");
        fs::write(&lib_path, "fn lib() {}\n").expect("write lib file");

        let mut editor = create_editor_with_content("kept");
        editor.file_path = PathBuf::from("kept.txt");
        editor
            .restore_project_session(&crate::session::ProjectSession {
                working_directory: session_dir.clone(),
                active_buffer: 0,
                alternate_buffer: None,
                buffers: vec![
                    crate::session::SessionBuffer {
                        path: main_path.clone(),
                        cursor: Cursor::new(0, 0),
                    },
                    crate::session::SessionBuffer {
                        path: lib_path.clone(),
                        cursor: Cursor::new(0, 0),
                    },
                ],
            })
            .expect("restore project session");

        assert_eq!(editor.file_name(), "main.rs");
        editor.goto_alternate_file();
        assert_eq!(editor.status_message, Some("No alternate file".to_string()));
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);

        let _ = fs::remove_dir_all(session_dir);
    }

    #[test]
    /// Last-modification jumps without committed edits should report informational feedback.
    fn test_goto_last_modification_without_change_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");

        editor.goto_last_modification();

        assert_eq!(
            editor.status_message.as_deref(),
            Some("No committed change")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// File-target motions without a path token should report informational feedback.
    fn test_goto_file_under_cursor_without_target_sets_info_message_kind() {
        let mut editor = create_editor_with_content("   ");

        editor.goto_file_under_cursor();

        assert_eq!(
            editor.status_message.as_deref(),
            Some("No file target under cursor")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    /// Building a session should capture the alternate buffer from recent history.
    #[test]
    fn test_build_project_session_captures_alternate_buffer() {
        let first = TempFile::with_suffix("_build_first.txt").expect("create first temp file");
        let second = TempFile::with_suffix("_build_second.txt").expect("create second temp file");
        let mut editor = EditorState::new(24);

        editor.set_startup_path(first.path());
        let first_id = editor.active_buffer_id();
        editor
            .open_buffer(second.path())
            .expect("open second buffer");
        editor.activate_buffer(first_id);

        let session = editor.build_project_session(PathBuf::from("/tmp/project"));
        let active_index = session.active_buffer;
        // The alternate buffer should be the most recently visited non-active buffer.
        let alternate_index = session
            .alternate_buffer
            .expect("alternate buffer should be set");
        assert_ne!(alternate_index, active_index);
        assert_eq!(alternate_index, 1);
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
    /// `O` on an empty buffer must create a real blank line above the original one.
    fn test_open_line_above_in_empty_buffer_creates_real_top_line() {
        let mut editor = create_editor_with_content("");

        editor.handle_key(Key::Char('O'));

        assert_eq!(editor.buffer.to_string(), "\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(0, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// `O` on an empty buffer must preserve the opened line after leaving Insert mode.
    fn test_open_line_above_in_empty_buffer_preserves_blank_line_after_escape_and_down() {
        let mut editor = create_editor_with_content("");

        editor.handle_key(Key::Char('O'));
        editor.handle_key(Key::Esc);

        // Moving onto the next line proves the opened blank line became a real logical line.
        editor.handle_key(Key::Char('j'));

        assert_eq!(editor.buffer.to_string(), "\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(editor.mode.is_normal());
    }

    #[test]
    /// Enter at EOF should create one real blank line that survives Normal-mode motion.
    fn test_insert_newline_at_eof_preserves_blank_line_after_escape_and_up() {
        let mut editor = create_editor_with_content("line1");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 5);
        editor.begin_history_transaction();

        // The inserted blank EOF line must stay addressable after Insert mode ends.
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.buffer.to_string(), "line1\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(1, 0));

        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('k'));

        assert_eq!(editor.buffer.to_string(), "line1\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(0, 0));
    }

    #[test]
    /// Opening a line below EOF should keep the new blank line visible after navigation.
    fn test_open_line_below_at_eof_preserves_blank_line_after_escape_and_up() {
        let mut editor = create_editor_with_content("line1");

        // `o` at EOF should materialize an editable blank line instead of a hidden sentinel.
        editor.handle_key(Key::Char('o'));
        assert_eq!(editor.buffer.to_string(), "line1\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));

        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('k'));

        assert_eq!(editor.buffer.to_string(), "line1\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(0, 0));
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
        // `o` opens a line after `fn main() {`, which is indented to 4 spaces.
        // Typing `x` (a word character with no trailing `;`) leaves `    x` as
        // an unterminated line — matching Neovim's cin_isterminated rule which
        // only terminates on `;`, `}`, or `{`.  The dot-repeat therefore opens
        // the next line at continuation-indent level: 8 spaces.
        let mut editor = create_syntax_editor("fn main() {\n}\n", "main.rs");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Char('x'));
        editor.handle_key(Key::Esc);
        editor.handle_key(Key::Char('.'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    x\n        x\n}\n"
        );
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_continues_line_comment() {
        let mut editor = create_syntax_editor("// alpha", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "// alpha\n// ");
        assert_eq!(editor.cursor, Cursor::new(1, 3));
    }

    #[test]
    fn test_insert_newline_continues_inline_line_comment() {
        let mut editor = create_syntax_editor("let x = 1; // alpha", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 19);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "let x = 1; // alpha\n           // "
        );
        assert_eq!(editor.cursor, Cursor::new(1, 14));
    }

    #[test]
    fn test_insert_newline_continues_block_comment_leader() {
        let mut editor = create_syntax_editor("/*\n * alpha\n */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(1, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/*\n * alpha\n * \n */");
        assert_eq!(editor.cursor, Cursor::new(2, 3));
    }

    #[test]
    fn test_open_line_below_continues_line_comment() {
        let mut editor = create_syntax_editor("// alpha\n", "main.rs");
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "// alpha\n// \n");
        assert_eq!(editor.cursor, Cursor::new(1, 3));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_above_block_comment_skips_leader() {
        let mut editor =
            create_syntax_editor("fn main() {\n    /*\n     * beta\n     */\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('O'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    \n    /*\n     * beta\n     */\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(1, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_insert_newline_repeat_enter_trims_previous_comment_continuation_line() {
        let mut editor = create_syntax_editor("// alpha", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "// alpha\n//\n// ");
        assert_eq!(editor.cursor, Cursor::new(2, 3));
    }

    #[test]
    fn test_insert_newline_escape_trims_empty_comment_continuation() {
        let mut editor = create_syntax_editor("// alpha", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "// alpha\n//");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor, Cursor::new(1, 1));
    }

    #[test]
    fn test_open_line_below_escape_trims_empty_line_comment_continuation() {
        let mut editor = create_syntax_editor("// alpha\n", "main.rs");
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Char('o'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "// alpha\n//\n");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor, Cursor::new(1, 1));
    }

    #[test]
    fn test_open_line_above_escape_trims_empty_line_comment_continuation() {
        let mut editor = create_syntax_editor("// alpha\n", "main.rs");
        editor.cursor = Cursor::new(0, 3);

        editor.handle_key(Key::Char('O'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "//\n// alpha\n");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor, Cursor::new(0, 1));
    }

    #[test]
    fn test_insert_newline_escape_trims_empty_block_comment_continuation() {
        let mut editor = create_syntax_editor("/*\n * alpha\n */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(1, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "/*\n * alpha\n *\n */");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor, Cursor::new(2, 1));
    }

    #[test]
    fn test_insert_slash_after_block_comment_leader_compacts_spacing() {
        let mut editor = create_syntax_editor("/*\n * alpha", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(1, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Char('/'));

        assert_eq!(editor.buffer.to_string(), "/*\n * alpha\n */");
        assert_eq!(editor.cursor, Cursor::new(2, 3));
    }

    #[test]
    fn test_insert_newline_continues_ocaml_block_comment_leader() {
        let mut editor = create_syntax_editor("(*\n * alpha", "sample.ml");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(1, 8);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "(*\n * alpha\n * ");
        assert_eq!(editor.cursor, Cursor::new(2, 3));
    }

    #[test]
    fn test_insert_newline_after_block_comment_close_skips_comment_indent() {
        let mut editor = create_syntax_editor("/*\n * Comment\n */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(2, 3);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/*\n * Comment\n */\n\n");
        assert_eq!(editor.cursor, Cursor::new(3, 0));
    }

    #[test]
    fn test_open_line_below_after_block_comment_close_uses_surrounding_indent() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    /*\n     * Comment\n     */\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(3, 6);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    /*\n     * Comment\n     */\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_single_line_block_comment_no_continuation() {
        // `o` from inside a single-line block comment must open a plain new line
        // without continuing the comment.  The new line is placed after the
        // already-closed comment, so no `*` leader is appropriate.
        let mut editor = create_syntax_editor("/* comment */\n", "main.rs");
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "/* comment */\n\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_at_start_of_single_line_block_comment() {
        // `o` with the cursor on the opening `/` must not continue the comment.
        let mut editor = create_syntax_editor("/* comment */\n", "main.rs");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "/* comment */\n\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_past_end_of_single_line_block_comment() {
        // `o` with cursor past the closing `/` must not continue the comment.
        // `line_len` for `/* comment */` is 13 characters.
        let mut editor = create_syntax_editor("/* comment */\n", "main.rs");
        editor.cursor = Cursor::new(0, 13);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "/* comment */\n\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_insert_newline_single_line_block_comment_continues_inside() {
        // Enter in the middle of a single-line block comment must continue the
        // comment with a `*` leader.  Cursor at column 5 is inside the comment
        // body (past `/*` at cols 0–1, before `*/` at cols 11–12).
        // The line is split at col 5: `/* co` | `mment */`, then `* ` is
        // prepended to the new line.
        let mut editor = create_syntax_editor("/* comment */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 5);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/* co\n * mment */");
        assert_eq!(editor.cursor, Cursor::new(1, 3));
    }

    #[test]
    fn test_insert_newline_at_column_zero_of_single_line_block_comment() {
        // Enter at column 0 (before `/*`) must not insert a `*` continuation.
        let mut editor = create_syntax_editor("/* comment */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 0);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "\n/* comment */");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_insert_newline_after_last_slash_of_single_line_block_comment() {
        // Enter with cursor past the final `/` of `*/` must not insert `*`.
        // `/* comment */` is 13 characters; cursor at column 13 is past the end.
        let mut editor = create_syntax_editor("/* comment */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 13);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/* comment */\n\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_insert_newline_right_before_close_of_single_line_block_comment() {
        // Enter with cursor at column 10 — the last body character before `*/`
        // (the space in `/* comment */`) — must continue the comment.
        // Column 10 is the space between `comment` and `*/`; the split leaves
        // `/* comment` on the left and ` */` on the right.
        let mut editor = create_syntax_editor("/* comment */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 10);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/* comment\n *  */");
        assert_eq!(editor.cursor, Cursor::new(1, 3));
    }

    #[test]
    fn test_insert_newline_cursor_before_star_of_close_continues() {
        // In Insert mode the cursor is a bar that sits *before* the character at
        // cursor_column.  Cursor at column 11 means the bar is before the `*` of
        // `*/`: pressing Enter inserts the newline before `*/`, so the left half
        // `/* comment ` has no closing delimiter and the comment should continue.
        let mut editor = create_syntax_editor("/* comment */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/* comment \n * */");
        assert_eq!(editor.cursor, Cursor::new(1, 3));
    }

    #[test]
    fn test_insert_newline_on_closing_slash_no_continuation() {
        // Enter with cursor on the `/` of `*/` (column 12) must not continue
        // the comment: the cursor is between `*` and `/` of the closer.
        let mut editor = create_syntax_editor("/* comment */", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 12);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "/* comment *\n/");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_open_line_above_single_line_block_comment_no_continuation() {
        // `O` from a single-line block comment must open a plain new line above.
        let mut editor = create_syntax_editor("/* comment */\n", "main.rs");
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('O'));

        assert_eq!(editor.buffer.to_string(), "\n/* comment */\n");
        assert_eq!(editor.cursor, Cursor::new(0, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_indented_single_line_block_comment_no_continuation() {
        // `o` from inside an indented single-line block comment must open a plain
        // new line.  The comment is already closed on the current line, so the
        // new line below it is outside the comment.
        let mut editor = create_syntax_editor("    /* comment */\n", "main.rs");
        editor.cursor = Cursor::new(0, 8);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "    /* comment */\n\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_multiline_block_comment_still_continues() {
        // Regression guard: `o` inside a multi-line block comment must still
        // continue the comment with a `*` leader.
        let mut editor = create_syntax_editor("/*\n * alpha\n */", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "/*\n * alpha\n * \n */");
        assert_eq!(editor.cursor, Cursor::new(2, 3));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_adjacent_single_line_block_comments_no_continuation() {
        // `o` from inside the first of two adjacent single-line block comments
        // must open a plain new line.  The first comment is self-contained, so
        // the new line below it is outside the comment.
        let mut editor = create_syntax_editor("/* first */\n/* second */\n", "main.rs");
        editor.cursor = Cursor::new(0, 5);

        editor.handle_key(Key::Char('o'));

        assert_eq!(editor.buffer.to_string(), "/* first */\n\n/* second */\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Insert));
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
    fn test_insert_newline_continuation_line_indents_extra() {
        // Pressing Enter after an incomplete statement (no ; , { } ) ] at the end)
        // must place the new line one extra indent level beyond the anchor line.
        let mut editor = create_syntax_editor("fn main() {\n    let var =\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `    let var =` (line 1, column 13 is past `=`)
        editor.cursor = Cursor::new(1, 13);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        // The new line must be indented by 8 spaces (base 4 + continuation 4).
        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let var =\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_complete_statement_does_not_add_continuation_indent() {
        // A line ending with `;` is a complete statement: the next line stays at
        // the same indent level as the anchor, with no extra continuation indent.
        let mut editor = create_syntax_editor("fn main() {\n    let x = 1;\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `    let x = 1;` (column 14)
        editor.cursor = Cursor::new(1, 14);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x = 1;\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 4));
    }

    #[test]
    fn test_insert_newline_continuation_no_stacking_on_third_line() {
        // When line 2 is already at continuation-indent level relative to line 1,
        // line 3 must match line 2's indent rather than adding another extra level.
        let mut editor =
            create_syntax_editor("fn main() {\n    let x =\n        foo\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `        foo` (line 2, column 11)
        editor.cursor = Cursor::new(2, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        // Line 3 must have the same indent as line 2 (8 spaces), not 12.
        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x =\n        foo\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 8));
    }

    #[test]
    fn test_insert_newline_continuation_after_plus_operator() {
        // A line ending with `+` is an incomplete expression; the continuation
        // indent applies.
        let mut editor = create_syntax_editor("fn main() {\n    let x = a +\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // `    let x = a +` is 15 characters (indices 0-14); column 15 is
        // just past the `+`, which is the correct insert-mode cursor position.
        editor.cursor = Cursor::new(1, 15);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x = a +\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_continuation_after_arrow() {
        // A line ending with `->` (return type arrow) is an incomplete function
        // signature; the continuation indent applies.
        let mut editor = create_syntax_editor("fn foo() ->\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "fn foo() ->\n    \n}\n");
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_reindent_trailing_comment_on_complete_statement_no_extra_indent() {
        // A trailing line comment must not cause a line ending with `;` to be
        // treated as a continuation.  The reindent operator (`==`) on the line
        // following the anchor must produce the same indent as the anchor, not
        // one extra level.
        let mut editor = create_syntax_editor(
            "fn main() {\n    let x = 1; // note\n        wrong;\n}\n",
            "main.rs",
        );
        // Cursor on `        wrong;` (line 2), which should be at 4-space indent.
        editor.cursor = Cursor::new(2, 8);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        // Must be at 4-space indent (same level as the anchor), not 8.
        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x = 1; // note\n    wrong;\n}\n"
        );
    }

    #[test]
    fn test_reindent_trailing_comment_after_operator_gives_continuation_indent() {
        // A trailing comment after an operator must not hide the continuation;
        // the operator before the comment is the last significant character and
        // drives the continuation indent.  The reindent operator (`==`) on the
        // following line must add one extra level.
        let mut editor = create_syntax_editor(
            "fn main() {\n    let x = // assign\n    wrong;\n}\n",
            "main.rs",
        );
        // Cursor on `    wrong;` (line 2), which should be reindented to 8 spaces.
        editor.cursor = Cursor::new(2, 4);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        // The `=` before the comment triggers continuation: 8-space indent.
        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x = // assign\n        wrong;\n}\n"
        );
    }

    #[test]
    /// Opening below `};` keeps block-level indentation instead of continuation indentation.
    fn test_open_line_below_after_brace_semicolon_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    match true {\n        true => (),\n        false => (),\n    };\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(4, 4);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    match true {\n        true => (),\n        false => (),\n    };\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(5, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `};` after a continuation-headed block keeps block-level indentation.
    fn test_open_line_below_after_continuation_headed_brace_semicolon_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n}\n",
            "main.rs",
        );
        // Cursor on the semicolon in `        };`.
        editor.cursor = Cursor::new(5, 9);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `};` keeps block-level indentation when `{` is on its own line.
    fn test_open_line_below_after_continuation_headed_standalone_open_brace_semicolon_uses_block_indent()
     {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match 1\n        {\n            1 => (),\n            _ => (),\n        };\n}\n",
            "main.rs",
        );
        // Cursor on the semicolon in `        };`.
        editor.cursor = Cursor::new(6, 9);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match 1\n        {\n            1 => (),\n            _ => (),\n        };\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(7, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `};` in a continuation-headed block is cursor-column invariant.
    fn test_open_line_below_after_continuation_headed_brace_semicolon_is_cursor_column_invariant() {
        let base = "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n}\n";
        let expected = "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n    \n}\n";
        for column in [8usize, 9, 10] {
            let mut editor = create_syntax_editor(base, "main.rs");
            // Test the closing brace, semicolon, and one trailing space position.
            editor.cursor = Cursor::new(5, column);
            editor.handle_key(Key::Char('o'));
            assert_eq!(editor.buffer.to_string(), expected);
            assert_eq!(editor.cursor, Cursor::new(6, 4));
            assert!(matches!(editor.mode, Mode::Insert));
        }
    }

    #[test]
    /// Enter after `};` in a continuation-headed block keeps block-level indentation.
    fn test_insert_newline_after_continuation_headed_brace_semicolon_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        // Cursor just after `;` in `        };`.
        editor.cursor = Cursor::new(5, 10);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening above `}` after a continuation-headed `};` keeps block-level indentation.
    fn test_open_line_above_after_continuation_headed_brace_semicolon_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n}\n",
            "main.rs",
        );
        // Cursor on the outer `}` line so `O` computes indent from the `};` anchor.
        editor.cursor = Cursor::new(6, 0);

        editor.handle_key(Key::Char('O'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Reindent below `};` in a continuation-headed block resolves to block-level indentation.
    fn test_equal_equal_after_continuation_headed_brace_semicolon_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n        wrong;\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(6, 8);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n    wrong;\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 4));
    }

    #[test]
    /// Opening below continuation-headed `};` uses one tab when tabs indentation is enabled.
    fn test_open_line_below_after_continuation_headed_brace_semicolon_respects_tab_setting() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n}\n",
            "main.rs",
        );
        editor.apply_config(&ConfigSettings {
            indent_width: Some(4),
            indent_with_tabs: Some(true),
            ..ConfigSettings::default()
        });
        editor.cursor = Cursor::new(5, 9);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => (),\n        };\n\t\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 1));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below a comma-terminated Rust match arm keeps arm-level indentation.
    fn test_open_line_below_after_match_arm_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            _ => (),\n        };\n    return;\n}\n",
            "main.rs",
        );
        // Cursor on `1 => (),` to verify `o` aligns with match-arm indentation.
        editor.cursor = Cursor::new(3, 12);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            \n            _ => (),\n        };\n    return;\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 12));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Enter after a comma-terminated Rust match arm keeps arm-level indentation.
    fn test_insert_newline_after_match_arm_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            _ => (),\n        };\n    return;\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        // Cursor just after `,` to emulate pressing Enter at end of arm line.
        editor.cursor = Cursor::new(3, 20);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            \n            _ => (),\n        };\n    return;\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 12));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening above a Rust match arm keeps arm-level indentation after comma anchors.
    fn test_open_line_above_after_match_arm_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            _ => (),\n        };\n    return;\n}\n",
            "main.rs",
        );
        // Cursor on `_ => (),` so `O` computes from the preceding comma anchor.
        editor.cursor = Cursor::new(4, 0);

        editor.handle_key(Key::Char('O'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            \n            _ => (),\n        };\n    return;\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 12));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Reindent after a comma-terminated Rust match arm resolves to arm-level indentation.
    fn test_equal_equal_after_match_arm_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n                wrong;\n            _ => (),\n        };\n    return;\n}\n",
            "main.rs",
        );
        // Cursor on an over-indented line following the comma-terminated match arm.
        editor.cursor = Cursor::new(4, 16);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match 1 {\n            1 => (),\n            wrong;\n            _ => (),\n        };\n    return;\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 12));
    }

    #[test]
    /// Opening below a comma-terminated Rust member keeps member-level indentation.
    fn test_open_line_below_after_member_comma_uses_member_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let value = MyType {\n        first: 1,\n        second: 2,\n    };\n}\n",
            "main.rs",
        );
        // Cursor on `first: 1,` to verify member lines avoid continuation stacking.
        editor.cursor = Cursor::new(2, 8);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let value = MyType {\n        first: 1,\n        \n        second: 2,\n    };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 8));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Reindent after a shorthand comma member stays at member-level indentation.
    fn test_equal_equal_after_shorthand_member_comma_uses_member_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let value = MyType {\n        very_long_field_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n            wrong,\n    };\n}\n",
            "main.rs",
        );
        // The over-indented shorthand member line should align with its siblings.
        editor.cursor = Cursor::new(3, 12);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let value = MyType {\n        very_long_field_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n        wrong,\n    };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 8));
    }

    #[test]
    /// Opening below a shorthand comma member keeps member-level indentation.
    fn test_open_line_below_after_shorthand_member_comma_uses_member_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let value = MyType {\n        very_long_field_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n    };\n}\n",
            "main.rs",
        );
        // `o` on the shorthand comma line should open at member indentation.
        editor.cursor = Cursor::new(2, 8);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let value = MyType {\n        very_long_field_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n        \n    };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 8));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Enter after a shorthand comma member keeps member-level indentation.
    fn test_insert_newline_after_shorthand_member_comma_uses_member_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let value = MyType {\n        very_long_field_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n    };\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        // Enter at the end of the shorthand comma line should preserve member indent.
        editor.cursor = Cursor::new(2, editor.buffer.line_len(2));
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let value = MyType {\n        very_long_field_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n        \n    };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 8));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Reindent after a shorthand destructuring comma stays at pattern-level indentation.
    fn test_equal_equal_after_destructuring_shorthand_comma_uses_pattern_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let MyType {\n        very_long_pattern_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n            very_long_pattern_name_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,\n    } = value;\n}\n",
            "main.rs",
        );
        // Pattern shorthand lines follow the same member-level alignment rule.
        editor.cursor = Cursor::new(3, 12);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let MyType {\n        very_long_pattern_name_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,\n        very_long_pattern_name_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,\n    } = value;\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 8));
    }

    #[test]
    /// Opening below `},` in a match-arm block body keeps match-arm indentation.
    fn test_open_line_below_after_match_arm_block_closer_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => {\n            },\n        };\n}\n",
            "main.rs",
        );
        // Cursor on the inner `},` line should not stack continuation indentation.
        editor.cursor = Cursor::new(5, 12);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false => {\n            },\n            \n        };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 12));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `},` after split `=>` / `{` keeps match-arm indentation.
    fn test_open_line_below_after_split_match_arm_block_closer_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false =>\n            {\n            },\n        };\n}\n",
            "main.rs",
        );
        // Cursor on `},` should align the new line with match arms.
        editor.cursor = Cursor::new(6, 12);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match true {\n            true => (),\n            false =>\n            {\n            },\n            \n        };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(7, 12));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `},` after a multi-line match-arm block keeps match-arm indentation.
    fn test_open_line_below_after_match_arm_block_body_closer_comma_uses_arm_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    let my_var =\n        match opt {\n            None => {\n                return Err(true);\n            },\n            Some(value) => value,\n        };\n}\n",
            "main.rs",
        );
        // Cursor on `},` should deduce the `None => {` owner at same indentation.
        editor.cursor = Cursor::new(5, 12);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    let my_var =\n        match opt {\n            None => {\n                return Err(true);\n            },\n            \n            Some(value) => value,\n        };\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(6, 12));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `},` in a brace block keeps block-level indentation.
    fn test_open_line_below_after_brace_comma_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    if cond {\n        value;\n    },\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(3, 4);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    if cond {\n        value;\n    },\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `});` keeps block-level indentation.
    fn test_open_line_below_after_brace_paren_semicolon_uses_block_indent() {
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    call(\n        value\n    });\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(3, 4);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    call(\n        value\n    });\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below `];` after a Rust inline struct literal keeps function-body indentation.
    fn test_open_line_below_after_array_semicolon_with_inline_struct_uses_body_indent() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    let array = [\n        10,\n        MyStruct {\n            field1: 10,\n            field2: 20,\n        },\n    ];\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(7, 5);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let array = [\n        10,\n        MyStruct {\n            field1: 10,\n            field2: 20,\n        },\n    ];\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(8, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Enter after `];` after a Rust inline struct literal keeps function-body indentation.
    fn test_insert_newline_after_array_semicolon_with_inline_struct_uses_body_indent() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    let array = [\n        10,\n        MyStruct {\n            field1: 10,\n            field2: 20,\n        },\n    ];\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(7, editor.buffer.line_len(7));
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let array = [\n        10,\n        MyStruct {\n            field1: 10,\n            field2: 20,\n        },\n    ];\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(8, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening above `}` after `];` with a Rust inline struct literal keeps function-body indentation.
    fn test_open_line_above_closing_brace_after_array_semicolon_with_inline_struct_uses_body_indent()
     {
        let mut editor = create_syntax_editor(
            "fn main() {\n    let array = [\n        10,\n        MyStruct {\n            field1: 10,\n            field2: 20,\n        },\n    ];\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(8, 0);

        editor.handle_key(Key::Char('O'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let array = [\n        10,\n        MyStruct {\n            field1: 10,\n            field2: 20,\n        },\n    ];\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(8, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Typing `}` after a continuation-indented newline dedents to the block level.
    fn test_insert_closing_brace_after_tail_expression_dedents_to_block_level() {
        let mut editor = create_syntax_editor("fn my_func() {\n    Ok(())\n", "main.rs");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(1, 10);
        editor.begin_history_transaction();

        // Enter after an unterminated tail expression creates a continuation indent.
        editor.handle_key(Key::Char('\n'));
        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    Ok(())\n        \n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));

        // Typing `}` must auto-dedent to column zero immediately.
        editor.handle_key(Key::Char('}'));
        assert_eq!(editor.buffer.to_string(), "fn my_func() {\n    Ok(())\n}\n");
        assert_eq!(editor.cursor, Cursor::new(2, 1));
    }

    #[test]
    /// Opening below a closing brace at EOF without a trailing newline uses the brace indent.
    fn test_open_line_below_after_closing_brace_at_eof_without_trailing_newline() {
        // The file ends on `}` and an earlier continuation-like line exists.
        let mut editor = create_syntax_editor("fn my_func() {\n    Ok(())\n}", "main.rs");
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('o'));

        // The opened line aligns with `}` (column 0), not with `Ok(())` (column 4).
        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    Ok(())\n}\n\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below a closing brace at EOF with a trailing newline uses the brace indent.
    fn test_open_line_below_after_closing_brace_at_eof_with_trailing_newline() {
        // The logical last line is `}` even though the buffer already has `\n` at EOF.
        let mut editor = create_syntax_editor("fn my_func() {\n    Ok(())\n}\n", "main.rs");
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('o'));

        // The opened line aligns with `}` and remains empty.
        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    Ok(())\n}\n\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 0));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    /// Opening below an indented closing brace preserves that closing brace indentation.
    fn test_open_line_below_after_indented_closing_brace_uses_closer_indent() {
        // Opening below the inner `}` should keep 4-space block alignment.
        let mut editor = create_syntax_editor(
            "fn my_func() {\n    if cond {\n        Ok(())\n    }\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(3, 4);

        editor.handle_key(Key::Char('o'));

        // The inserted line follows the inner closing brace indent.
        assert_eq!(
            editor.buffer.to_string(),
            "fn my_func() {\n    if cond {\n        Ok(())\n    }\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 4));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_open_line_below_continuation_indents_extra() {
        // The `o` command must apply the same continuation-indent logic as Enter.
        let mut editor = create_syntax_editor("fn main() {\n    let x =\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x =\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
        assert!(matches!(editor.mode, Mode::Insert));
    }

    #[test]
    fn test_equal_equal_reindents_continuation_line() {
        // The `==` reindent operator must place a continuation line at the
        // correct extra-indented column.
        let mut editor = create_syntax_editor("fn main() {\n    let x =\n    12;\n}\n", "main.rs");
        // Cursor on line 2 (`    12;`) which should be at 8-space continuation indent.
        editor.cursor = Cursor::new(2, 4);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x =\n        12;\n}\n"
        );
    }

    #[test]
    fn test_insert_newline_continuation_unsupported_language_no_indent() {
        // Plain-text files have no indentation profile; neither the existing
        // block-opener indent nor the new continuation indent must fire.
        let mut editor = create_syntax_editor("let x =\n", "notes.txt");
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 7);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(editor.buffer.to_string(), "let x =\n\n");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    fn test_insert_newline_after_fat_arrow_gives_continuation_indent() {
        // `=>` (fat arrow) in a match arm indicates the expression continues.
        let mut editor = create_syntax_editor(
            "fn main() {\n    match x {\n        Some(v) =>\n    }\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        // Cursor at end of `        Some(v) =>`
        editor.cursor = Cursor::new(2, 18);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    match x {\n        Some(v) =>\n            \n    }\n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 12));
    }

    #[test]
    fn test_insert_newline_unmatched_open_bracket_indents() {
        // A line with an unmatched `[` (e.g. an array literal split across
        // lines) must indent the next line one extra level, even though the
        // last character of the line is `,` not `[`.
        let mut editor = create_syntax_editor("fn main() {\n    let v = vec![10,\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor just past the `,` on `    let v = vec![10,` (column 20)
        editor.cursor = Cursor::new(1, 20);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let v = vec![10,\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_unmatched_open_paren_indents() {
        // A function call split across lines has an unmatched `(`: the next
        // line must be indented one extra level.
        let mut editor = create_syntax_editor("fn main() {\n    call(10,\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor just past the `,` on `    call(10,` (column 12)
        editor.cursor = Cursor::new(1, 12);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    call(10,\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_balanced_parens_no_extra_indent() {
        // A line where every `(` is matched by a `)` (e.g. a complete call
        // followed by a trailing operator) must not trigger the
        // unmatched-delimiter rule.
        let mut editor = create_syntax_editor("fn main() {\n    let x = foo() +\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor just past `+` on `    let x = foo() +` (column 19)
        editor.cursor = Cursor::new(1, 19);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        // `foo() +` ends with `+` (operator) → continuation indent applies.
        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x = foo() +\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_call_result_is_continuation() {
        // A line ending with `)` (a closed function call, no trailing `;`) is
        // unterminated — it mirrors Neovim's cin_isterminated which only
        // terminates on `;`, `}`, and `{`.  The next line therefore receives
        // one extra continuation indent level.
        //
        //     let val = call()
        //         + more        ← 8 spaces (4 base + 4 continuation)
        let mut editor = create_syntax_editor("fn main() {\n    let val = call()\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `    let val = call()` (column 20)
        editor.cursor = Cursor::new(1, 20);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let val = call()\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_identifier_is_continuation() {
        // A line ending with a bare identifier (no trailing `;`) is unterminated,
        // matching Neovim's cin_isterminated which only terminates on `;`, `}`,
        // and `{`.  The next line receives one extra continuation indent level.
        //
        //     let var = var2
        //         + more        ← 8 spaces (4 base + 4 continuation)
        let mut editor = create_syntax_editor("fn main() {\n    let var = var2\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `    let var = var2` (column 18)
        editor.cursor = Cursor::new(1, 18);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let var = var2\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_inline_block_comment_does_not_mask_terminator() {
        // An inline block comment between code tokens must not prevent the
        // terminator from being recognised.  `let x /* note */ = 1;` ends with
        // `;` after the comment is skipped, so the next line must stay at the
        // same indent level (4 spaces), not receive a continuation indent.
        let mut editor =
            create_syntax_editor("fn main() {\n    let x /* note */ = 1;\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `    let x /* note */ = 1;` (column 25)
        editor.cursor = Cursor::new(1, 25);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x /* note */ = 1;\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 4));
    }

    #[test]
    fn test_insert_newline_index_result_is_continuation() {
        // A line ending with `]` (a closed index expression, no trailing `;`)
        // is unterminated, matching Neovim's cin_isterminated behaviour.
        let mut editor = create_syntax_editor("fn main() {\n    let val = arr[i]\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `    let val = arr[i]` (column 20)
        editor.cursor = Cursor::new(1, 20);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let val = arr[i]\n        \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_insert_newline_dedents_after_continuation_semicolon() {
        // After a continuation body line ends with `;`, the following line must
        // return to the indent of the statement head, not stay at the
        // continuation body's deeper indent.
        //
        //     let var =
        //         12;
        //     <cursor here — 4 spaces, same as `let var =`>
        let mut editor =
            create_syntax_editor("fn main() {\n    let var =\n        12;\n}\n", "main.rs");
        editor.mode = Mode::Insert;
        // Cursor at end of `        12;` (column 11)
        editor.cursor = Cursor::new(2, 11);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let var =\n        12;\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 4));
    }

    #[test]
    fn test_insert_newline_dedents_after_multiline_continuation_semicolon() {
        // A continuation spanning three lines must dedent all the way back to
        // the statement head after the final `;`.
        //
        //     let var =
        //         foo() +
        //         bar();
        //     <cursor here — 4 spaces>
        let mut editor = create_syntax_editor(
            "fn main() {\n    let var =\n        foo() +\n        bar();\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        // Cursor at end of `        bar();` (column 14)
        editor.cursor = Cursor::new(3, 14);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let var =\n        foo() +\n        bar();\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(4, 4));
    }

    #[test]
    fn test_insert_newline_no_dedent_for_normal_terminated_statement() {
        // Two consecutive terminated statements at the same level must not
        // trigger the backward-scan dedent.  The second `;` line is not
        // preceded by any continuation line, so the new line stays at the
        // same indent.
        let mut editor = create_syntax_editor(
            "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
            "main.rs",
        );
        editor.mode = Mode::Insert;
        // Cursor at end of `    let y = 2;` (column 14)
        editor.cursor = Cursor::new(2, 14);
        editor.begin_history_transaction();

        editor.handle_key(Key::Char('\n'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let x = 1;\n    let y = 2;\n    \n}\n"
        );
        assert_eq!(editor.cursor, Cursor::new(3, 4));
    }

    #[test]
    fn test_equal_equal_reindents_line_after_continuation_semicolon() {
        // The `==` reindent operator on the line after a continuation body
        // must place it at the statement-head level.
        //
        //     let var =
        //         12;
        //         wrong;   ← should be reindented to 4 spaces
        let mut editor = create_syntax_editor(
            "fn main() {\n    let var =\n        12;\n        wrong;\n}\n",
            "main.rs",
        );
        // Cursor on `        wrong;` (line 3).
        editor.cursor = Cursor::new(3, 8);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let var =\n        12;\n    wrong;\n}\n"
        );
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
    fn test_insert_literal_tab_inserts_tab_character() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Ctrl('v'));
        assert_eq!(editor.pending_prefix_label(), Some("^V".to_string()));
        editor.handle_key(Key::Ctrl('i'));

        assert!(editor.mode.is_insert());
        assert_eq!(editor.buffer.to_string(), "a\t");
        assert_eq!(editor.cursor, Cursor::new(0, 2));
        assert_eq!(editor.pending_prefix_label(), None);
    }

    #[test]
    fn test_insert_literal_tab_without_prefix_does_not_insert_tab() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Ctrl('i'));

        assert!(editor.mode.is_insert());
        assert_eq!(editor.buffer.to_string(), "a");
    }

    #[test]
    fn test_insert_literal_printable_character() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Char('x'));

        assert_eq!(editor.buffer.to_string(), "xa");
    }

    #[test]
    fn test_insert_literal_esc_cancels_prefix_and_exits_insert_mode() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Esc);

        assert!(editor.mode.is_normal());
        assert_eq!(editor.buffer.to_string(), "a");
    }

    #[test]
    fn test_insert_literal_unsupported_follower_clears_pending_without_insert() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Backspace);

        assert!(editor.mode.is_insert());
        assert_eq!(editor.buffer.to_string(), "a");
        assert!(!editor.pending_insert_literal);
    }

    #[test]
    fn test_insert_literal_double_ctrl_v_rearms_pending_state() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Ctrl('v'));

        assert!(editor.mode.is_insert());
        assert!(!editor.pending_insert_literal);
        assert_eq!(editor.buffer.to_string(), "a");
    }

    #[test]
    fn test_insert_literal_tab_is_grouped_into_insert_session_undo() {
        let mut editor = create_editor_with_content("a");
        editor.handle_key(Key::Char('a'));
        editor.handle_key(Key::Ctrl('v'));
        editor.handle_key(Key::Ctrl('i'));
        editor.handle_key(Key::Esc);
        assert_eq!(editor.buffer.to_string(), "a\t");
        editor.handle_key(Key::Char('u'));

        assert_eq!(editor.buffer.to_string(), "a");
    }

    #[test]
    fn test_normal_mode_ctrl_v_still_enters_visual_block_mode() {
        let mut editor = create_editor_with_content("abcd");
        editor.handle_key(Key::Ctrl('v'));

        assert_eq!(editor.mode, Mode::Visual(VisualKind::Block));
        assert!(!editor.pending_insert_literal);
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

    /// Picker Ctrl-w uses whitespace-only boundaries and deletes across punctuation.
    #[test]
    fn test_picker_ctrl_w_deletes_across_hyphens() {
        let mut editor = create_editor_with_content("hello");
        editor.open_buffer_switcher();
        for c in "foo-bar-baz".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.mode.picker_string(), Some(""));
    }

    /// Picker Ctrl-w stops only at whitespace, so a two-token query deletes one token.
    #[test]
    fn test_picker_ctrl_w_stops_at_whitespace() {
        let mut editor = create_editor_with_content("hello");
        editor.open_buffer_switcher();
        for c in "foo-bar baz".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.mode.picker_string(), Some("foo-bar "));
    }

    /// Picker Ctrl-w deletes across path separators and extensions in one go.
    #[test]
    fn test_picker_ctrl_w_deletes_across_path_separators() {
        let mut editor = create_editor_with_content("hello");
        editor.open_buffer_switcher();
        for c in "src/main.rs".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.mode.picker_string(), Some(""));
    }

    /// Picker Alt-Backspace uses punctuation-aware word boundaries.
    #[test]
    fn test_picker_alt_backspace_stops_at_hyphens() {
        let mut editor = create_editor_with_content("hello");
        editor.open_buffer_switcher();
        for c in "foo-bar-baz".chars() {
            editor.handle_key(Key::Char(c));
        }

        // Alt-Backspace is encoded as ESC + DEL (0x7f).
        // The boundary logic treats `-` as Punctuation, so only `baz` is deleted.
        editor.handle_key(Key::Alt('\x7f'));
        assert_eq!(editor.mode.picker_string(), Some("foo-bar-"));

        // The next Alt-Backspace skips the trailing `-` then deletes the keyword `bar`,
        // consuming both together.
        editor.handle_key(Key::Alt('\x7f'));
        assert_eq!(editor.mode.picker_string(), Some("foo-"));
    }

    /// Picker Alt-Backspace successive calls peel off one punctuation-separated segment at a time.
    #[test]
    fn test_picker_alt_backspace_successive_calls() {
        let mut editor = create_editor_with_content("hello");
        editor.open_buffer_switcher();
        for c in "foo_bar -baz".chars() {
            editor.handle_key(Key::Char(c));
        }

        editor.handle_key(Key::Alt('\x7f'));
        assert_eq!(editor.mode.picker_string(), Some("foo_bar -"));

        editor.handle_key(Key::Alt('\x7f'));
        assert_eq!(editor.mode.picker_string(), Some("foo_bar "));
    }

    /// Command-mode Ctrl-w is unaffected by the picker change.
    #[test]
    fn test_command_ctrl_w_still_uses_keyword_boundaries_after_picker_change() {
        let mut editor = create_editor_with_content("hello");
        editor.handle_key(Key::Char(':'));
        for c in "src/main.rs".chars() {
            editor.handle_key(Key::Char(c));
        }

        // First Ctrl-w deletes the Keyword run "rs".
        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.input_line(), Some("src/main."));

        // Second Ctrl-w deletes the Punctuation run ".".
        editor.handle_key(Key::Ctrl('w'));
        assert_eq!(editor.input_line(), Some("src/main"));
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
    /// `:A` should jump from one C source file to its corresponding header file.
    fn test_alternate_command_opens_corresponding_file() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.c", "int main(void) { return 0; }\n")
            .expect("write C source");
        tree.write_file("src/main.h", "#pragma once\n")
            .expect("write C header");

        let mut editor = EditorState::new(24);
        // Load the source file as the active named buffer before executing `:A`.
        editor
            .load_file(tree.path().join("src/main.c"))
            .expect("load source file");
        editor.mode = Mode::command_with_text("A");
        editor.execute_command();

        assert_eq!(editor.file_path, tree.path().join("src/main.h"));
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// `:A` should jump from one C header file to a corresponding `.cc` implementation.
    fn test_alternate_command_prefers_cc_for_c_header() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.h", "#pragma once\n")
            .expect("write C header");
        tree.write_file("src/main.cc", "int main_impl() { return 0; }\n")
            .expect("write C++ implementation");
        tree.write_file("src/main.c", "int main(void) { return 0; }\n")
            .expect("write C fallback");

        let mut editor = EditorState::new(24);
        // Load the header before executing `:A` so resolution starts from `.h`.
        editor
            .load_file(tree.path().join("src/main.h"))
            .expect("load header file");
        editor.mode = Mode::command_with_text("A");
        editor.execute_command();

        assert_eq!(editor.file_path, tree.path().join("src/main.cc"));
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// `:A` should report an error when no corresponding file exists on disk.
    fn test_alternate_command_reports_missing_corresponding_file() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.py", "def run() -> int:\n    return 1\n")
            .expect("write Python source");

        let mut editor = EditorState::new(24);
        // Keep the original active file path to confirm no buffer switch occurs.
        let source_path = tree.path().join("src/main.py");
        editor.load_file(&source_path).expect("load source file");
        editor.mode = Mode::command_with_text("A");
        editor.execute_command();

        assert_eq!(editor.file_path, source_path);
        assert_eq!(
            editor.status_message,
            Some("No corresponding file found".to_string())
        );
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
    /// Search should add to jump history so Ctrl-O can return to the original location.
    fn test_search_adds_to_jump_history() {
        let mut editor = create_editor_with_content("line1\nline2\ntarget\nline4");

        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('/'));
        for c in "target\n".chars() {
            editor.handle_key(Key::Char(c));
        }
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);

        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Search jump history should allow forward navigation with Ctrl-I after going back.
    fn test_search_jump_history_forward_backward_roundtrip() {
        let mut editor = create_editor_with_content("line1\nline2\ntarget\nline4");

        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('/'));
        for c in "target\n".chars() {
            editor.handle_key(Key::Char(c));
        }
        assert_eq!(editor.cursor.line(), 2);

        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.cursor.line(), 0);

        editor.handle_key(Key::Ctrl('i'));
        assert_eq!(editor.cursor.line(), 2);
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
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
        #[expect(clippy::permissions_set_readonly_false)]
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
                        keys: "a".to_string(),
                        action: "Go to alternate file".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "d".to_string(),
                        action: "Go to definition".to_string(),
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
                        keys: "f".to_string(),
                        action: "Go to file under cursor".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "F".to_string(),
                        action: "Go to file under cursor at position".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "g".to_string(),
                        action: "Move to first line".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "h".to_string(),
                        action: "Go to corresponding file".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "r".to_string(),
                        action: "Go to references".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "v".to_string(),
                        action: "Recreate last selection".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "$".to_string(),
                        action: "Move line end".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: ".".to_string(),
                        action: "Go to last modification".to_string(),
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
                        keys: "b".to_string(),
                        action: "Open buffer switcher".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "c".to_string(),
                        action: "Toggle line comment".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "C".to_string(),
                        action: "Toggle block comment".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "d".to_string(),
                        action: "Open diagnostics".to_string(),
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
                        keys: "p".to_string(),
                        action: "Paste clipboard after cursor".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "P".to_string(),
                        action: "Paste clipboard before cursor".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "q".to_string(),
                        action: "Update current file and quit".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "r".to_string(),
                        action: "Rename symbol".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "w".to_string(),
                        action: "Save current file".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "y".to_string(),
                        action: "Yank clipboard".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "*".to_string(),
                        action: "Grep word under cursor".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "/".to_string(),
                        action: "Prompt grep command".to_string(),
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
                        keys: "b".to_string(),
                        action: "Align viewport bottom".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "t".to_string(),
                        action: "Align viewport top".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "z".to_string(),
                        action: "Align viewport center".to_string(),
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
                        keys: "a".to_string(),
                        action: "Delete around text object".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "b".to_string(),
                        action: "Delete word backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "B".to_string(),
                        action: "Delete WORD backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "d".to_string(),
                        action: "Delete current line".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "e".to_string(),
                        action: "Delete word end".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "E".to_string(),
                        action: "Delete WORD end".to_string(),
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
                        keys: "g".to_string(),
                        action: "Delete to first line".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "G".to_string(),
                        action: "Delete to last line".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "i".to_string(),
                        action: "Delete inner text object".to_string(),
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
                        keys: "w".to_string(),
                        action: "Delete word forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "W".to_string(),
                        action: "Delete WORD forward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "$".to_string(),
                        action: "Delete to line end".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "%".to_string(),
                        action: "Delete matching delimiter".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "0".to_string(),
                        action: "Delete to line start".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "^".to_string(),
                        action: "Delete to first non-blank".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "{".to_string(),
                        action: "Delete paragraph backward".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "}".to_string(),
                        action: "Delete paragraph forward".to_string(),
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
                    binding: Binding::actions(ActionBinding::Multiple(vec![
                        Action::MoveDown,
                        Action::MoveRight,
                    ])),
                    source: "test".to_string(),
                },
                crate::config::ConfiguredSequenceBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    keys: vec![KeyInput::Char('z'), KeyInput::Char('q')],
                    binding: Binding::actions(ActionBinding::single(Action::SaveCurrentFile)),
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
                        keys: "b".to_string(),
                        action: "Align viewport bottom".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "q".to_string(),
                        action: "Save current file".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "t".to_string(),
                        action: "Align viewport top".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "u".to_string(),
                        action: "Move down -> Move right".to_string(),
                    },
                    SequenceDiscoveryEntry {
                        keys: "z".to_string(),
                        action: "Align viewport center".to_string(),
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
    /// Keep `zt` alignment stable when `j` is a no-op at EOF.
    fn test_zt_then_noop_down_at_eof_preserves_viewport() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\nline 09\nline 10\nline 11\nline 12\n",
        );
        editor.viewport.set_scroll_margin(1);
        editor.handle_resize(80, 10);

        // Align EOF near the top margin first.
        editor.handle_key(Key::Char('G'));
        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('t'));
        let aligned_first_visible = editor.viewport.first_visible_line();

        // Then verify that a no-op down motion does not reflow the viewport.
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 11);
        assert_eq!(editor.viewport.first_visible_line(), aligned_first_visible);
    }

    #[test]
    /// Keep `zt` alignment stable when entering insert mode at EOF.
    fn test_zt_then_insert_mode_at_eof_preserves_viewport() {
        let mut editor = create_editor_with_content(
            "line 01\nline 02\nline 03\nline 04\nline 05\nline 06\nline 07\nline 08\nline 09\nline 10\nline 11\nline 12\n",
        );
        editor.viewport.set_scroll_margin(1);
        editor.handle_resize(80, 10);

        // Align EOF near the top margin first.
        editor.handle_key(Key::Char('G'));
        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('t'));
        let aligned_first_visible = editor.viewport.first_visible_line();

        // Entering insert mode with unchanged cursor should keep the viewport origin.
        editor.handle_key(Key::Char('i'));
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor.line(), 11);
        assert_eq!(editor.viewport.first_visible_line(), aligned_first_visible);
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
    fn test_ctrl_e_keeps_cursor_when_still_visible_after_scroll() {
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
        assert_eq!(editor.cursor.line(), 10);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_ctrl_y_keeps_cursor_when_still_visible_after_scroll() {
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
        assert_eq!(editor.cursor.line(), 16);
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_configured_single_key_binding_beats_built_in_z_prefix() {
        let mut editor = create_editor_with_content("ab\n");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: Binding::actions(ActionBinding::single(Action::MoveRight)),
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
    /// Ensure `diw` on a Rust doc-comment leader removes the punctuation word under the cursor.
    fn test_diw_from_doc_comment_leader_deletes_leader_not_following_identifier() {
        let mut editor = create_editor_with_content("//! Cool\n//! Continued");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), " Cool\n//! Continued");
        assert_eq!(editor.cursor, Cursor::new(0, 0));
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    /// Ensure `diw` in the multiline user repro deletes only the selected `//!` token.
    fn test_diw_in_multiline_doc_comment_repro_deletes_only_selected_leader() {
        let mut editor = create_editor_with_content("//!\n//! Cool\n//! Continued");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('i'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), "//!\n Cool\n//! Continued");
        assert_eq!(editor.cursor, Cursor::new(1, 0));
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
    /// Ensure small-word `dw` deletes punctuation words like `//!` plus following separator.
    fn test_dw_deletes_doc_comment_leader_word() {
        let mut editor = create_editor_with_content("//! Cool");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('w'));

        assert_eq!(editor.buffer.to_string(), "Cool");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    /// Ensure small-word `de` stops at the end of the punctuation word under the cursor.
    fn test_de_deletes_only_doc_comment_leader_word() {
        let mut editor = create_editor_with_content("//! Cool");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('e'));

        assert_eq!(editor.buffer.to_string(), " Cool");
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    /// Ensure small-word `db` from an identifier start removes the prior punctuation word.
    fn test_db_from_identifier_start_deletes_previous_punctuation_word() {
        let mut editor = create_editor_with_content("//! Cool");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('b'));

        assert_eq!(editor.buffer.to_string(), "Cool");
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
    fn test_y_dollar_yanks_from_cursor_to_line_end() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('$'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "alpha beta".to_string(),
                kind: YankKind::Character,
            })
        );
        assert_eq!(editor.buffer.to_string(), "alpha beta");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_y_dollar_yanks_partial_line_from_mid_cursor() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('$'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "beta".to_string(),
                kind: YankKind::Character,
            })
        );
        assert_eq!(editor.buffer.to_string(), "alpha beta");
    }

    #[test]
    fn test_y_dollar_at_line_end_yanks_last_char() {
        let mut editor = create_editor_with_content("ab");
        editor.cursor = Cursor::new(0, 1);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('$'));

        // The cursor is on the last character, so only that character is yanked.
        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "b".to_string(),
                kind: YankKind::Character,
            })
        );
    }

    #[test]
    fn test_y_dollar_on_empty_line_is_noop() {
        let mut editor = create_editor_with_content("one\n\ntwo");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('$'));

        // The line is empty so cursor_to_line_end_selection returns None and the
        // yank buffer is left unchanged.
        assert_eq!(editor.yank_buffer, None);
        assert_eq!(editor.buffer.to_string(), "one\n\ntwo");
    }

    #[test]
    fn test_y_dollar_does_not_include_newline() {
        let mut editor = create_editor_with_content("first\nsecond");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('$'));

        // The yank must stop at the line boundary, not consume the newline.
        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "first".to_string(),
                kind: YankKind::Character,
            })
        );
    }

    #[test]
    fn test_d_dollar_deletes_to_line_end() {
        let mut editor = create_editor_with_content("alpha beta\nz");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('$'));

        assert_eq!(editor.buffer.to_string(), "alpha \nz");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_d_dollar_matches_d_alias_behavior() {
        // d$ and D must produce identical results from the same cursor position.
        let mut editor_op = create_editor_with_content("hello world\nz");
        editor_op.cursor = Cursor::new(0, 6);
        editor_op.handle_key(Key::Char('d'));
        editor_op.handle_key(Key::Char('$'));

        let mut editor_alias = create_editor_with_content("hello world\nz");
        editor_alias.cursor = Cursor::new(0, 6);
        editor_alias.handle_key(Key::Char('D'));

        assert_eq!(
            editor_op.buffer.to_string(),
            editor_alias.buffer.to_string()
        );
        assert_eq!(editor_op.yank_buffer, editor_alias.yank_buffer);
    }

    #[test]
    fn test_c_dollar_changes_to_line_end_and_enters_insert() {
        let mut editor = create_editor_with_content("alpha beta\nz");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('$'));

        assert_eq!(editor.buffer.to_string(), "alpha \nz");
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn test_c_dollar_matches_c_alias_behavior() {
        // c$ and C must enter Insert mode and leave the same buffer content.
        let mut editor_op = create_editor_with_content("hello world\nz");
        editor_op.cursor = Cursor::new(0, 6);
        editor_op.handle_key(Key::Char('c'));
        editor_op.handle_key(Key::Char('$'));

        let mut editor_alias = create_editor_with_content("hello world\nz");
        editor_alias.cursor = Cursor::new(0, 6);
        editor_alias.handle_key(Key::Char('C'));

        assert_eq!(
            editor_op.buffer.to_string(),
            editor_alias.buffer.to_string()
        );
        assert_eq!(editor_op.mode, editor_alias.mode);
        assert_eq!(editor_op.yank_buffer, editor_alias.yank_buffer);
    }

    #[test]
    fn test_y_zero_yanks_from_line_start_to_cursor() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('0'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "alpha ".to_string(),
                kind: YankKind::Character,
            })
        );
        assert_eq!(editor.buffer.to_string(), "alpha beta");
    }

    #[test]
    fn test_y_zero_at_line_start_is_noop() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('0'));

        // Cursor is already at column 0, so the motion covers nothing.
        assert_eq!(editor.yank_buffer, None);
        assert_eq!(editor.buffer.to_string(), "alpha beta");
    }

    #[test]
    fn test_d_zero_deletes_to_line_start() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('0'));

        assert_eq!(editor.buffer.to_string(), "beta");
        assert!(editor.mode.is_normal());
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_d_zero_at_first_column_is_noop() {
        let mut editor = create_editor_with_content("alpha");

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('0'));

        // d0 in column 0 covers no characters and must be a no-op.
        assert_eq!(editor.buffer.to_string(), "alpha");
    }

    #[test]
    fn test_c_zero_deletes_to_line_start_and_enters_insert() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('0'));

        assert_eq!(editor.buffer.to_string(), "beta");
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn test_y_caret_yanks_to_first_non_blank_from_before_it() {
        let mut editor = create_editor_with_content("  alpha");
        // Cursor at column 0, which is before the first non-blank at column 2.
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('^'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "  ".to_string(),
                kind: YankKind::Character,
            })
        );
    }

    #[test]
    fn test_y_caret_yanks_to_first_non_blank_from_after_it() {
        let mut editor = create_editor_with_content("  alpha beta");
        // Cursor at column 7 (the space between "alpha" and "beta"); first non-blank
        // is at column 2.  The range [2, 7) covers "alpha", not including the space
        // at the cursor position because the motion is exclusive of the endpoint.
        editor.cursor = Cursor::new(0, 7);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('^'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "alpha".to_string(),
                kind: YankKind::Character,
            })
        );
    }

    #[test]
    fn test_y_caret_at_first_non_blank_is_noop() {
        let mut editor = create_editor_with_content("  alpha");
        // Cursor exactly on the first non-blank character.
        editor.cursor = Cursor::new(0, 2);

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('^'));

        // No characters between the cursor and first-non-blank, so it is a no-op.
        assert_eq!(editor.yank_buffer, None);
    }

    #[test]
    fn test_d_caret_deletes_to_first_non_blank() {
        let mut editor = create_editor_with_content("  alpha beta");
        // Cursor at column 7 (the space before "beta"); first non-blank at column 2.
        // d^ deletes the range [2, 7) = "alpha", leaving "  " + " beta" = "   beta".
        editor.cursor = Cursor::new(0, 7);

        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('^'));

        assert_eq!(editor.buffer.to_string(), "   beta");
        assert_eq!(editor.cursor.column(), 2);
    }

    #[test]
    fn test_c_caret_deletes_to_first_non_blank_and_enters_insert() {
        let mut editor = create_editor_with_content("  alpha beta");
        // Cursor at column 7; c^ deletes [2, 7) and enters Insert mode.
        editor.cursor = Cursor::new(0, 7);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('^'));

        assert_eq!(editor.buffer.to_string(), "   beta");
        assert_eq!(editor.mode, Mode::Insert);
    }

    #[test]
    fn test_capital_y_yanks_from_cursor_to_line_end() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.cursor = Cursor::new(0, 6);

        editor.handle_key(Key::Char('Y'));

        assert_eq!(
            editor.yank_buffer,
            Some(YankBuffer {
                text: "beta".to_string(),
                kind: YankKind::Character,
            })
        );
        assert_eq!(editor.buffer.to_string(), "alpha beta");
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_capital_y_matches_y_dollar_result() {
        // Y and y$ must store identical yank buffers from the same cursor position.
        let mut editor_y = create_editor_with_content("hello world");
        editor_y.cursor = Cursor::new(0, 6);
        editor_y.handle_key(Key::Char('Y'));
        let yank_y = editor_y.yank_buffer.clone();

        let mut editor_op = create_editor_with_content("hello world");
        editor_op.cursor = Cursor::new(0, 6);
        editor_op.handle_key(Key::Char('y'));
        editor_op.handle_key(Key::Char('$'));
        let yank_op = editor_op.yank_buffer.clone();

        assert_eq!(yank_y, yank_op);
    }

    #[test]
    fn test_operator_discovery_popup_includes_line_end_motion() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('y'));

        let popup = editor.sequence_discovery_popup().expect("popup shown");
        assert!(
            popup.entries.iter().any(|e| e.keys == "$"),
            "discovery popup must list $ as a yank motion"
        );
    }

    #[test]
    fn test_operator_discovery_popup_includes_line_start_motion() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('d'));

        let popup = editor.sequence_discovery_popup().expect("popup shown");
        assert!(
            popup.entries.iter().any(|e| e.keys == "0"),
            "discovery popup must list 0 as a delete motion"
        );
    }

    #[test]
    fn test_operator_discovery_popup_includes_first_non_blank_motion() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('d'));

        let popup = editor.sequence_discovery_popup().expect("popup shown");
        assert!(
            popup.entries.iter().any(|e| e.keys == "^"),
            "discovery popup must list ^ as a delete motion"
        );
    }

    #[test]
    fn test_operator_discovery_popup_sorts_letters_before_non_letters() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('d'));

        let popup = editor.sequence_discovery_popup().expect("popup shown");
        let keys: Vec<&str> = popup.entries.iter().map(|e| e.keys.as_str()).collect();

        let letter_indices: Vec<usize> = keys
            .iter()
            .enumerate()
            .filter(|(_, k)| k.chars().next().is_some_and(|c| c.is_alphabetic()))
            .map(|(i, _)| i)
            .collect();
        let non_letter_indices: Vec<usize> = keys
            .iter()
            .enumerate()
            .filter(|(_, k)| k.chars().next().is_some_and(|c| !c.is_alphabetic()))
            .map(|(i, _)| i)
            .collect();

        let max_letter = letter_indices.iter().max().copied();
        let min_non_letter = non_letter_indices.iter().min().copied();

        if let (Some(max_letter), Some(min_non_letter)) = (max_letter, min_non_letter) {
            assert!(
                max_letter < min_non_letter,
                "letters must sort before non-letters: {keys:?}"
            );
        }

        let letter_keys: Vec<char> = keys
            .iter()
            .filter(|k| k.chars().next().is_some_and(|c| c.is_alphabetic()))
            .map(|k| k.chars().next().unwrap())
            .collect();
        assert!(
            letter_keys.windows(2).all(|w| {
                let a = w[0];
                let b = w[1];
                // Same letter (case-insensitive): lowercase before uppercase
                // Different letters: by letter ordering
                let a_lower = a.to_lowercase().next().unwrap_or(a);
                let b_lower = b.to_lowercase().next().unwrap_or(b);
                if a_lower == b_lower {
                    a.is_lowercase()
                } else {
                    a < b
                }
            }),
            "letter entries must be sorted with lowercase before uppercase within each letter: {keys:?}"
        );

        // Non-letters: sorted by char value (no case conversion)
        let non_letter_keys: Vec<char> = keys
            .iter()
            .filter(|k| k.chars().next().is_some_and(|c| !c.is_alphabetic()))
            .map(|k| k.chars().next().unwrap())
            .collect();
        assert!(
            non_letter_keys.windows(2).all(|w| w[0] <= w[1]),
            "non-letter entries must be sorted: {keys:?}"
        );
    }

    #[test]
    fn test_entry_sort_key_orders_unicode_lowercase_before_uppercase() {
        // Verify Unicode letters like é sort before É (lowercase before uppercase)
        let lowercase_entry = SequenceDiscoveryEntry {
            keys: "é".to_string(),
            action: "Action 1".to_string(),
        };
        let uppercase_entry = SequenceDiscoveryEntry {
            keys: "É".to_string(),
            action: "Action 2".to_string(),
        };
        let sort_key_l = crate::keybindings::entry_sort_key(&lowercase_entry.keys);
        let sort_key_u = crate::keybindings::entry_sort_key(&uppercase_entry.keys);
        assert!(
            sort_key_l < sort_key_u,
            "unicode lowercase should sort before uppercase: {sort_key_l:?} < {sort_key_u:?}"
        );
    }

    #[test]
    fn test_y_dollar_does_not_modify_buffer() {
        let mut editor = create_editor_with_content("alpha beta");

        editor.handle_key(Key::Char('y'));
        editor.handle_key(Key::Char('$'));

        // Yank must be non-destructive.
        assert_eq!(editor.buffer.to_string(), "alpha beta");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_d_dollar_reindent_indent_dedent_accept_line_end_motion() {
        // All six operators must accept $ as a valid operator motion binding.
        let operators = [
            (Key::Char('d'), false),
            (Key::Char('c'), false),
            (Key::Char('>'), false),
            (Key::Char('<'), false),
        ];
        for (op_key, _) in operators {
            let mut editor = create_editor_with_content("alpha beta");
            editor.cursor = Cursor::new(0, 0);
            editor.handle_key(op_key);
            editor.handle_key(Key::Char('$'));
            // The operator must not leave the editor in operator-pending mode.
            assert!(
                editor.pending_operator.is_none(),
                "{:?}$ must resolve and not leave pending operator",
                op_key
            );
        }
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
    fn test_cc_indents_to_current_level_for_supported_language() {
        // `cc` on a line inside a block should re-enter Insert mode with the
        // auto-computed indentation prefix already placed on the blank line.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n    \n}\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_cc_no_indent_for_plain_text() {
        // Files without a recognized language profile receive no auto-indent.
        let mut editor = create_syntax_editor("    hello\nworld\n", "notes.txt");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "\nworld\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(0, 0));
    }

    #[test]
    fn test_cc_on_first_line_no_indent() {
        // The first line has no predecessor, so the indent level is zero.
        let mut editor = create_syntax_editor("fn foo() {\n}\n", "main.rs");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "\n}\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(0, 0));
    }

    #[test]
    fn test_cc_escape_cleans_up_auto_indent() {
        // Pressing Escape after `cc` without typing anything removes the
        // trailing-whitespace-only indent prefix, matching `o`/`O` behaviour.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n\n}\n");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_cc_with_count_uses_previous_line_context() {
        // `2cc` deletes two lines; the auto-indent is computed from the context
        // above the deleted range (the opening brace line).
        let mut editor =
            create_syntax_editor("fn foo() {\n    let a = 1;\n    let b = 2;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n    \n}\n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_cc_on_last_line_without_trailing_newline() {
        // `cc` on the last line when the buffer has no trailing newline inserts
        // a blank line with the correct indentation prefix.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n    \n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_cc_deeply_nested_indentation() {
        // Lines at three indent levels receive the correct three-level prefix.
        let mut editor = create_syntax_editor(
            "fn foo() {\n    if true {\n        let x = 1;\n    }\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(2, 8);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn foo() {\n    if true {\n        \n    }\n}\n"
        );
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_cc_python_indentation() {
        // Python (PythonLike profile) `cc` should restore the 4-space indent.
        let mut editor = create_syntax_editor("def foo():\n    return 1\n", "main.py");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "def foo():\n    \n");
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.cursor, Cursor::new(1, 4));
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
    /// Verify `dG` deletes from the current line through the last line.
    fn test_delete_to_last_line_with_g_motion() {
        let mut editor = create_editor_with_content("alpha\nbeta\ngamma\n");
        // Move cursor to line 1 ("beta") before issuing dG.
        editor.handle_key(Key::Char('j'));
        assert_eq!(
            editor.keybindings.get_operator_binding(Key::Char('G')),
            Some(crate::keybindings::OperatorBinding::LineToLast),
            "G must be bound as LineToLast in OPERATOR_BINDINGS"
        );
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('G'));
        assert_eq!(editor.buffer.to_string(), "alpha\n");
    }

    #[test]
    /// Verify `dgg` deletes from the first line through the current line.
    fn test_delete_to_first_line_with_gg_motion() {
        let mut editor = create_editor_with_content("alpha\nbeta\ngamma\n");
        // Move cursor to line 1 ("beta") before issuing dgg.
        editor.handle_key(Key::Char('j'));
        assert_eq!(
            editor.keybindings.get_operator_binding(Key::Char('g')),
            Some(crate::keybindings::OperatorBinding::LineToFirst),
            "g must be bound as LineToFirst in OPERATOR_BINDINGS"
        );
        editor.handle_key(Key::Char('d'));
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));
        assert_eq!(editor.buffer.to_string(), "gamma\n");
    }

    #[test]
    fn test_normal_mode_motion_remap_does_not_change_operator_motion_keys() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('é'),
                binding: Binding::actions(ActionBinding::single(Action::MoveWordForward)),
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
                binding: Binding::actions(ActionBinding::Multiple(vec![
                    Action::MoveDown,
                    Action::MoveRight,
                ])),
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
                binding: Binding::actions(ActionBinding::Multiple(vec![
                    Action::MoveDown,
                    Action::MoveRight,
                ])),
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
                binding: Binding::actions(ActionBinding::Multiple(vec![
                    Action::MoveDown,
                    Action::MoveRight,
                ])),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));
        editor.handle_key(Key::Char('u'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 1);
    }

    /// Build one replay binding payload for config-driven editor-state tests.
    fn config_replay_binding(trigger: &str, syntax: &str, keys: Vec<KeyInput>) -> Binding {
        Binding::Replay(crate::keybindings::ReplayBinding::new(
            keys,
            syntax.to_string(),
            trigger.to_string(),
            format!("test:keymap.normal:{trigger}"),
        ))
    }

    #[test]
    fn test_replay_binding_executes_operator_sequence() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: config_replay_binding(
                    "z",
                    "diw",
                    vec![
                        KeyInput::Char('d'),
                        KeyInput::Char('i'),
                        KeyInput::Char('w'),
                    ],
                ),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(
            editor.buffer.slice_string(0, editor.buffer.chars_count()),
            " beta"
        );
        assert_eq!(editor.mode, Mode::Normal);
    }

    #[test]
    fn test_replay_binding_preserves_pending_operator_prefix() {
        let mut editor = create_editor_with_content("alpha beta");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: config_replay_binding(
                    "z",
                    "di",
                    vec![KeyInput::Char('d'), KeyInput::Char('i')],
                ),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.buffer.to_string(), "alpha beta");
        assert_eq!(editor.pending_prefix_label(), Some("di".to_string()));
    }

    #[test]
    fn test_replay_binding_repeats_whole_sequence_for_counts() {
        let mut editor = create_editor_with_content("a\nb\nc\nd");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: config_replay_binding("z", "j", vec![KeyInput::Char('j')]),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('3'));
        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 3);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    fn test_replay_binding_detects_direct_recursion() {
        let mut editor = create_editor_with_content("alpha");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: config_replay_binding("z", "z", vec![KeyInput::Char('z')]),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(
            editor.status_message.as_deref(),
            Some("Config replay binding `z` would recurse")
        );
    }

    #[test]
    fn test_replay_binding_detects_indirect_recursion() {
        let mut editor = create_editor_with_content("alpha");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![
                crate::config::ConfiguredBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    key: KeyInput::Char('z'),
                    binding: config_replay_binding("z", "u", vec![KeyInput::Char('u')]),
                    source: "test".to_string(),
                },
                crate::config::ConfiguredBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    key: KeyInput::Char('u'),
                    binding: config_replay_binding("u", "z", vec![KeyInput::Char('z')]),
                    source: "test".to_string(),
                },
            ],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(
            editor.status_message.as_deref(),
            Some("Config replay binding `z` would recurse")
        );
    }

    #[test]
    fn test_replay_binding_allows_non_recursive_nested_replay() {
        let mut editor = create_editor_with_content("a\nb");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![
                crate::config::ConfiguredBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    key: KeyInput::Char('z'),
                    binding: config_replay_binding("z", "u", vec![KeyInput::Char('u')]),
                    source: "test".to_string(),
                },
                crate::config::ConfiguredBinding {
                    mode: crate::keybindings::ModeContext::Normal,
                    key: KeyInput::Char('u'),
                    binding: config_replay_binding("u", "j", vec![KeyInput::Char('j')]),
                    source: "test".to_string(),
                },
            ],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    fn test_replay_binding_replays_tab_jump_forward() {
        let mut editor = create_editor_with_content("line1\nline2\nline3\nline4\nline5");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: config_replay_binding("z", "<Tab>", vec![KeyInput::Ctrl('i')]),
                source: "test".to_string(),
            }],
            ..ConfigSettings::default()
        });

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('4'));
        editor.handle_key(Key::Char('\n'));
        editor.handle_key(Key::Ctrl('o'));
        assert_eq!(editor.cursor.line(), 0);

        editor.handle_key(Key::Char('z'));

        assert_eq!(editor.cursor.line(), 3);
    }

    #[test]
    fn test_replace_config_resets_removed_bindings_to_defaults() {
        let mut editor = create_editor_with_content("ab\ncd");
        editor.apply_config(&ConfigSettings {
            key_bindings: vec![crate::config::ConfiguredBinding {
                mode: crate::keybindings::ModeContext::Normal,
                key: KeyInput::Char('z'),
                binding: Binding::actions(ActionBinding::single(Action::MoveRight)),
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
    fn test_apply_config_updates_tab_width() {
        let mut editor = create_editor_with_content("a\tb");

        editor.apply_config(&ConfigSettings {
            tab_width: Some(4),
            ..ConfigSettings::default()
        });

        assert_eq!(editor.tab_width(), 4);
    }

    #[test]
    fn test_replace_config_resets_tab_width_to_default() {
        let mut editor = create_editor_with_content("a\tb");
        editor.apply_config(&ConfigSettings {
            tab_width: Some(4),
            ..ConfigSettings::default()
        });

        editor.replace_config(&ConfigSettings::default());

        assert_eq!(editor.tab_width(), 8);
    }

    #[test]
    /// Wrapped normal-mode row motion should advance across tab-expanded rows.
    fn test_wrapped_row_motion_handles_tabs_in_normal_mode() {
        let mut editor = create_editor_with_content("a\tb");
        editor.apply_config(&ConfigSettings {
            soft_wrap: Some(true),
            tab_width: Some(8),
            ..ConfigSettings::default()
        });
        editor.viewport.set_width(4);
        editor.cursor = Cursor::new(0, 0);

        // The first wrapped move lands inside the expanded tab cells.
        editor.move_down_wrapped();
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);

        // A tab is one source cell with multiple display cells, so moving down
        // again keeps the cursor anchored to that source tab column.
        editor.move_down_wrapped();
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    /// Wrapped insert-mode row motion should map visual columns through tabs.
    fn test_wrapped_row_motion_handles_tabs_in_insert_mode() {
        let mut editor = create_editor_with_content("a\tb");
        editor.apply_config(&ConfigSettings {
            soft_wrap: Some(true),
            tab_width: Some(8),
            ..ConfigSettings::default()
        });
        editor.viewport.set_width(4);
        editor.mode = Mode::Insert;
        editor.cursor = Cursor::new(0, 3);

        // Insert-mode cursor starts one cell past the last glyph and should
        // move to the tab cell that aligns with the same visual column above.
        editor.move_up_wrapped();
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 1);
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
    fn test_apply_config_can_set_long_line_column() {
        let mut editor = create_editor_with_content("alpha");
        editor.apply_config(&ConfigSettings {
            long_line_column: Some(100),
            ..ConfigSettings::default()
        });
        assert_eq!(editor.long_line_column(), Some(100));
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
    fn test_replace_config_resets_long_line_column_to_default() {
        let mut editor = create_editor_with_content("alpha");
        editor.apply_config(&ConfigSettings {
            long_line_column: Some(100),
            ..ConfigSettings::default()
        });
        editor.replace_config(&ConfigSettings::default());
        assert_eq!(editor.long_line_column(), None);
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
    /// `==` below a Rust attribute keeps top-level indentation.
    fn test_equal_equal_after_rust_attribute_stays_top_level() {
        let mut editor = create_syntax_editor(
            "#![allow(unused)]\n// crate comment\n    wrong;\n",
            "/tmp/main.rs",
        );
        editor.cursor = Cursor::new(2, 4);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "#![allow(unused)]\n// crate comment\nwrong;\n"
        );
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `==` after one bare `}` keeps function-body indentation.
    fn test_equal_equal_after_bare_block_closer_keeps_body_indent() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    if cond {\n        work();\n    }\n        wrong;\n}\n",
            "/tmp/main.rs",
        );
        editor.cursor = Cursor::new(4, 8);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    if cond {\n        work();\n    }\n    wrong;\n}\n"
        );
        assert_eq!(editor.cursor.line(), 4);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    /// `==` reindents Rust comment-only lines to surrounding code depth.
    fn test_equal_equal_reindents_comment_line_in_rust_block() {
        let mut editor = create_syntax_editor("fn main() {\n// note\nx();\n}\n", "/tmp/main.rs");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    // note\nx();\n}\n"
        );
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    /// `==` normalizes Rust block-comment leader lines to one-space offset.
    fn test_equal_equal_reindents_rust_block_comment_leader_with_one_space_offset() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    /*\n      * note\n     */\n}\n",
            "/tmp/main.rs",
        );
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    /*\n     * note\n     */\n}\n"
        );
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 5);
    }

    #[test]
    /// `==` normalizes Rust block-comment closer lines to one-space offset.
    fn test_equal_equal_reindents_rust_block_comment_closer_with_one_space_offset() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    /*\n     * note\n      */\n}\n",
            "/tmp/main.rs",
        );
        editor.cursor = Cursor::new(3, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    /*\n     * note\n     */\n}\n"
        );
        assert_eq!(editor.cursor.line(), 3);
        assert_eq!(editor.cursor.column(), 5);
    }

    #[test]
    /// `==` leaves block-comment opener lines on surrounding code depth.
    fn test_equal_equal_reindents_rust_block_comment_opener_without_one_space_offset() {
        let mut editor = create_syntax_editor("fn main() {\n      /*\n}\n", "/tmp/main.rs");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n    /*\n}\n");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 4);
    }

    #[test]
    /// `==` applies block-comment leader and closer offset for HTML comments.
    fn test_equal_equal_reindents_html_block_comment_closer_with_one_space_offset() {
        let mut editor = create_syntax_editor("<!--\n  -- note\n  -->\n", "/tmp/index.html");
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), "<!--\n  -- note\n -->\n");
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 1);
    }

    #[test]
    /// `==` does not rewrite raw-string payload indentation in Rust.
    fn test_equal_equal_keeps_raw_string_payload_indentation() {
        let mut editor = create_syntax_editor(
            "fn main() {\n    let script = r#\"\nif True:\nprint('x')\n\"#;\n}\n",
            "/tmp/main.rs",
        );
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "fn main() {\n    let script = r#\"\nif True:\nprint('x')\n\"#;\n}\n"
        );
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `==` after a multiline Rust raw string with comma text returns to top-level indentation.
    fn test_equal_equal_after_rust_raw_string_with_comma_keeps_top_level_indent() {
        let mut editor = create_syntax_editor(
            "const string: &str = r#\"hello,\n    world\"#;\n\n    const string2: &str = \"hello2\";\n",
            "/tmp/main.rs",
        );
        // Cursor on the over-indented top-level declaration below the raw string.
        editor.cursor = Cursor::new(3, 4);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "const string: &str = r#\"hello,\n    world\"#;\n\nconst string2: &str = \"hello2\";\n"
        );
        assert_eq!(editor.cursor.line(), 3);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `==` after a multiline Rust raw string without comma text also keeps top-level indentation.
    fn test_equal_equal_after_rust_raw_string_without_comma_keeps_top_level_indent() {
        let mut editor = create_syntax_editor(
            "const string: &str = r#\"hello\n    world\"#;\n\n    const string2: &str = \"hello2\";\n",
            "/tmp/main.rs",
        );
        // This variant locks the same boundary behavior without comma tokens.
        editor.cursor = Cursor::new(3, 4);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(
            editor.buffer.to_string(),
            "const string: &str = r#\"hello\n    world\"#;\n\nconst string2: &str = \"hello2\";\n"
        );
        assert_eq!(editor.cursor.line(), 3);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Opening a line below a multiline raw-string closer does not inherit payload indentation.
    fn test_open_line_below_after_rust_raw_string_closer_keeps_top_level_indent() {
        let mut editor = create_syntax_editor(
            "const string: &str = r#\"hello,\n    world\"#;\nconst after: i32 = 1;\n",
            "/tmp/main.rs",
        );
        // Opening below the raw-string closer must create a top-level blank line.
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('o'));

        assert_eq!(
            editor.buffer.to_string(),
            "const string: &str = r#\"hello,\n    world\"#;\n\nconst after: i32 = 1;\n"
        );
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Whole-file Rust reindent keeps top-level items outside a multiline tail-expression function.
    fn test_visual_equal_whole_file_keeps_top_level_after_tail_expression_function() {
        let content = "\
fn keep_scope() -> bool {
    let significant = gather();
    cond_one(&significant)
        || cond_two(&significant)
        || cond_three(
            &significant,
        )
}

/// must stay top-level
fn still_top_level() {}
";
        let mut editor = create_syntax_editor(content, "/tmp/main.rs");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('G'));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), content);
    }

    #[test]
    /// Whole-file Rust reindent keeps a following top-level function outside a multiline bool tail.
    fn test_visual_equal_whole_file_keeps_top_level_function_after_multiline_bool_tail() {
        let content = "\
pub(crate) fn skip_prefix(line: &str) -> bool {
    line.starts_with(\"#\")
        || line.starts_with(\"//\")
        || line.ends_with(\")\")
}

pub(crate) fn adjust_indent() {}
";
        let mut editor = create_syntax_editor(content, "/tmp/main.rs");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('G'));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), content);
    }

    #[test]
    /// Whole-file Rust reindent keeps inner-block closers aligned to their enclosing block.
    fn test_visual_equal_whole_file_keeps_inner_block_closer_after_continuation() {
        let content = "\
fn nested() {
    if ready() {
        call(
            alpha,
            beta,
        )
    }
}
";
        let mut editor = create_syntax_editor(content, "/tmp/main.rs");

        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('g'));
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('G'));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), content);
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
    /// `>>` on a completely empty line must not insert any whitespace.
    fn test_greater_greater_skips_empty_line() {
        let mut editor = create_syntax_editor("alpha\n\nbeta\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('>'));
        editor.handle_key(Key::Char('>'));

        assert_eq!(editor.buffer.to_string(), "alpha\n\nbeta\n");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `>>` on a whitespace-only line must not add more whitespace.
    fn test_greater_greater_skips_blank_line() {
        let mut editor = create_syntax_editor("alpha\n    \nbeta\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('>'));
        editor.handle_key(Key::Char('>'));

        assert_eq!(editor.buffer.to_string(), "alpha\n    \nbeta\n");
    }

    #[test]
    /// `<<` on a completely empty line must remain a no-op.
    fn test_less_less_skips_empty_line() {
        let mut editor = create_syntax_editor("alpha\n\nbeta\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('<'));
        editor.handle_key(Key::Char('<'));

        assert_eq!(editor.buffer.to_string(), "alpha\n\nbeta\n");
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `<<` on a whitespace-only line must not strip the whitespace.
    fn test_less_less_skips_blank_line() {
        let mut editor = create_syntax_editor("alpha\n    \nbeta\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('<'));
        editor.handle_key(Key::Char('<'));

        assert_eq!(editor.buffer.to_string(), "alpha\n    \nbeta\n");
    }

    #[test]
    /// Visual `>` over a selection that includes empty lines must skip those lines.
    fn test_visual_greater_skips_empty_lines_in_selection() {
        let mut editor = create_syntax_editor("alpha\n\nbeta\n", "/tmp/notes.txt");
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('>'));

        assert_eq!(editor.buffer.to_string(), "    alpha\n\n    beta\n");
    }

    #[test]
    /// `==` on an empty line must remain a no-op (existing behavior).
    fn test_equal_equal_skips_empty_line() {
        let mut editor = create_syntax_editor("fn main() {\n\n}\n", "/tmp/test.rs");
        editor.cursor = Cursor::new(1, 0);

        editor.handle_key(Key::Char('='));
        editor.handle_key(Key::Char('='));

        assert_eq!(editor.buffer.to_string(), "fn main() {\n\n}\n");
        assert_eq!(editor.cursor.line(), 1);
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

    /// `<count>j` with soft-wrap enabled must jump `count` logical buffer lines,
    /// not `count` visual/wrapped rows.
    ///
    /// The buffer has three logical lines; the first spans two visual rows when
    /// the viewport is 4 columns wide.  Pressing `2j` should land on logical
    /// line 2 directly instead of stopping midway through the first wrapped line.
    #[test]
    fn test_counted_j_moves_by_logical_lines_with_soft_wrap() {
        // Line 0: "abcdefgh" wraps into two visual rows at width 4.
        // Line 1: "xx"
        // Line 2: "yy"
        let mut editor = create_editor_with_content("abcdefgh\nxx\nyy");
        editor.handle_resize(4, 10);
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('j'));

        // Must land on logical line 2, not on the second visual row of line 0.
        assert_eq!(editor.cursor.line(), 2);
    }

    /// `<count>k` with soft-wrap enabled must jump `count` logical buffer lines
    /// upward, not `count` visual/wrapped rows.
    #[test]
    fn test_counted_k_moves_by_logical_lines_with_soft_wrap() {
        let mut editor = create_editor_with_content("xx\nyy\nabcdefgh");
        editor.handle_resize(4, 10);
        // Start on logical line 2.
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char('k'));

        // Must land on logical line 0, not on a visual row of line 2.
        assert_eq!(editor.cursor.line(), 0);
    }

    /// `1j` (explicit count of 1) with soft-wrap enabled must move to the next
    /// logical line, not to the next visual row within the same wrapped line.
    #[test]
    fn test_count_one_j_moves_to_next_logical_line_with_soft_wrap() {
        // Line 0: "abcdefgh" wraps into two visual rows at width 4.
        // Line 1: "zz"
        let mut editor = create_editor_with_content("abcdefgh\nzz");
        editor.handle_resize(4, 10);
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('j'));

        // Must land on logical line 1, not on visual row 1 of line 0.
        assert_eq!(editor.cursor.line(), 1);
    }

    /// Plain `j` (no count) with soft-wrap enabled continues to move by visual
    /// wrapped rows, not by logical lines.
    #[test]
    fn test_plain_j_still_moves_by_wrapped_rows_with_soft_wrap() {
        // Line 0: "abcdefgh" wraps into two visual rows at width 4.
        let mut editor = create_editor_with_content("abcdefgh\nzz");
        editor.handle_resize(4, 10);
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('j'));

        // Plain j must stay on line 0 (moved to second visual row of same line).
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 4);
    }

    /// `<count>j` preserves `desired_visual_column` so a subsequent plain `j`
    /// continues moving toward the same visual goal column.
    #[test]
    fn test_counted_j_preserves_desired_visual_column() {
        // Line 0: "abcde" – cursor at column 3 (visual column 3).
        // Line 1: "xx"   – short, cursor will clamp.
        // Line 2: "abcde" – plain j from line 1 should restore column 3.
        let mut editor = create_editor_with_content("abcde\nxx\nabcde");
        editor.handle_resize(20, 10);
        editor.cursor = Cursor::new(0, 3);

        // 1j: jump one logical line to "xx"; column clamps to 1 (last valid col).
        editor.handle_key(Key::Char('1'));
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 1);

        // Plain j from "xx" to "abcde": desired_visual_column was preserved,
        // so cursor should restore to column 3.
        editor.handle_key(Key::Char('j'));
        assert_eq!(editor.cursor.line(), 2);
        assert_eq!(editor.cursor.column(), 3);
    }

    /// `<count>j` with a count that exceeds remaining lines clamps to the last
    /// logical line without panicking.
    #[test]
    fn test_counted_j_clamps_at_last_line_with_soft_wrap() {
        let mut editor = create_editor_with_content("aa\nbb\ncc");
        editor.handle_resize(20, 10);
        editor.cursor = Cursor::new(0, 0);

        editor.handle_key(Key::Char('9'));
        editor.handle_key(Key::Char('9'));
        editor.handle_key(Key::Char('j'));

        assert_eq!(editor.cursor.line(), 2);
    }

    /// `<count>k` from the first line clamps to line 0 without panicking.
    #[test]
    fn test_counted_k_clamps_at_first_line_with_soft_wrap() {
        let mut editor = create_editor_with_content("aa\nbb\ncc");
        editor.handle_resize(20, 10);
        editor.cursor = Cursor::new(2, 0);

        editor.handle_key(Key::Char('9'));
        editor.handle_key(Key::Char('9'));
        editor.handle_key(Key::Char('k'));

        assert_eq!(editor.cursor.line(), 0);
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
        // EditorState::new(24) → height = 24 - RESERVED_SCREEN_ROWS(3) = 21. scroll_margin=3.
        // page_size = height - 1 = 20.
        // 2ctrl-f: scroll_rows=40, viewport 0→40, cursor at 40+3=43.
        // 2ctrl-b: scroll_rows=40, viewport 40→0, cursor at bottom_row = 21-1-3=17.
        let lines = (1..=200).map(|i| format!("line{}", i)).collect::<Vec<_>>();
        let mut editor = create_editor_with_content(&lines.join("\n"));

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Ctrl('f'));
        // Cursor lands at scroll_margin rows from top of new viewport.
        assert!(editor.cursor.line() >= 40);

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Ctrl('b'));
        // Cursor lands at bottom-margin row of the new viewport (viewport scrolled back to 0).
        assert_eq!(editor.cursor.line(), 17);
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
    /// Picker bracketed paste should produce one single-line query.
    fn test_flattened_picker_paste_text_collapses_line_breaks() {
        assert_eq!(EditorState::flattened_picker_paste_text("alpha"), "alpha");
        assert_eq!(
            EditorState::flattened_picker_paste_text("alpha\nbeta"),
            "alpha beta"
        );
        assert_eq!(
            EditorState::flattened_picker_paste_text("alpha\n\nbeta\n"),
            "alpha beta"
        );
        assert_eq!(
            EditorState::flattened_picker_paste_text("cafe\n東京"),
            "cafe 東京"
        );
        assert_eq!(EditorState::flattened_picker_paste_text("\n\n"), "");
    }

    #[test]
    /// Regression test for undo after a large Insert-mode bracketed paste.
    fn test_undo_after_large_insert_mode_bracketed_paste_resets_wrapped_viewport_origin() {
        let payload = (1..=100)
            .map(|line| format!("line{line:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = create_editor_with_content("");
        // Use a short viewport so the paste scrolls well away from the top
        // before undo collapses the buffer back to a single logical line.
        editor.handle_resize(20, 8);
        editor.handle_key(Key::Char('i'));
        editor.handle_paste(&payload);
        editor.exit_to_normal_mode();

        assert!(editor.first_visible_line() > 0);

        editor.handle_key(Key::Char('u'));

        assert_eq!(editor.buffer.lines_count(), 1);
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.first_visible_line(), 0);
        assert_eq!(editor.first_visible_row(), 0);
    }

    #[test]
    /// Insert-mode bracketed paste ending with a newline should materialize a real EOF blank line.
    fn test_insert_mode_bracketed_paste_trailing_newline_materializes_eof_blank_line() {
        let mut editor = create_editor_with_content("");

        // Use the real Insert-mode entry path so the paste shares the same undo
        // transaction and cursor semantics as interactive editing.
        editor.handle_key(Key::Char('i'));
        editor.handle_paste("line\n");

        assert_eq!(editor.buffer.to_string(), "line\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    /// Normal-mode bracketed paste ending with a newline should materialize a real EOF blank line.
    fn test_normal_mode_bracketed_paste_trailing_newline_materializes_eof_blank_line() {
        let mut editor = create_editor_with_content("");

        editor.handle_paste("line\n");

        assert_eq!(editor.buffer.to_string(), "line\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
    }

    #[test]
    /// Normal-mode bracketed paste without a trailing newline should end on the last inserted character.
    fn test_normal_mode_bracketed_paste_without_trailing_newline_ends_on_last_character() {
        let mut editor = create_editor_with_content("");

        editor.handle_paste("line");

        assert_eq!(editor.buffer.to_string(), "line");
        assert_eq!(editor.cursor, Cursor::new(0, 3));
    }

    #[test]
    /// Visual bracketed paste ending with a newline should replace the selection and keep the EOF blank line real.
    fn test_visual_mode_bracketed_paste_trailing_newline_materializes_eof_blank_line() {
        let mut editor = create_editor_with_content("x");

        // Select the only character so the replacement paste runs at EOF after
        // the shared Visual delete path removes the original content.
        editor.handle_key(Key::Char('v'));
        editor.handle_paste("line\n");

        assert_eq!(editor.buffer.to_string(), "line\n\n");
        assert_eq!(editor.buffer.lines_count(), 2);
        assert_eq!(editor.cursor, Cursor::new(1, 0));
        assert!(matches!(editor.mode, Mode::Normal));
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
    /// Diagnostic motions without active diagnostics should surface informational feedback.
    fn test_diagnostic_navigation_without_diagnostics_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha\nbeta");

        // Without LSP diagnostics, each motion should report the no-result state.
        editor.goto_next_diagnostic();
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No diagnostics in active buffer")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);

        editor.goto_prev_diagnostic();
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No diagnostics in active buffer")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// Diagnostic motions at list boundaries should use informational no-result messages.
    fn test_diagnostic_boundary_navigation_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha\nbeta");
        apply_test_diagnostics(
            &mut editor,
            "/tmp/diagnostics.rs",
            vec![(0, 0, 5, LspDiagnosticSeverity::Error, "alpha error")],
        );

        // One diagnostic has no predecessor at the initial cursor position.
        editor.goto_prev_diagnostic();
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No previous diagnostic")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);

        // After jumping to the single entry, there is no next diagnostic.
        editor.goto_next_diagnostic();
        editor.goto_next_diagnostic();
        assert_eq!(editor.status_message.as_deref(), Some("No next diagnostic"));
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// Opening the diagnostics picker without diagnostics should report informational feedback.
    fn test_open_diagnostics_picker_without_diagnostics_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");

        editor.open_diagnostics_picker();

        assert_eq!(
            editor.status_message.as_deref(),
            Some("No diagnostics in active buffer")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
        assert_eq!(popup.query_suffix, "2/2 ");
        assert_eq!(popup.entries.len(), 2);
        assert!(popup.entries[0].label.contains("alpha error"));
        assert!(popup.entries[1].label.contains("beta warning"));
    }

    #[test]
    /// Confirming a diagnostics-picker target should center the diagnostic line.
    fn test_confirm_diagnostics_picker_selection_centers_destination_line() {
        let lines = (1..=40)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut editor = create_editor_with_content(&lines);
        editor.viewport.set_soft_wrap(false);
        editor.viewport.set_scroll_margin(1);
        editor.viewport.set_height(8);
        apply_test_diagnostics(
            &mut editor,
            "/tmp/diagnostics.rs",
            vec![(20, 0, 4, LspDiagnosticSeverity::Error, "target diagnostic")],
        );
        editor.open_diagnostics_picker();

        editor.confirm_diagnostics_picker_selection();

        assert_eq!(editor.cursor.line(), 20);
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.viewport.first_visible_line(), 16);
    }

    #[test]
    /// Search-picker target jumps should center the destination line after opening the file.
    fn test_goto_search_picker_target_centers_destination_line() {
        let source = TempFile::with_suffix("_main.rs").expect("create source file");
        source
            .write_all(b"fn main() {}\n")
            .expect("seed source file");
        let target = TempFile::with_suffix("_search.rs").expect("create target file");
        // Keep enough context so center alignment is distinguishable from simple visibility.
        let lines = (1..=40)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        target
            .write_all(format!("{lines}\n").as_bytes())
            .expect("seed target file");
        let mut editor = EditorState::new(24);
        editor.load_file(source.path()).expect("load source file");
        editor.viewport.set_soft_wrap(false);
        editor.viewport.set_scroll_margin(1);
        editor.viewport.set_height(8);

        editor.goto_search_picker_target(&SearchPickerTarget {
            file_path: target.path().to_path_buf(),
            line: 20,
            column: 0,
        });

        assert_eq!(editor.file_path, target.path());
        assert_eq!(editor.cursor.line(), 20);
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.viewport.first_visible_line(), 16);
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
    /// Named active buffers should keep external-file monitoring on the background poll path.
    fn test_named_active_buffer_keeps_background_poll_active() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");

        assert!(editor.needs_background_poll());
    }

    #[test]
    /// Unnamed-only sessions should not poll in the background without other pending work.
    fn test_unnamed_session_without_pending_work_does_not_poll_background() {
        let editor = create_editor_with_content("alpha");

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
    /// Rename prefill for markdown stops at a dash because the profile uses the
    /// plain ASCII identifier pattern, which does not include dashes.
    fn test_prompt_rename_symbol_in_markdown_stops_at_dash() {
        let mut editor = create_editor_with_content("project-name");
        editor.file_path = PathBuf::from("notes.md");

        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('r'));

        assert_eq!(editor.mode.command_string(), Some("rename project"));
    }

    #[test]
    /// Typing a partial command name should refresh command completion in prompt state.
    fn test_command_prompt_typing_refreshes_command_completion_popup() {
        let mut editor = create_editor_with_content("alpha");

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('w'));
        editor.handle_key(Key::Char('r'));

        let popup = editor
            .command_completion_popup()
            .expect("command completion popup");
        assert!(popup.entries.iter().any(|entry| entry.label == "write"));
    }

    #[test]
    /// Typing a supported argument prefix should refresh command-argument completion in prompt state.
    fn test_command_prompt_typing_refreshes_argument_completion_popup() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("state/file.txt", "demo\n")
            .expect("write file");
        let mut editor = create_editor_with_content("alpha");
        let prefix = tree.path().join("st").display().to_string();

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('e'));
        editor.handle_key(Key::Char(' '));
        for ch in prefix.chars() {
            editor.handle_key(Key::Char(ch));
        }

        // Command argument scans run on a background worker so the test polls
        // until the async result has been merged back into editor state.
        for _ in 0..20 {
            editor.poll_background_tasks();
            if editor.command_completion_popup().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let popup = editor
            .command_completion_popup()
            .expect("command argument completion popup");
        assert!(popup.entries.iter().any(|entry| entry.label == "state/"));
    }

    #[test]
    /// Typing a narrower async command argument prefix should keep the popup visible.
    fn test_command_prompt_typing_keeps_async_argument_popup_visible_while_refreshing() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("state/file.txt", "demo\n")
            .expect("write file");
        let mut editor = create_editor_with_content("alpha");
        let prefix = tree.path().join("s").display().to_string();

        editor.handle_key(Key::Char(':'));
        editor.handle_key(Key::Char('e'));
        editor.handle_key(Key::Char(' '));
        for ch in prefix.chars() {
            editor.handle_key(Key::Char(ch));
        }

        for _ in 0..20 {
            editor.poll_background_tasks();
            if editor.command_completion_popup().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        editor.handle_key(Key::Char('t'));

        // Retaining the filtered popup avoids a hide/show blink while the next
        // background scan recomputes the authoritative directory entries.
        let popup = editor
            .command_completion_popup()
            .expect("retained command argument completion popup");
        assert!(popup.entries.iter().any(|entry| entry.label == "state/"));
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
            .open_buffer(tree.path().join("src/lib.rs"))
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
    /// Empty rename results should report informational no-result feedback.
    fn test_apply_rename_lookup_result_not_found_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_rename_lookup = Some(ActiveRenameLookup {
            token: 13,
            document_version: 2,
            request_edit_generation: 0,
            new_name: "beta".to_string(),
        });

        let changed = editor.apply_rename_lookup_result(RenameLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 13,
            document_version: 2,
            outcome: RenameLookupOutcome::NotFound,
        });

        assert!(changed);
        assert_eq!(editor.active_rename_lookup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No rename changes found")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// Rename edit payloads that apply zero text edits should report informational feedback.
    fn test_apply_rename_lookup_result_noop_edit_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_rename_lookup = Some(ActiveRenameLookup {
            token: 14,
            document_version: 2,
            request_edit_generation: 0,
            new_name: "alpha".to_string(),
        });

        let changed = editor.apply_rename_lookup_result(RenameLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 14,
            document_version: 2,
            outcome: RenameLookupOutcome::Applied(LspWorkspaceEdit {
                document_edits: vec![],
            }),
        });

        assert!(changed);
        assert_eq!(editor.active_rename_lookup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("Rename produced no changes")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
    /// Empty hover lookups should clear stale popups and report informational feedback.
    fn test_apply_hover_lookup_result_not_found_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_hover_lookup = Some(ActiveHoverLookup {
            token: 3,
            document_version: 5,
        });
        editor.hover_popup = Some(HoverPopup::new("stale hover"));

        let changed = editor.apply_hover_lookup_result(HoverLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 3,
            document_version: 5,
            outcome: HoverLookupOutcome::NotFound,
        });

        assert!(changed);
        assert_eq!(editor.active_hover_lookup, None);
        assert_eq!(editor.hover_popup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No hover information found")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
            .open_buffer(second.path())
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
    fn test_definition_location_picker_popup_includes_preview() {
        let first = TempFile::with_suffix("_main.rs").expect("create main file");
        first.write_all(b"fn main() {}\n").expect("seed main file");
        let target = TempFile::with_suffix("_target.rs").expect("create target file");
        target
            .write_all(b"mod alpha;\npub fn helper() {}\n")
            .expect("seed target file");
        let mut editor = EditorState::new(24);

        editor.load_file(first.path()).expect("load main file");
        let main_id = editor.active_buffer_id();
        editor
            .open_buffer(target.path())
            .expect("open target buffer");
        editor.activate_buffer(main_id);
        editor.open_location_picker(
            NavigationKind::Definition,
            vec![NavigationTarget {
                file_path: target.path().to_path_buf(),
                line: 1,
                character: 7,
                display_label: format!("{}:2:8", target.path().display()),
            }],
        );

        let popup = editor.picker_popup().expect("definition picker popup");
        let preview = popup.preview.expect("preview pane");
        assert!(
            preview
                .lines
                .iter()
                .any(|line| line.highlighted && line.text.contains("pub fn helper() {}"))
        );
    }

    #[test]
    /// Confirming a location-picker target should center the destination line.
    fn test_confirm_location_picker_selection_centers_destination_line() {
        let source = TempFile::with_suffix("_main.rs").expect("create source file");
        source
            .write_all(b"fn main() {}\n")
            .expect("seed source file");
        let target = TempFile::with_suffix("_target.rs").expect("create target file");
        // Use enough lines so centered alignment has room to move the viewport.
        let lines = (1..=40)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        target
            .write_all(format!("{lines}\n").as_bytes())
            .expect("seed target file");
        let mut editor = EditorState::new(24);
        editor.load_file(source.path()).expect("load source file");
        editor.viewport.set_soft_wrap(false);
        editor.viewport.set_scroll_margin(1);
        editor.viewport.set_height(8);
        editor.open_location_picker(
            NavigationKind::Definition,
            vec![NavigationTarget {
                file_path: target.path().to_path_buf(),
                line: 20,
                character: 0,
                display_label: format!("{}:21:1", target.path().display()),
            }],
        );

        editor.confirm_location_picker_selection();

        assert_eq!(editor.file_path, target.path());
        assert_eq!(editor.cursor.line(), 20);
        assert_eq!(editor.cursor.column(), 0);
        assert_eq!(editor.viewport.first_visible_line(), 16);
    }

    #[test]
    /// Location-picker jumps near the start of a file should clamp viewport origin to line zero.
    fn test_confirm_location_picker_selection_clamps_centering_at_file_start() {
        let source = TempFile::with_suffix("_main.rs").expect("create source file");
        source
            .write_all(b"fn main() {}\n")
            .expect("seed source file");
        let target = TempFile::with_suffix("_target.rs").expect("create target file");
        let lines = (1..=10)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        target
            .write_all(format!("{lines}\n").as_bytes())
            .expect("seed target file");
        let mut editor = EditorState::new(24);
        editor.load_file(source.path()).expect("load source file");
        editor.viewport.set_soft_wrap(false);
        editor.viewport.set_scroll_margin(1);
        editor.viewport.set_height(8);
        editor.open_location_picker(
            NavigationKind::Definition,
            vec![NavigationTarget {
                file_path: target.path().to_path_buf(),
                line: 1,
                character: 0,
                display_label: format!("{}:2:1", target.path().display()),
            }],
        );

        editor.confirm_location_picker_selection();

        assert_eq!(editor.file_path, target.path());
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.viewport.first_visible_line(), 0);
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
    /// Empty code-action lookups should report informational no-result feedback.
    fn test_apply_code_action_lookup_result_not_found_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");
        editor.file_path = PathBuf::from("src/main.rs");
        editor.active_code_action_lookup = Some(ActiveCodeActionLookup {
            token: 15,
            document_version: 1,
            request_edit_generation: 0,
        });

        let changed = editor.apply_code_action_lookup_result(CodeActionLookupResult {
            buffer_id: editor.active_buffer_id,
            lookup_token: 15,
            document_version: 1,
            outcome: CodeActionLookupOutcome::NotFound,
        });

        assert!(changed);
        assert_eq!(editor.active_code_action_lookup, None);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No supported code actions available")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// A single-target lookup whose destination matches the current position clears the
    /// transient resolving message even though no cursor move occurs.
    fn test_apply_navigation_lookup_result_clears_message_on_same_position_single_target() {
        let mut editor = create_editor_with_content("fn main() {}");
        editor.file_path = PathBuf::from("src/main.rs");
        // Simulate the transient message shown while the request was in-flight.
        editor.show_transient_status_message("Resolving definition...");
        editor.active_navigation_lookup = Some(ActiveNavigationLookup {
            kind: NavigationKind::Definition,
            token: 1,
            document_version: 0,
        });
        let current_path = editor.file_path.clone();
        // The cursor starts at line 0, column 0, so returning that same position
        // is a same-location result and should not move the cursor.
        let changed = editor.apply_navigation_lookup_result(NavigationLookupResult {
            kind: NavigationKind::Definition,
            buffer_id: editor.active_buffer_id,
            lookup_token: 1,
            document_version: 0,
            outcome: NavigationLookupOutcome::Single(NavigationTarget {
                file_path: current_path,
                line: 0,
                character: 0,
                display_label: "src/main.rs:1:1".to_string(),
            }),
        });

        // The result is accepted and the lookup is cleared.
        assert!(changed);
        assert_eq!(editor.active_navigation_lookup, None);
        // The transient message must be gone so the terminal message row does not
        // show a stale "Resolving definition..." after the no-op jump.
        assert_eq!(editor.status_message, None);
        // The cursor must remain at the original position.
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
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
    /// Regression: `c` on a single middle line in visual line mode leaves one empty line
    /// in place for typing, matching vim behaviour.
    fn test_visual_line_change_single_middle_line_keeps_empty_line() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");
        // Position cursor on the second line ("two").
        editor.handle_key(Key::Char('j'));
        // Enter visual line mode and immediately press `c`.
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        // An empty line must remain between "one" and "three".
        assert_eq!(editor.buffer.to_string(), "one\n\nthree");
        assert!(editor.mode.is_insert());
        // Cursor lands on the empty line at column 0.
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression: `c` on multiple consecutive lines in visual line mode leaves one
    /// empty line in place for typing, matching vim behaviour.
    fn test_visual_line_change_multiple_middle_lines_keeps_empty_line() {
        let mut editor = create_editor_with_content("a\nb\nc\nd");
        // Position cursor on the second line ("b").
        editor.handle_key(Key::Char('j'));
        // Enter visual line mode, extend to the third line ("c"), then change.
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('c'));

        // An empty line must remain between "a" and "d".
        assert_eq!(editor.buffer.to_string(), "a\n\nd");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Regression: `c` on the first line in visual line mode leaves one empty line at
    /// the top, matching vim behaviour.
    fn test_visual_line_change_first_line_keeps_empty_line() {
        let mut editor = create_editor_with_content("first\nsecond");
        // Cursor already on line 0 ("first").
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        // The first line is replaced by an empty line; "second" stays.
        assert_eq!(editor.buffer.to_string(), "\nsecond");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `c` on the sole line in a buffer with no trailing newline inserts a blank
    /// line and enters Insert mode.
    fn test_visual_line_change_only_line_no_trailing_newline_empties_buffer() {
        let mut editor = create_editor_with_content("only");
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        // The blank line slot is always inserted so the user has a line to type on.
        assert_eq!(editor.buffer.to_string(), "\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `c` on the last line of a multi-line buffer without a trailing newline
    /// inserts a blank line after the preceding content and enters Insert mode.
    fn test_visual_line_change_last_line_keeps_empty_line() {
        let mut editor = create_editor_with_content("first\nlast");
        // Position cursor on the last line ("last").
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        // A blank line slot is inserted after "first\n" so the user has a line
        // to type on regardless of whether following lines existed.
        assert_eq!(editor.buffer.to_string(), "first\n\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.line(), 1);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// `c` on all lines of a buffer inserts a blank line and enters Insert mode.
    fn test_visual_line_change_all_lines_empties_buffer() {
        let mut editor = create_editor_with_content("a\nb\nc");
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('c'));

        // The blank line slot is always inserted so the user has a line to type on.
        assert_eq!(editor.buffer.to_string(), "\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 0);
    }

    #[test]
    /// Undoing a visual line `c` fully restores the original lines.
    fn test_visual_line_change_undo_restores_original_lines() {
        let mut editor = create_editor_with_content("one\ntwo\nthree");
        // Change the middle line.
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));
        // Leave insert mode and undo.
        editor.handle_key(Key::Esc);
        assert_eq!(editor.buffer.to_string(), "one\n\nthree");
        editor.handle_key(Key::Char('u'));

        assert_eq!(editor.buffer.to_string(), "one\ntwo\nthree");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_visual_line_change_indents_to_current_level_for_supported_language() {
        // `V` + `c` on a line inside a block should land the cursor at the
        // auto-computed indentation level, matching `cc` behaviour.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n    \n}\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_visual_line_change_no_indent_for_plain_text() {
        // Files without a recognized language profile receive no auto-indent.
        let mut editor = create_syntax_editor("    hello\nworld\n", "notes.txt");
        editor.cursor = Cursor::new(0, 4);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "\nworld\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor, Cursor::new(0, 0));
    }

    #[test]
    fn test_visual_line_change_deeply_nested_indentation() {
        // Lines at three indent levels receive the correct three-level prefix.
        let mut editor = create_syntax_editor(
            "fn foo() {\n    if true {\n        let x = 1;\n    }\n}\n",
            "main.rs",
        );
        editor.cursor = Cursor::new(2, 8);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(
            editor.buffer.to_string(),
            "fn foo() {\n    if true {\n        \n    }\n}\n"
        );
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor, Cursor::new(2, 8));
    }

    #[test]
    fn test_visual_line_change_escape_cleans_up_auto_indent() {
        // Pressing Escape after `Vc` without typing anything removes the
        // trailing-whitespace-only indent prefix.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Esc);

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n\n}\n");
        assert!(editor.mode.is_normal());
    }

    #[test]
    fn test_visual_line_change_multiple_lines_indents_to_first_line_context() {
        // `V` selecting two lines then `c` uses the context above the deleted
        // range for the auto-indent level.
        let mut editor =
            create_syntax_editor("fn foo() {\n    let a = 1;\n    let b = 2;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n    \n}\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_visual_line_change_last_line_no_trailing_newline_gets_indent() {
        // `Vc` on the last line when there is no trailing newline should still
        // insert a blank line with the correct indentation prefix.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        editor.handle_key(Key::Char('V'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n    \n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor, Cursor::new(1, 4));
    }

    #[test]
    fn test_visual_character_change_does_not_add_indent() {
        // Characterwise visual `c` deletes the selected text and enters Insert
        // mode at the deletion point without any auto-indent prefix.
        let mut editor = create_syntax_editor("fn foo() {\n    let x = 1;\n}\n", "main.rs");
        editor.cursor = Cursor::new(1, 4);

        // Select "let" (3 chars) with `vll` then change.
        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('l'));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "fn foo() {\n     x = 1;\n}\n");
        assert!(editor.mode.is_insert());
        assert_eq!(editor.cursor, Cursor::new(1, 4));
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
    fn test_space_c_toggles_rust_line_comments_and_dot_repeats() {
        let mut editor = create_syntax_editor("let alpha = 1;\nlet beta = 2;", "sample.rs");

        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(
            editor.buffer.to_string(),
            "// let alpha = 1;\n// let beta = 2;"
        );
        assert_eq!(editor.cursor.line(), 1);
    }

    #[test]
    fn test_space_c_falls_back_to_linewise_block_comments_for_xml() {
        let mut editor = create_syntax_editor("<first>\n<second>", "sample.xml");

        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('c'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char('.'));

        assert_eq!(
            editor.buffer.to_string(),
            "<!-- <first> -->\n<!-- <second> -->"
        );
    }

    #[test]
    fn test_space_shift_c_wraps_counted_lines_once_and_unwraps() {
        let mut editor = create_syntax_editor("let alpha = 1;\nlet beta = 2;", "sample.rs");

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('C'));
        assert_eq!(
            editor.buffer.to_string(),
            "/* let alpha = 1;\nlet beta = 2; */"
        );

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('C'));
        assert_eq!(editor.buffer.to_string(), "let alpha = 1;\nlet beta = 2;");
    }

    #[test]
    fn test_space_shift_c_inserts_opening_block_comment_after_indent() {
        let mut editor = create_syntax_editor("    let alpha = 1;\n    let beta = 2;", "sample.rs");

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('C'));
        assert_eq!(
            editor.buffer.to_string(),
            "    /* let alpha = 1;\n    let beta = 2; */"
        );

        editor.handle_key(Key::Char('2'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('C'));
        assert_eq!(
            editor.buffer.to_string(),
            "    let alpha = 1;\n    let beta = 2;"
        );
    }

    #[test]
    fn test_visual_space_c_comments_full_touched_lines() {
        let mut editor = create_syntax_editor("alpha\nbeta\ngamma", "sample.rs");

        editor.handle_key(Key::Char('v'));
        editor.handle_key(Key::Char('j'));
        editor.handle_key(Key::Char(' '));
        editor.handle_key(Key::Char('c'));

        assert_eq!(editor.buffer.to_string(), "// alpha\n// beta\ngamma");
        assert!(editor.mode.is_normal());
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
    /// Number offsets without an in-line decimal should report informational feedback.
    fn test_ctrl_a_without_number_sets_info_message_kind() {
        let mut editor = create_editor_with_content("alpha");

        editor.handle_key(Key::Ctrl('a'));

        assert_eq!(
            editor.status_message.as_deref(),
            Some("No number on current line")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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
    /// Pressing `*` should start the background search count.
    fn test_star_starts_search_count() {
        let mut editor = create_editor_with_content("test\ntext\ntest\n");

        editor.handle_key(Key::Char('*'));

        // Poll repeatedly to drain background events from the worker thread.
        for _ in 0..100 {
            editor.search_count.poll();
            if editor.search_count.format_message().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let label = editor.search_count.format_message();
        assert!(label.is_some(), "search count label should be set after *");
    }

    #[test]
    fn test_star_searches_next_word_on_same_line_from_whitespace() {
        let mut editor = create_editor_with_content("  word test word");

        editor.handle_key(Key::Char('*'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 12);
    }

    #[test]
    fn test_star_searches_next_candidate_after_same_line_seed_word() {
        let mut editor = create_editor_with_content("(word) test word");

        editor.handle_key(Key::Char('*'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 12);
    }

    #[test]
    /// AsciiDoc buffers should use profile identifiers for `*` word search.
    fn test_star_searches_asciidoc_word_under_cursor() {
        let mut editor = create_syntax_editor("target target", "guide.adoc");

        editor.handle_key(Key::Char('*'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 7);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// AsciiDoc list markers should fall forward to the next same-line word.
    fn test_star_searches_asciidoc_word_after_list_marker() {
        let mut editor = create_syntax_editor("* target target", "guide.adoc");

        editor.handle_key(Key::Char('*'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 9);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// AsciiDoc separator-only suffixes should not borrow words from another line.
    fn test_star_on_asciidoc_line_without_candidate_reports_missing_word() {
        let mut editor = create_syntax_editor("***\ntarget", "guide.adoc");

        editor.cursor = Cursor::new(0, 2);
        editor.handle_key(Key::Char('*'));

        assert_eq!(editor.cursor.line(), 0);
        assert_eq!(editor.cursor.column(), 2);
        assert_eq!(
            editor.status_message.as_deref(),
            Some("No word under cursor")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
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

    #[test]
    /// Code-action edits that apply no text changes should report informational feedback.
    fn test_apply_selected_code_action_noop_edit_sets_info_message_kind() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.rs", "let value = 1;")
            .expect("write main");
        let mut editor = EditorState::new(24);
        editor
            .load_file(tree.path().join("src/main.rs"))
            .expect("load main");

        editor.apply_selected_code_action(
            &LspCodeAction {
                title: "No-op quick fix".to_string(),
                edit: LspWorkspaceEdit {
                    document_edits: vec![],
                },
            },
            editor.active_buffer_id,
            0,
        );

        assert_eq!(
            editor.status_message.as_deref(),
            Some("Code action \"No-op quick fix\" made no changes")
        );
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// Verify that `show_error_message` sets the kind to `Error`.
    fn test_show_error_message_sets_kind() {
        let mut editor = create_editor_with_content("abc");
        editor.show_error_message("something failed");
        assert_eq!(editor.status_message.as_deref(), Some("something failed"));
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Error);
    }

    #[test]
    /// Verify that `show_warning_message` sets the kind to `Warning`.
    fn test_show_warning_message_sets_kind() {
        let mut editor = create_editor_with_content("abc");
        editor.show_warning_message("careful now");
        assert_eq!(editor.status_message.as_deref(), Some("careful now"));
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Warning);
    }

    #[test]
    /// Verify that `show_status_message` resets the kind to `Info`.
    fn test_show_status_message_resets_kind_to_info() {
        let mut editor = create_editor_with_content("abc");
        editor.show_error_message("error first");
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Error);
        editor.show_status_message("info next");
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
    }

    #[test]
    /// Verify that `clear_status_message` resets the kind to `Info`.
    fn test_clear_status_message_resets_kind() {
        let mut editor = create_editor_with_content("abc");
        editor.show_error_message("error");
        editor.clear_status_message();
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Info);
        assert_eq!(editor.status_message, None);
    }

    #[test]
    /// Verify that "No file name" errors set the kind to `Error`.
    fn test_no_file_name_sets_error_kind() {
        let mut editor = create_editor_with_content("abc");
        editor.buffer.insert(0, "x");
        editor.mode = Mode::command_with_text("w");
        editor.handle_key(Key::Char('\n'));
        assert_eq!(editor.status_message.as_deref(), Some("No file name"));
        assert_eq!(editor.status_message_kind(), StatusMessageKind::Error);
    }
}
