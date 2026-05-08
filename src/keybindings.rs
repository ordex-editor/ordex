//! Key bindings configuration
//!
//! This module provides a mapping from key inputs to editor actions.
//! Shared types stay in this file, while defaults, runtime registry behavior,
//! and config parsing live in focused child modules.

use crate::mode::Mode;
use termion::event::Key;

mod defaults;
mod parse;
mod registry;

pub(crate) use parse::{
    parse_action, parse_key_input, parse_key_sequence, parse_mode_context, parse_operator_binding,
};
pub(crate) use registry::KeyBindings;

/// Operator-pending targets that can be rebound in `[keymap.operator]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OperatorBinding {
    WordForward,
    WordForwardBig,
    WordEnd,
    WordEndBig,
    WordBackward,
    WordBackwardBig,
    ParagraphForward,
    ParagraphBackward,
    FindForward,
    FindBackward,
    TillForward,
    TillBackward,
    MatchDelimiter,
    TextObjectInner,
    TextObjectAround,
}

/// Actions that can be triggered by key bindings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Action {
    // Navigation actions
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveDownFirstNonBlank,
    MoveWordForward,
    MoveBigWordForward,
    MoveWordBackward,
    MoveBigWordBackward,
    MoveWordEnd,
    MoveBigWordEnd,
    MoveWordEndBackward,
    MoveBigWordEndBackward,
    MoveParagraphForward,
    MoveParagraphBackward,
    MoveLineStart,
    MoveLineEnd,
    MovePastLineEnd,
    MoveFirstNonBlank,
    MoveToFirstLine,
    MoveToLastLine,
    AlignViewportTop,
    AlignViewportCenter,
    AlignViewportBottom,
    ScrollLineUp,
    ScrollLineDown,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    FindForward,
    FindBackward,
    TillForward,
    TillBackward,
    RepeatFindForward,
    RepeatFindBackward,
    RepeatLastChange,
    JumpOlder,
    JumpNewer,
    MatchBracket,
    GotoDefinition,
    GotoReferences,
    GotoFileUnderCursor,
    GotoFileUnderCursorAtPosition,
    GotoAlternateFile,
    GotoLastModification,
    ShowHover,
    OpenCodeActions,
    OpenDiagnosticsPicker,
    NextDiagnostic,
    PrevDiagnostic,
    PromptRenameSymbol,
    BeginMacroRecord,
    BeginMacroPlayback,

    // Mode switching
    EnterInsertMode,
    EnterVisualMode,
    EnterVisualLineMode,
    SwapVisualAnchor,
    RecreateLastSelection,
    InsertAfterCursor,
    OpenLineBelow,
    OpenLineAbove,
    EnterCommandMode,
    EnterSearchMode,
    OpenBufferSwitcher,
    OpenFilePicker,
    ExitToNormalMode,
    SearchNext,
    SearchPrevious,
    Undo,
    Redo,
    SaveCurrentFile,
    SaveCurrentFileAndQuit,
    UpdateCurrentFileAndQuit,
    RequestFullRedraw,

    // Editing actions
    ToggleCaseAtCursor,
    DeleteToLineEnd,
    ChangeToLineEnd,
    IncrementNextNumber,
    DecrementNextNumber,
    JoinLines,
    BeginReplaceChar,
    SearchWordUnderCursor,
    DeleteCharBackward,
    DeleteCharForward,
    CompletionSelectUp,
    CompletionSelectDown,
    DeleteCharAtCursor,
    DeleteWordBackward,
    DeleteToLineStart,
    InsertNewline,
    DeleteSelection,
    IndentSelection,
    ChangeSelection,
    YankSelection,
    YankCurrentLine,
    PasteAfterCursor,
    PasteBeforeCursor,
    BeginDeleteOperator,
    BeginChangeOperator,
    BeginYankOperator,
    BeginIndentOperator,
    IndentCurrentLine,
    DedentCurrentLine,

    // Command/Search mode actions
    ExecuteCommand,
    CancelCommand,
    PromptHistoryPrev,
    PromptHistoryNext,
    PromptHistoryPrevFull,
    PromptHistoryNextFull,
    DeleteInputChar,
    DeleteInputCharForward,
    DeleteInputWordBackward,
    DeleteInputToStart,
    DeleteInputToEnd,
    MoveInputStart,
    MoveInputEnd,
    MoveInputLeft,
    MoveInputRight,
    MoveInputWordLeft,
    MoveInputWordRight,
}

