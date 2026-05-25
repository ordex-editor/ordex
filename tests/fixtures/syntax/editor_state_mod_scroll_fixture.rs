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
mod auto_insert;
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