impl Action {
    /// Return a short human-readable label for UI surfaces.
    pub(crate) fn label(self) -> &'static str {
        match self {
            // Navigation actions.
            Self::MoveLeft => "Move left",
            Self::MoveRight => "Move right",
            Self::MoveUp => "Move up",
            Self::MoveDown => "Move down",
            Self::MoveDownFirstNonBlank => "Move down first non-blank",
            Self::MoveWordForward => "Move word forward",
            Self::MoveBigWordForward => "Move WORD forward",
            Self::MoveWordBackward => "Move word backward",
            Self::MoveBigWordBackward => "Move WORD backward",
            Self::MoveWordEnd => "Move word end",
            Self::MoveBigWordEnd => "Move WORD end",
            Self::MoveWordEndBackward => "Move word end backward",
            Self::MoveBigWordEndBackward => "Move WORD end backward",
            Self::MoveParagraphForward => "Move paragraph forward",
            Self::MoveParagraphBackward => "Move paragraph backward",
            Self::MoveLineStart => "Move line start",
            Self::MoveLineEnd => "Move line end",
            Self::MovePastLineEnd => "Move past line end",
            Self::MoveFirstNonBlank => "Move first non-blank",
            Self::MoveToFirstLine => "Move to first line",
            Self::MoveToLastLine => "Move to last line",
            Self::AlignViewportTop => "Align viewport top",
            Self::AlignViewportCenter => "Align viewport center",
            Self::AlignViewportBottom => "Align viewport bottom",
            Self::ScrollLineUp => "Scroll line up",
            Self::ScrollLineDown => "Scroll line down",
            Self::PageUp => "Page up",
            Self::PageDown => "Page down",
            Self::HalfPageUp => "Half-page up",
            Self::HalfPageDown => "Half-page down",
            Self::FindForward => "Find forward",
            Self::FindBackward => "Find backward",
            Self::TillForward => "Till forward",
            Self::TillBackward => "Till backward",
            Self::RepeatFindForward => "Repeat find forward",
            Self::RepeatFindBackward => "Repeat find backward",
            Self::RepeatLastChange => "Repeat last change",
            Self::JumpOlder => "Jump older",
            Self::JumpNewer => "Jump newer",
            Self::MatchBracket => "Jump to matching delimiter",
            Self::GotoDefinition => "Go to definition",
            Self::GotoReferences => "Go to references",
            Self::GotoFileUnderCursor => "Go to file under cursor",
            Self::GotoFileUnderCursorAtPosition => "Go to file under cursor at position",
            Self::GotoAlternateFile => "Go to alternate file",
            Self::GotoLastModification => "Go to last modification",
            Self::ShowHover => "Show hover",
            Self::OpenCodeActions => "Open code actions",
            Self::OpenDiagnosticsPicker => "Open diagnostics",
            Self::NextDiagnostic => "Next diagnostic",
            Self::PrevDiagnostic => "Previous diagnostic",
            Self::PromptRenameSymbol => "Rename symbol",
            Self::BeginMacroRecord => "Record macro",
            Self::BeginMacroPlayback => "Replay macro",

            // Mode and file actions.
            Self::EnterInsertMode => "Enter insert mode",
            Self::EnterVisualMode => "Enter visual mode",
            Self::EnterVisualLineMode => "Enter visual line mode",
            Self::SwapVisualAnchor => "Swap visual selection end",
            Self::RecreateLastSelection => "Recreate last selection",
            Self::InsertAfterCursor => "Insert after cursor",
            Self::OpenLineBelow => "Open line below",
            Self::OpenLineAbove => "Open line above",
            Self::EnterCommandMode => "Enter command mode",
            Self::EnterSearchMode => "Enter search mode",
            Self::OpenBufferSwitcher => "Open buffer switcher",
            Self::OpenFilePicker => "Open file picker",
            Self::ExitToNormalMode => "Exit to normal mode",
            Self::SearchNext => "Search next",
            Self::SearchPrevious => "Search previous",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::SaveCurrentFile => "Save current file",
            Self::SaveCurrentFileAndQuit => "Save current file and quit",
            Self::UpdateCurrentFileAndQuit => "Update current file and quit",
            Self::RequestFullRedraw => "Redraw screen",

            // Editing actions.
            Self::ToggleCaseAtCursor => "Toggle case at cursor",
            Self::DeleteToLineEnd => "Delete to line end",
            Self::ChangeToLineEnd => "Change to line end",
            Self::IncrementNextNumber => "Increment next number",
            Self::DecrementNextNumber => "Decrement next number",
            Self::JoinLines => "Join lines",
            Self::BeginReplaceChar => "Replace char",
            Self::SearchWordUnderCursor => "Search word under cursor",
            Self::DeleteCharBackward => "Delete char backward",
            Self::DeleteCharForward => "Delete char forward",
            Self::CompletionSelectUp => "Select completion up",
            Self::CompletionSelectDown => "Select completion down",
            Self::DeleteCharAtCursor => "Delete char at cursor",
            Self::DeleteWordBackward => "Delete word backward",
            Self::DeleteToLineStart => "Delete to line start",
            Self::InsertNewline => "Insert newline",
            Self::DeleteSelection => "Delete selection",
            Self::IndentSelection => "Indent selection",
            Self::ChangeSelection => "Change selection",
            Self::YankSelection => "Yank selection",
            Self::YankCurrentLine => "Yank current line",
            Self::PasteAfterCursor => "Paste after cursor",
            Self::PasteBeforeCursor => "Paste before cursor",
            Self::BeginDeleteOperator => "Delete",
            Self::BeginChangeOperator => "Change",
            Self::BeginYankOperator => "Yank",
            Self::BeginIndentOperator => "Indent",
            Self::IndentCurrentLine => "Indent current line",
            Self::DedentCurrentLine => "Dedent current line",

            // Command and search input actions.
            Self::ExecuteCommand => "Execute command",
            Self::CancelCommand => "Cancel command",
            Self::PromptHistoryPrev => "Previous prompt history",
            Self::PromptHistoryNext => "Next prompt history",
            Self::PromptHistoryPrevFull => "Previous full prompt history",
            Self::PromptHistoryNextFull => "Next full prompt history",
            Self::DeleteInputChar => "Delete input char",
            Self::DeleteInputCharForward => "Delete input char forward",
            Self::DeleteInputWordBackward => "Delete input word backward",
            Self::DeleteInputToStart => "Delete input to start",
            Self::DeleteInputToEnd => "Delete input to end",
            Self::MoveInputStart => "Move input start",
            Self::MoveInputEnd => "Move input end",
            Self::MoveInputLeft => "Move input left",
            Self::MoveInputRight => "Move input right",
            Self::MoveInputWordLeft => "Move input word left",
            Self::MoveInputWordRight => "Move input word right",
        }
    }
}

/// Result of matching a typed key sequence against configured multi-key bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SequenceMatch {
    /// Sequence fully matches a binding and should execute the action now.
    Exact(ActionBinding),
    /// Sequence is a valid prefix; wait for additional keys.
    Prefix,
    /// Sequence doesn't match any configured multi-key binding.
    NoMatch,
}

/// Stores either one action without allocation or a heap-backed multi-action sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionBinding {
    Single(Action),
    Multiple(Vec<Action>),
}

impl ActionBinding {
    /// Create a binding that stores exactly one action without allocating.
    pub(crate) fn single(action: Action) -> Self {
        Self::Single(action)
    }

    /// Build the most compact binding representation for the provided actions.
    pub(crate) fn from_actions(mut actions: Vec<Action>) -> Option<Self> {
        match actions.len() {
            0 => None,
            1 => actions.pop().map(Self::Single),
            _ => Some(Self::Multiple(actions)),
        }
    }

    #[cfg(test)]
    pub(crate) fn as_slice(&self) -> &[Action] {
        match self {
            Self::Single(action) => std::slice::from_ref(action),
            Self::Multiple(actions) => actions.as_slice(),
        }
    }

    /// Format this binding as one human-readable label.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Single(action) => action.label().to_string(),
            Self::Multiple(actions) => {
                // Multi-action bindings run left-to-right, so mirror that order.
                actions
                    .iter()
                    .map(|action| action.label())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            }
        }
    }
}

/// One discoverable sequence that continues from the currently typed prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequenceContinuation {
    pub(crate) remaining_keys: Vec<KeyInput>,
    pub(crate) actions: ActionBinding,
}

impl SequenceContinuation {
    /// Return the human-readable suffix that completes this sequence.
    pub(crate) fn keys_label(&self) -> String {
        KeyInput::sequence_label(&self.remaining_keys)
    }

    /// Return one human-readable label for this continuation's action payload.
    pub(crate) fn action_label(&self) -> String {
        self.actions.label()
    }
}

/// Wrapper for Key that implements Hash (termion's Key doesn't implement Hash)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum KeyInput {
    Char(char),
    Ctrl(char),
    Alt(char),
    Unsupported,
    Backspace,
    Escape,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    ShiftUp,
    ShiftDown,
    ShiftLeft,
    ShiftRight,
    AltUp,
    AltDown,
    AltLeft,
    AltRight,
    CtrlUp,
    CtrlDown,
    CtrlLeft,
    CtrlRight,
    Home,
    CtrlHome,
    End,
    CtrlEnd,
    PageUp,
    PageDown,
    Delete,
    Insert,
    F(u8),
}

impl From<Key> for KeyInput {
    /// Convert a termion key into the hashable keybinding representation.
    fn from(key: Key) -> Self {
        match key {
            Key::Char(c) => KeyInput::Char(c),
            Key::Ctrl(c) => KeyInput::Ctrl(c),
            Key::Alt(c) => KeyInput::Alt(c),
            Key::Backspace => KeyInput::Backspace,
            Key::Esc => KeyInput::Escape,
            Key::BackTab => KeyInput::BackTab,
            Key::Up => KeyInput::Up,
            Key::ShiftUp => KeyInput::ShiftUp,
            Key::AltUp => KeyInput::AltUp,
            Key::CtrlUp => KeyInput::CtrlUp,
            Key::Down => KeyInput::Down,
            Key::ShiftDown => KeyInput::ShiftDown,
            Key::AltDown => KeyInput::AltDown,
            Key::CtrlDown => KeyInput::CtrlDown,
            Key::Left => KeyInput::Left,
            Key::ShiftLeft => KeyInput::ShiftLeft,
            Key::AltLeft => KeyInput::AltLeft,
            Key::CtrlLeft => KeyInput::CtrlLeft,
            Key::Right => KeyInput::Right,
            Key::ShiftRight => KeyInput::ShiftRight,
            Key::AltRight => KeyInput::AltRight,
            Key::CtrlRight => KeyInput::CtrlRight,
            Key::Home => KeyInput::Home,
            Key::CtrlHome => KeyInput::CtrlHome,
            Key::End => KeyInput::End,
            Key::CtrlEnd => KeyInput::CtrlEnd,
            Key::PageUp => KeyInput::PageUp,
            Key::PageDown => KeyInput::PageDown,
            Key::Delete => KeyInput::Delete,
            Key::Insert => KeyInput::Insert,
            Key::F(n) => KeyInput::F(n),
            _ => KeyInput::Unsupported,
        }
    }
}

impl KeyInput {
    /// Format one key input for status lines, prompts, and discovery popups.
    pub(crate) fn label(&self) -> String {
        match self {
            // Character-like inputs keep the typed glyph visible when possible.
            Self::Char('\t') => "Tab".to_string(),
            Self::Char(c) => c.to_string(),
            Self::Ctrl('i') => "Tab".to_string(),
            Self::Ctrl(c) => format!("^{}", c),
            Self::Alt(c) => format!("M-{}", c),
            Self::Unsupported => "?".to_string(),
            Self::Backspace => "BS".to_string(),
            Self::Escape => "Esc".to_string(),
            Self::BackTab => "S-Tab".to_string(),
            Self::Up => "Up".to_string(),
            Self::Down => "Down".to_string(),
            Self::Left => "Left".to_string(),
            Self::Right => "Right".to_string(),

            // Modified navigation keys use the same compact prefixes as the UI.
            Self::ShiftUp => "S-Up".to_string(),
            Self::ShiftDown => "S-Down".to_string(),
            Self::ShiftLeft => "S-Left".to_string(),
            Self::ShiftRight => "S-Right".to_string(),
            Self::AltUp => "M-Up".to_string(),
            Self::AltDown => "M-Down".to_string(),
            Self::AltLeft => "M-Left".to_string(),
            Self::AltRight => "M-Right".to_string(),
            Self::CtrlUp => "C-Up".to_string(),
            Self::CtrlDown => "C-Down".to_string(),
            Self::CtrlLeft => "C-Left".to_string(),
            Self::CtrlRight => "C-Right".to_string(),
            Self::Home => "Home".to_string(),
            Self::CtrlHome => "C-Home".to_string(),
            Self::End => "End".to_string(),
            Self::CtrlEnd => "C-End".to_string(),
            Self::PageUp => "PgUp".to_string(),
            Self::PageDown => "PgDn".to_string(),
            Self::Delete => "Del".to_string(),
            Self::Insert => "Ins".to_string(),
            Self::F(n) => format!("F{}", n),
        }
    }

    /// Format a full key sequence by concatenating the per-key display labels.
    pub(crate) fn sequence_label(keys: &[Self]) -> String {
        keys.iter().map(Self::label).collect()
    }
}

/// Mode context for key binding lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ModeContext {
    Normal,
    Visual,
    Insert,
    Command,
    Search,
}

impl From<&Mode> for ModeContext {
    /// Convert an editor mode into the keybinding lookup context.
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Normal => ModeContext::Normal,
            Mode::Visual(_) => ModeContext::Visual,
            Mode::Insert => ModeContext::Insert,
            Mode::Command(_) => ModeContext::Command,
            Mode::Search(_) => ModeContext::Search,
            Mode::BufferSwitch(_)
            | Mode::FilePicker(_)
            | Mode::LocationPicker(_)
            | Mode::DiagnosticPicker(_)
            | Mode::CodeActionPicker(_) => ModeContext::Command,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_mode_hjkl() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('h'), &mode),
            Some(Action::MoveLeft)
        );
        assert_eq!(
            bindings.get_action(Key::Char('j'), &mode),
            Some(Action::MoveDown)
        );
        assert_eq!(
            bindings.get_action(Key::Char('k'), &mode),
            Some(Action::MoveUp)
        );
        assert_eq!(
            bindings.get_action(Key::Char('l'), &mode),
            Some(Action::MoveRight)
        );
    }

    #[test]
    fn test_normal_mode_word_navigation() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('w'), &mode),
            Some(Action::MoveWordForward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('b'), &mode),
            Some(Action::MoveWordBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('W'), &mode),
            Some(Action::MoveBigWordForward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('B'), &mode),
            Some(Action::MoveBigWordBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('E'), &mode),
            Some(Action::MoveBigWordEnd)
        );
        assert_eq!(
            bindings.get_action(Key::Char('_'), &mode),
            Some(Action::MoveDownFirstNonBlank)
        );
        assert_eq!(
            bindings.get_action(Key::Char('d'), &mode),
            Some(Action::BeginDeleteOperator)
        );
        assert_eq!(
            bindings.get_action(Key::Char('c'), &mode),
            Some(Action::BeginChangeOperator)
        );
        assert_eq!(
            bindings.get_action(Key::Char('y'), &mode),
            Some(Action::BeginYankOperator)
        );
        assert_eq!(
            bindings.get_action(Key::Char('='), &mode),
            Some(Action::BeginIndentOperator)
        );
        assert_eq!(
            bindings.get_action(Key::Char('{'), &mode),
            Some(Action::MoveParagraphBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('}'), &mode),
            Some(Action::MoveParagraphForward)
        );
    }

    #[test]
    fn test_normal_mode_page_navigation() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Ctrl('f'), &mode),
            Some(Action::PageDown)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('b'), &mode),
            Some(Action::PageUp)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('d'), &mode),
            Some(Action::HalfPageDown)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('u'), &mode),
            Some(Action::HalfPageUp)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('y'), &mode),
            Some(Action::ScrollLineUp)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('e'), &mode),
            Some(Action::ScrollLineDown)
        );
    }

    #[test]
    fn test_normal_mode_enter_insert() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('i'), &mode),
            Some(Action::EnterInsertMode)
        );
        assert_eq!(
            bindings.get_action(Key::Char('a'), &mode),
            Some(Action::InsertAfterCursor)
        );
        assert_eq!(
            bindings.get_action(Key::Char('p'), &mode),
            Some(Action::PasteAfterCursor)
        );
        assert_eq!(
            bindings.get_action(Key::Char('P'), &mode),
            Some(Action::PasteBeforeCursor)
        );
        assert_eq!(
            bindings
                .get_binding(Key::Char('I'), &mode)
                .unwrap()
                .as_slice(),
            &[Action::MoveFirstNonBlank, Action::EnterInsertMode]
        );
        assert_eq!(
            bindings
                .get_binding(Key::Char('A'), &mode)
                .unwrap()
                .as_slice(),
            &[Action::MoveLineEnd, Action::InsertAfterCursor]
        );
        assert_eq!(
            bindings.get_action(Key::Char('v'), &mode),
            Some(Action::EnterVisualMode)
        );
        assert_eq!(
            bindings.get_action(Key::Char('V'), &mode),
            Some(Action::EnterVisualLineMode)
        );
        assert_eq!(
            bindings.get_action(Key::Char('o'), &mode),
            Some(Action::OpenLineBelow)
        );
        assert_eq!(
            bindings.get_action(Key::Char('O'), &mode),
            Some(Action::OpenLineAbove)
        );
    }

    #[test]
    fn test_normal_mode_delete_char_at_cursor() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('x'), &mode),
            Some(Action::DeleteCharAtCursor)
        );
        assert_eq!(
            bindings.get_action(Key::Char('~'), &mode),
            Some(Action::ToggleCaseAtCursor)
        );
        assert_eq!(
            bindings.get_action(Key::Char('D'), &mode),
            Some(Action::DeleteToLineEnd)
        );
        assert_eq!(
            bindings.get_action(Key::Char('C'), &mode),
            Some(Action::ChangeToLineEnd)
        );
        assert_eq!(
            bindings.get_action(Key::Char('J'), &mode),
            Some(Action::JoinLines)
        );
        assert_eq!(
            bindings.get_action(Key::Char('r'), &mode),
            Some(Action::BeginReplaceChar)
        );
        assert_eq!(
            bindings.get_action(Key::Char('*'), &mode),
            Some(Action::SearchWordUnderCursor)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('a'), &mode),
            Some(Action::IncrementNextNumber)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('x'), &mode),
            Some(Action::DecrementNextNumber)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('l'), &mode),
            Some(Action::RequestFullRedraw)
        );
        assert_eq!(
            bindings.get_action(Key::Char('u'), &mode),
            Some(Action::Undo)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('r'), &mode),
            Some(Action::Redo)
        );
        assert_eq!(
            bindings.get_action(Key::Char('.'), &mode),
            Some(Action::RepeatLastChange)
        );
    }

    #[test]
    fn test_insert_mode_exit() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;

        assert_eq!(
            bindings.get_action(Key::Esc, &mode),
            Some(Action::ExitToNormalMode)
        );
    }

    #[test]
    fn test_insert_mode_special_keys() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;

        assert_eq!(
            bindings.get_action(Key::Char('\n'), &mode),
            Some(Action::InsertNewline)
        );
        assert_eq!(
            bindings.get_action(Key::Backspace, &mode),
            Some(Action::DeleteCharBackward)
        );
    }

    #[test]
    fn test_insert_mode_arrow_keys() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;

        assert_eq!(
            bindings.get_action(Key::Left, &mode),
            Some(Action::MoveLeft)
        );
        assert_eq!(
            bindings.get_action(Key::Right, &mode),
            Some(Action::MoveRight)
        );
        assert_eq!(
            bindings.get_action(Key::Up, &mode),
            Some(Action::CompletionSelectUp)
        );
        assert_eq!(
            bindings.get_action(Key::Down, &mode),
            Some(Action::CompletionSelectDown)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('p'), &mode),
            Some(Action::CompletionSelectUp)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('n'), &mode),
            Some(Action::CompletionSelectDown)
        );
    }

    #[test]
    fn test_insert_mode_home_end() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;

        assert_eq!(
            bindings.get_action(Key::Home, &mode),
            Some(Action::MoveLineStart)
        );
        assert_eq!(
            bindings.get_action(Key::End, &mode),
            Some(Action::MovePastLineEnd)
        );
    }

    #[test]
    fn test_insert_mode_delete_keys() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;

        assert_eq!(
            bindings.get_action(Key::Delete, &mode),
            Some(Action::DeleteCharForward)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('w'), &mode),
            Some(Action::DeleteWordBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('h'), &mode),
            Some(Action::DeleteCharBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('u'), &mode),
            Some(Action::DeleteToLineStart)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('t'), &mode),
            Some(Action::IndentCurrentLine)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('d'), &mode),
            Some(Action::DedentCurrentLine)
        );
    }

    #[test]
    fn test_insert_mode_regular_char_not_in_bindings() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;

        // Regular characters should return None (handled by is_insertable_char)
        assert_eq!(bindings.get_action(Key::Char('a'), &mode), None);
        assert_eq!(KeyBindings::is_insertable_char(Key::Char('a')), Some('a'));
    }

    #[test]
    fn test_command_mode() {
        let bindings = KeyBindings::new();
        let mode = Mode::command_empty();

        assert_eq!(
            bindings.get_action(Key::Char('\n'), &mode),
            Some(Action::ExecuteCommand)
        );
        assert_eq!(
            bindings.get_action(Key::Esc, &mode),
            Some(Action::CancelCommand)
        );
        assert_eq!(
            bindings.get_action(Key::Up, &mode),
            Some(Action::PromptHistoryPrev)
        );
        assert_eq!(
            bindings.get_action(Key::Down, &mode),
            Some(Action::PromptHistoryNext)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('p'), &mode),
            Some(Action::PromptHistoryPrevFull)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('n'), &mode),
            Some(Action::PromptHistoryNextFull)
        );
        assert_eq!(
            bindings.get_action(Key::Backspace, &mode),
            Some(Action::DeleteInputChar)
        );
        assert_eq!(
            bindings.get_action(Key::Delete, &mode),
            Some(Action::DeleteInputCharForward)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('w'), &mode),
            Some(Action::DeleteInputWordBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('u'), &mode),
            Some(Action::DeleteInputToStart)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('k'), &mode),
            Some(Action::DeleteInputToEnd)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('a'), &mode),
            Some(Action::MoveInputStart)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('e'), &mode),
            Some(Action::MoveInputEnd)
        );
        assert_eq!(
            bindings.get_action(Key::Left, &mode),
            Some(Action::MoveInputLeft)
        );
        assert_eq!(
            bindings.get_action(Key::Right, &mode),
            Some(Action::MoveInputRight)
        );
        assert_eq!(
            bindings.get_action(Key::Alt('b'), &mode),
            Some(Action::MoveInputWordLeft)
        );
        assert_eq!(
            bindings.get_action(Key::Alt('f'), &mode),
            Some(Action::MoveInputWordRight)
        );
    }

    #[test]
    fn test_search_mode() {
        let bindings = KeyBindings::new();
        let mode = Mode::search_empty();

        assert_eq!(
            bindings.get_action(Key::Char('\n'), &mode),
            Some(Action::ExecuteCommand)
        );
        assert_eq!(
            bindings.get_action(Key::Esc, &mode),
            Some(Action::CancelCommand)
        );
        assert_eq!(
            bindings.get_action(Key::Up, &mode),
            Some(Action::PromptHistoryPrev)
        );
        assert_eq!(
            bindings.get_action(Key::Down, &mode),
            Some(Action::PromptHistoryNext)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('p'), &mode),
            Some(Action::PromptHistoryPrevFull)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('n'), &mode),
            Some(Action::PromptHistoryNextFull)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('b'), &mode),
            Some(Action::MoveInputLeft)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('f'), &mode),
            Some(Action::MoveInputRight)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('d'), &mode),
            Some(Action::DeleteInputCharForward)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('h'), &mode),
            Some(Action::DeleteInputChar)
        );
    }

    #[test]
    fn test_normal_mode_search_repeat() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('n'), &mode),
            Some(Action::SearchNext)
        );
        assert_eq!(
            bindings.get_action(Key::Char('N'), &mode),
            Some(Action::SearchPrevious)
        );
    }

    #[test]
    fn test_visual_mode_bindings() {
        let bindings = KeyBindings::new();
        let mode = Mode::visual_character();

        assert_eq!(
            bindings.get_action(Key::Char('h'), &mode),
            Some(Action::MoveLeft)
        );
        assert_eq!(
            bindings.get_action(Key::Char('d'), &mode),
            Some(Action::DeleteSelection)
        );
        assert_eq!(
            bindings.get_action(Key::Char('~'), &mode),
            Some(Action::ToggleCaseAtCursor)
        );
        assert_eq!(
            bindings.get_action(Key::Char('y'), &mode),
            Some(Action::YankSelection)
        );
        assert_eq!(
            bindings.get_action(Key::Char('c'), &mode),
            Some(Action::ChangeSelection)
        );
        assert_eq!(
            bindings.get_action(Key::Char('='), &mode),
            Some(Action::IndentSelection)
        );
        assert_eq!(
            bindings.get_action(Key::Char('v'), &mode),
            Some(Action::EnterVisualMode)
        );
        assert_eq!(
            bindings.get_action(Key::Char('o'), &mode),
            Some(Action::SwapVisualAnchor)
        );
        assert_eq!(
            bindings.get_action(Key::Esc, &mode),
            Some(Action::ExitToNormalMode)
        );
        assert_eq!(
            bindings.get_action(Key::Char('%'), &mode),
            Some(Action::MatchBracket)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('y'), &mode),
            Some(Action::ScrollLineUp)
        );
        assert_eq!(
            bindings.get_action(Key::Ctrl('e'), &mode),
            Some(Action::ScrollLineDown)
        );
    }

    #[test]
    fn test_normal_mode_find_and_till_navigation() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('f'), &mode),
            Some(Action::FindForward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('F'), &mode),
            Some(Action::FindBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('t'), &mode),
            Some(Action::TillForward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('T'), &mode),
            Some(Action::TillBackward)
        );
    }

    #[test]
    fn test_normal_mode_find_repeat_keys() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char(';'), &mode),
            Some(Action::RepeatFindForward)
        );
        assert_eq!(
            bindings.get_action(Key::Char(','), &mode),
            Some(Action::RepeatFindBackward)
        );
        assert_eq!(
            bindings.get_action(Key::Char('%'), &mode),
            Some(Action::MatchBracket)
        );
    }

    #[test]
    fn test_unbound_key_returns_none() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        // 'z' is not bound in normal mode
        assert_eq!(bindings.get_action(Key::Char('z'), &mode), None);
    }

    #[test]
    fn test_sequence_g_prefix() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Prefix
        );
    }

    #[test]
    fn test_sequence_continuations_for_g_prefix() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let continuations = bindings.continuations_for_prefix(&mode, &[KeyInput::Char('g')]);

        let labels: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::keys_label)
            .collect();
        let actions: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::action_label)
            .collect();

        assert_eq!(
            labels,
            vec!["g", "$", "0", "e", "E", "v", "d", "r", "f", "F", "a", "."]
        );
        assert_eq!(
            actions,
            vec![
                "Move to first line",
                "Move line end",
                "Move line start",
                "Move word end backward",
                "Move WORD end backward",
                "Recreate last selection",
                "Go to definition",
                "Go to references",
                "Go to file under cursor",
                "Go to file under cursor at position",
                "Go to alternate file",
                "Go to last modification",
            ]
        );
    }

    #[test]
    fn test_sequence_continuations_for_space_prefix() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let continuations = bindings.continuations_for_prefix(&mode, &[KeyInput::Char(' ')]);

        let labels: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::keys_label)
            .collect();
        let actions: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::action_label)
            .collect();

        assert_eq!(labels, vec!["a", "d", "w", "q", "b", "f", "r"]);
        assert_eq!(
            actions,
            vec![
                "Open code actions",
                "Open diagnostics",
                "Save current file",
                "Update current file and quit",
                "Open buffer switcher",
                "Open file picker",
                "Rename symbol",
            ]
        );
    }

    #[test]
    fn test_sequence_continuations_include_configured_multi_action_labels() {
        let mut bindings = KeyBindings::new();
        let mode = Mode::Normal;
        bindings.set_sequence_binding_actions(
            ModeContext::Normal,
            vec![KeyInput::Char('z'), KeyInput::Char('u')],
            vec![Action::MoveDown, Action::MoveRight],
        );
        bindings.set_sequence_binding_action_binding(
            ModeContext::Normal,
            vec![KeyInput::Char('z'), KeyInput::Char('q')],
            ActionBinding::single(Action::SaveCurrentFile),
        );

        let continuations = bindings.continuations_for_prefix(&mode, &[KeyInput::Char('z')]);
        let labels: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::keys_label)
            .collect();
        let actions: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::action_label)
            .collect();

        assert_eq!(labels, vec!["t", "z", "b", "u", "q"]);
        assert_eq!(
            actions,
            vec![
                "Align viewport top",
                "Align viewport center",
                "Align viewport bottom",
                "Move down -> Move right",
                "Save current file",
            ]
        );
    }

    #[test]
    fn test_sequence_gg_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('g')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::MoveToFirstLine))
        );
    }

    #[test]
    fn test_sequence_g_dollar_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('$')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::MoveLineEnd))
        );
    }

    #[test]
    fn test_sequence_g_zero_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('0')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::MoveLineStart))
        );
    }

    #[test]
    fn test_sequence_g_v_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('v')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::RecreateLastSelection))
        );
    }

    #[test]
    fn test_sequence_g_i_no_match() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('i')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::NoMatch
        );
    }

    #[test]
    fn test_sequence_z_prefix() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('z')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Prefix
        );
    }

    #[test]
    fn test_sequence_continuations_for_z_prefix() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let continuations = bindings.continuations_for_prefix(&mode, &[KeyInput::Char('z')]);

        let labels: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::keys_label)
            .collect();
        let actions: Vec<String> = continuations
            .iter()
            .map(SequenceContinuation::action_label)
            .collect();

        assert_eq!(labels, vec!["t", "z", "b"]);
        assert_eq!(
            actions,
            vec![
                "Align viewport top",
                "Align viewport center",
                "Align viewport bottom",
            ]
        );
    }

    #[test]
    fn test_sequence_zt_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('z'), KeyInput::Char('t')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::AlignViewportTop))
        );
    }

    #[test]
    fn test_sequence_zz_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('z'), KeyInput::Char('z')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::AlignViewportCenter))
        );
    }

    #[test]
    fn test_sequence_zb_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('z'), KeyInput::Char('b')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::AlignViewportBottom))
        );
    }

    #[test]
    fn test_visual_mode_sequence_gg_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::visual_character();
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('g')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::MoveToFirstLine))
        );
    }

    #[test]
    fn test_sequence_does_not_match_in_insert_mode() {
        let bindings = KeyBindings::new();
        let mode = Mode::Insert;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('g')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::NoMatch
        );
    }

    #[test]
    fn test_sequence_space_w_prefix() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' ')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Prefix
        );
    }

    #[test]
    fn test_sequence_space_w_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' '), KeyInput::Char('w')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::SaveCurrentFile))
        );
    }

    #[test]
    fn test_sequence_space_a_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' '), KeyInput::Char('a')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::OpenCodeActions))
        );
    }

    #[test]
    fn test_sequence_space_q_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' '), KeyInput::Char('q')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::UpdateCurrentFileAndQuit))
        );
    }

    #[test]
    fn test_sequence_space_r_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' '), KeyInput::Char('r')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::PromptRenameSymbol))
        );
    }

    #[test]
    fn test_parse_key_input_complex_keys() {
        assert_eq!(parse_key_input("ctrl-f"), Some(KeyInput::Ctrl('f')));
        assert_eq!(parse_key_input("alt-b"), Some(KeyInput::Alt('b')));
        assert_eq!(parse_key_input("ctrl-home"), Some(KeyInput::CtrlHome));
        assert_eq!(parse_key_input("ctrl-end"), Some(KeyInput::CtrlEnd));
        assert_eq!(parse_key_input("tab"), Some(KeyInput::Ctrl('i')));
        assert_eq!(parse_key_input("shift-tab"), Some(KeyInput::BackTab));
        assert_eq!(parse_key_input("alt-left"), Some(KeyInput::AltLeft));
        assert_eq!(parse_key_input("ctrl+end"), None);
        assert_eq!(parse_key_input("home"), Some(KeyInput::Home));
        assert_eq!(parse_key_input("delete"), Some(KeyInput::Delete));
        assert_eq!(parse_key_input("space"), Some(KeyInput::Char(' ')));
        assert_eq!(parse_key_input("pageup"), Some(KeyInput::PageUp));
        assert_eq!(parse_key_input("é"), Some(KeyInput::Char('é')));
    }

    #[test]
    fn test_parse_key_sequence_multi_keys() {
        assert_eq!(
            parse_key_sequence("zu"),
            Some(vec![KeyInput::Char('z'), KeyInput::Char('u')])
        );
        assert_eq!(
            parse_key_sequence("ctrl-home"),
            Some(vec![KeyInput::CtrlHome])
        );
        assert_eq!(parse_key_sequence("ctrl+home"), None);
        assert_eq!(parse_key_sequence("ctrl-hom"), None);
    }

    #[test]
    fn test_parse_action_accepts_kebab_case_names() {
        assert_eq!(parse_action("move-down"), Some(Action::MoveDown));
        assert_eq!(
            parse_action("delete-char-at-cursor"),
            Some(Action::DeleteCharAtCursor)
        );
        assert_eq!(
            parse_action("save-current-file-and-quit"),
            Some(Action::SaveCurrentFileAndQuit)
        );
        assert_eq!(
            parse_action("update-current-file-and-quit"),
            Some(Action::UpdateCurrentFileAndQuit)
        );
        assert_eq!(
            parse_action("enter-visual-mode"),
            Some(Action::EnterVisualMode)
        );
        assert_eq!(
            parse_action("swap-visual-anchor"),
            Some(Action::SwapVisualAnchor)
        );
        assert_eq!(
            parse_action("recreate-last-selection"),
            Some(Action::RecreateLastSelection)
        );
        assert_eq!(
            parse_action("change-selection"),
            Some(Action::ChangeSelection)
        );
        assert_eq!(parse_action("yank-selection"), Some(Action::YankSelection));
        assert_eq!(
            parse_action("indent-selection"),
            Some(Action::IndentSelection)
        );
        assert_eq!(
            parse_action("yank-current-line"),
            Some(Action::YankCurrentLine)
        );
        assert_eq!(
            parse_action("paste-after-cursor"),
            Some(Action::PasteAfterCursor)
        );
        assert_eq!(
            parse_action("begin-indent-operator"),
            Some(Action::BeginIndentOperator)
        );
        assert_eq!(
            parse_action("paste-before-cursor"),
            Some(Action::PasteBeforeCursor)
        );
        assert_eq!(
            parse_action("align-viewport-center"),
            Some(Action::AlignViewportCenter)
        );
        assert_eq!(
            parse_action("scroll-line-down"),
            Some(Action::ScrollLineDown)
        );
        assert_eq!(
            parse_action("jump-to-matching-delimiter"),
            Some(Action::MatchBracket)
        );
        assert_eq!(
            parse_action("repeat-last-change"),
            Some(Action::RepeatLastChange)
        );
        assert_eq!(
            parse_action("move-word-end-backward"),
            Some(Action::MoveWordEndBackward)
        );
        assert_eq!(
            parse_action("move-big-word-forward"),
            Some(Action::MoveBigWordForward)
        );
        assert_eq!(
            parse_action("move-big-word-end-backward"),
            Some(Action::MoveBigWordEndBackward)
        );
        assert_eq!(
            parse_action("begin-replace-char"),
            Some(Action::BeginReplaceChar)
        );
        assert_eq!(
            parse_action("request-full-redraw"),
            Some(Action::RequestFullRedraw)
        );
        assert_eq!(parse_action("jump-older"), Some(Action::JumpOlder));
        assert_eq!(parse_action("jump-newer"), Some(Action::JumpNewer));
        assert_eq!(
            parse_action("goto-file-under-cursor"),
            Some(Action::GotoFileUnderCursor)
        );
        assert_eq!(
            parse_action("goto-file-under-cursor-at-position"),
            Some(Action::GotoFileUnderCursorAtPosition)
        );
        assert_eq!(
            parse_action("goto-alternate-file"),
            Some(Action::GotoAlternateFile)
        );
        assert_eq!(
            parse_action("goto-last-modification"),
            Some(Action::GotoLastModification)
        );
        assert_eq!(parse_action("undo"), Some(Action::Undo));
        assert_eq!(parse_action("redo"), Some(Action::Redo));
        assert_eq!(
            parse_action("open-code-actions"),
            Some(Action::OpenCodeActions)
        );
    }

    #[test]
    fn test_parse_action_is_case_sensitive_and_requires_hyphens() {
        assert_eq!(parse_action("MoveDown"), None);
        assert_eq!(parse_action("move_down"), None);
        assert_eq!(parse_action("movedown"), None);
        assert_eq!(parse_action("move-Down"), None);
    }

    #[test]
    fn test_parse_operator_binding_accepts_supported_names() {
        assert_eq!(
            parse_operator_binding("word-forward"),
            Some(OperatorBinding::WordForward)
        );
        assert_eq!(
            parse_operator_binding("big-word-end"),
            Some(OperatorBinding::WordEndBig)
        );
        assert_eq!(
            parse_operator_binding("text-object-around"),
            Some(OperatorBinding::TextObjectAround)
        );
        assert_eq!(
            parse_operator_binding("jump-to-matching-delimiter"),
            Some(OperatorBinding::MatchDelimiter)
        );
    }

    #[test]
    fn test_parse_mode_context_supports_visual() {
        assert_eq!(parse_mode_context("visual"), Some(ModeContext::Visual));
    }

    #[test]
    fn test_runtime_multi_action_binding_returns_all_actions() {
        let mut bindings = KeyBindings::new();
        let mode = Mode::Normal;
        bindings.set_binding_actions(
            ModeContext::Normal,
            KeyInput::Char('z'),
            vec![Action::MoveDown, Action::MoveRight],
        );

        assert_eq!(
            bindings.get_binding(Key::Char('z'), &mode),
            Some(&ActionBinding::Multiple(vec![
                Action::MoveDown,
                Action::MoveRight,
            ]))
        );
        assert_eq!(bindings.get_action(Key::Char('z'), &mode), None);
    }

    #[test]
    fn test_runtime_multi_action_sequence_returns_all_actions() {
        let mut bindings = KeyBindings::new();
        let mode = Mode::Normal;
        bindings.set_sequence_binding_actions(
            ModeContext::Normal,
            vec![KeyInput::Char('z'), KeyInput::Char('u')],
            vec![Action::MoveDown, Action::MoveRight],
        );

        assert_eq!(
            bindings.match_sequence(&mode, &[KeyInput::Char('z'), KeyInput::Char('u')]),
            SequenceMatch::Exact(ActionBinding::Multiple(vec![
                Action::MoveDown,
                Action::MoveRight,
            ]))
        );
    }

    #[test]
    fn test_action_binding_single_exposes_one_action_without_allocation() {
        let binding = ActionBinding::from_actions(vec![Action::MoveDown]).expect("single action");

        assert_eq!(binding, ActionBinding::Single(Action::MoveDown));
        assert_eq!(binding.as_slice(), &[Action::MoveDown]);
    }
}
