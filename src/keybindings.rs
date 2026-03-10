//! Key bindings configuration
//!
//! This module provides a mapping from key inputs to editor actions.
//! The configuration is read-only during editor session, allowing for
//! future file-based configuration support.

use crate::mode::Mode;
use std::collections::HashMap;
use termion::event::Key;

/// Actions that can be triggered by key bindings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Action {
    // Navigation actions
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordForward,
    MoveWordBackward,
    MoveWordEnd,
    MoveParagraphForward,
    MoveParagraphBackward,
    MoveLineStart,
    MoveLineEnd,
    MovePastLineEnd,
    MoveFirstNonBlank,
    MoveToFirstLine,
    MoveToLastLine,
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

    // Mode switching
    EnterInsertMode,
    EnterVisualMode,
    EnterVisualLineMode,
    InsertAfterCursor,
    OpenLineBelow,
    OpenLineAbove,
    EnterCommandMode,
    EnterSearchMode,
    ExitToNormalMode,
    SearchNext,
    SearchPrevious,
    SaveCurrentFile,
    SaveCurrentFileAndQuit,

    // Insert mode actions (parameterized actions handled specially)
    DeleteCharBackward,
    DeleteCharForward,
    DeleteCharAtCursor,
    DeleteWordBackward,
    DeleteToLineStart,
    InsertNewline,
    DeleteSelection,
    ChangeSelection,
    ChangeInnerWord,
    DeleteInnerWord,
    DeleteAroundParen,

    // Command/Search mode actions
    ExecuteCommand,
    CancelCommand,
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
            Self::MoveWordForward => "Move word forward",
            Self::MoveWordBackward => "Move word backward",
            Self::MoveWordEnd => "Move word end",
            Self::MoveParagraphForward => "Move paragraph forward",
            Self::MoveParagraphBackward => "Move paragraph backward",
            Self::MoveLineStart => "Move line start",
            Self::MoveLineEnd => "Move line end",
            Self::MovePastLineEnd => "Move past line end",
            Self::MoveFirstNonBlank => "Move first non-blank",
            Self::MoveToFirstLine => "Move to first line",
            Self::MoveToLastLine => "Move to last line",
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

            // Mode and file actions.
            Self::EnterInsertMode => "Enter insert mode",
            Self::EnterVisualMode => "Enter visual mode",
            Self::EnterVisualLineMode => "Enter visual line mode",
            Self::InsertAfterCursor => "Insert after cursor",
            Self::OpenLineBelow => "Open line below",
            Self::OpenLineAbove => "Open line above",
            Self::EnterCommandMode => "Enter command mode",
            Self::EnterSearchMode => "Enter search mode",
            Self::ExitToNormalMode => "Exit to normal mode",
            Self::SearchNext => "Search next",
            Self::SearchPrevious => "Search previous",
            Self::SaveCurrentFile => "Save current file",
            Self::SaveCurrentFileAndQuit => "Save current file and quit",

            // Editing actions.
            Self::DeleteCharBackward => "Delete char backward",
            Self::DeleteCharForward => "Delete char forward",
            Self::DeleteCharAtCursor => "Delete char at cursor",
            Self::DeleteWordBackward => "Delete word backward",
            Self::DeleteToLineStart => "Delete to line start",
            Self::InsertNewline => "Insert newline",
            Self::DeleteSelection => "Delete selection",
            Self::ChangeSelection => "Change selection",
            Self::ChangeInnerWord => "Change inner word",
            Self::DeleteInnerWord => "Delete inner word",
            Self::DeleteAroundParen => "Delete around paren",

            // Command and search input actions.
            Self::ExecuteCommand => "Execute command",
            Self::CancelCommand => "Cancel command",
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

/// Internal storage for a configured multi-key binding and its action payload.
/// Multi-key sequence binding.
#[derive(Debug, Clone)]
struct SequenceBinding {
    mode: ModeContext,
    keys: Vec<KeyInput>,
    actions: ActionBinding,
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
            Self::Char(c) => c.to_string(),
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
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Normal => ModeContext::Normal,
            Mode::Visual(_) => ModeContext::Visual,
            Mode::Insert => ModeContext::Insert,
            Mode::Command(_) => ModeContext::Command,
            Mode::Search(_) => ModeContext::Search,
        }
    }
}

/// Key bindings configuration
/// Uses HashMaps to store bindings, making it easy to load from config file later
pub(crate) struct KeyBindings {
    /// Bindings for each mode: (ModeContext, KeyInput) -> actions
    bindings: HashMap<(ModeContext, KeyInput), ActionBinding>,
    /// Sequence bindings for each mode (e.g. "gg").
    sequence_bindings: Vec<SequenceBinding>,
}

impl KeyBindings {
    /// Create default key bindings
    pub(crate) fn new() -> Self {
        let mut bindings = HashMap::new();
        let mut sequence_bindings = Vec::new();

        // Normal mode bindings
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('h'),
            Action::MoveLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('j'),
            Action::MoveDown,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('k'),
            Action::MoveUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('l'),
            Action::MoveRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('w'),
            Action::MoveWordForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('b'),
            Action::MoveWordBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Ctrl('f'),
            Action::PageDown,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Ctrl('b'),
            Action::PageUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Ctrl('d'),
            Action::HalfPageDown,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Ctrl('u'),
            Action::HalfPageUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('i'),
            Action::EnterInsertMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('v'),
            Action::EnterVisualMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('V'),
            Action::EnterVisualLineMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('a'),
            Action::InsertAfterCursor,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('x'),
            Action::DeleteCharAtCursor,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('o'),
            Action::OpenLineBelow,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('O'),
            Action::OpenLineAbove,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char(':'),
            Action::EnterCommandMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('/'),
            Action::EnterSearchMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('n'),
            Action::SearchNext,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('N'),
            Action::SearchPrevious,
        );
        // Line navigation
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('0'),
            Action::MoveLineStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('$'),
            Action::MoveLineEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('^'),
            Action::MoveFirstNonBlank,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('e'),
            Action::MoveWordEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('{'),
            Action::MoveParagraphBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('}'),
            Action::MoveParagraphForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('f'),
            Action::FindForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('F'),
            Action::FindBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('t'),
            Action::TillForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('T'),
            Action::TillBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char(';'),
            Action::RepeatFindForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char(','),
            Action::RepeatFindBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Normal,
            KeyInput::Char('G'),
            Action::MoveToLastLine,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![KeyInput::Char('g'), KeyInput::Char('g')],
            Action::MoveToFirstLine,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![KeyInput::Char('g'), KeyInput::Char('$')],
            Action::MoveLineEnd,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![KeyInput::Char('g'), KeyInput::Char('0')],
            Action::MoveLineStart,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![
                KeyInput::Char('c'),
                KeyInput::Char('i'),
                KeyInput::Char('w'),
            ],
            Action::ChangeInnerWord,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![
                KeyInput::Char('d'),
                KeyInput::Char('i'),
                KeyInput::Char('w'),
            ],
            Action::DeleteInnerWord,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![
                KeyInput::Char('d'),
                KeyInput::Char('a'),
                KeyInput::Char('('),
            ],
            Action::DeleteAroundParen,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![KeyInput::Char(' '), KeyInput::Char('w')],
            Action::SaveCurrentFile,
        );
        // TODO: switch this to a dedicated "write all files and quit" action once available.
        // TODO: instead of always writing the file, only write it if the buffer was modified, like
        // the :update command in vim.
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Normal,
            vec![KeyInput::Char(' '), KeyInput::Char('q')],
            Action::SaveCurrentFileAndQuit,
        );

        // Visual mode bindings mirror the existing normal-mode motion set so
        // selections can be adjusted with the same muscle memory.
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('h'),
            Action::MoveLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('j'),
            Action::MoveDown,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('k'),
            Action::MoveUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('l'),
            Action::MoveRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('w'),
            Action::MoveWordForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('b'),
            Action::MoveWordBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('e'),
            Action::MoveWordEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('{'),
            Action::MoveParagraphBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('}'),
            Action::MoveParagraphForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('0'),
            Action::MoveLineStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('$'),
            Action::MoveLineEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('^'),
            Action::MoveFirstNonBlank,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('G'),
            Action::MoveToLastLine,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Ctrl('f'),
            Action::PageDown,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Ctrl('b'),
            Action::PageUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Ctrl('d'),
            Action::HalfPageDown,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Ctrl('u'),
            Action::HalfPageUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('f'),
            Action::FindForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('F'),
            Action::FindBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('t'),
            Action::TillForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('T'),
            Action::TillBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char(';'),
            Action::RepeatFindForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char(','),
            Action::RepeatFindBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('n'),
            Action::SearchNext,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('N'),
            Action::SearchPrevious,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('v'),
            Action::EnterVisualMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('V'),
            Action::EnterVisualLineMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('d'),
            Action::DeleteSelection,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Char('c'),
            Action::ChangeSelection,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Visual,
            KeyInput::Escape,
            Action::ExitToNormalMode,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Visual,
            vec![KeyInput::Char('g'), KeyInput::Char('g')],
            Action::MoveToFirstLine,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Visual,
            vec![KeyInput::Char('g'), KeyInput::Char('$')],
            Action::MoveLineEnd,
        );
        Self::add_sequence_binding(
            &mut sequence_bindings,
            ModeContext::Visual,
            vec![KeyInput::Char('g'), KeyInput::Char('0')],
            Action::MoveLineStart,
        );

        // Insert mode bindings
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Escape,
            Action::ExitToNormalMode,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Backspace,
            Action::DeleteCharBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Char('\n'),
            Action::InsertNewline,
        );
        // Arrow keys
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Left,
            Action::MoveLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Right,
            Action::MoveRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Up,
            Action::MoveUp,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Down,
            Action::MoveDown,
        );
        // Home/End
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Home,
            Action::MoveLineStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::End,
            Action::MovePastLineEnd,
        );
        // Delete forward
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Delete,
            Action::DeleteCharForward,
        );
        // Ctrl+W: delete word backward
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Ctrl('w'),
            Action::DeleteWordBackward,
        );
        // Ctrl+H: delete char backward (same as backspace)
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Ctrl('h'),
            Action::DeleteCharBackward,
        );
        // Ctrl+U: delete to line start (delete word backward to beginning)
        Self::add_binding(
            &mut bindings,
            ModeContext::Insert,
            KeyInput::Ctrl('u'),
            Action::DeleteToLineStart,
        );

        // Command mode bindings
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Escape,
            Action::CancelCommand,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Char('\n'),
            Action::ExecuteCommand,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Backspace,
            Action::DeleteInputChar,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('h'),
            Action::DeleteInputChar,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Delete,
            Action::DeleteInputCharForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('d'),
            Action::DeleteInputCharForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('w'),
            Action::DeleteInputWordBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('u'),
            Action::DeleteInputToStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('k'),
            Action::DeleteInputToEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('a'),
            Action::MoveInputStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('e'),
            Action::MoveInputEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Home,
            Action::MoveInputStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::End,
            Action::MoveInputEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('b'),
            Action::MoveInputLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Ctrl('f'),
            Action::MoveInputRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Left,
            Action::MoveInputLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Right,
            Action::MoveInputRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Alt('b'),
            Action::MoveInputWordLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Command,
            KeyInput::Alt('f'),
            Action::MoveInputWordRight,
        );

        // Search mode bindings
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Escape,
            Action::CancelCommand,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Char('\n'),
            Action::ExecuteCommand,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Backspace,
            Action::DeleteInputChar,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('h'),
            Action::DeleteInputChar,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Delete,
            Action::DeleteInputCharForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('d'),
            Action::DeleteInputCharForward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('w'),
            Action::DeleteInputWordBackward,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('u'),
            Action::DeleteInputToStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('k'),
            Action::DeleteInputToEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('a'),
            Action::MoveInputStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('e'),
            Action::MoveInputEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Home,
            Action::MoveInputStart,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::End,
            Action::MoveInputEnd,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('b'),
            Action::MoveInputLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Ctrl('f'),
            Action::MoveInputRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Left,
            Action::MoveInputLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Right,
            Action::MoveInputRight,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Alt('b'),
            Action::MoveInputWordLeft,
        );
        Self::add_binding(
            &mut bindings,
            ModeContext::Search,
            KeyInput::Alt('f'),
            Action::MoveInputWordRight,
        );

        Self {
            bindings,
            sequence_bindings,
        }
    }

    fn add_binding(
        bindings: &mut HashMap<(ModeContext, KeyInput), ActionBinding>,
        mode: ModeContext,
        key: KeyInput,
        action: Action,
    ) {
        bindings.insert((mode, key), ActionBinding::single(action));
    }

    fn add_sequence_binding(
        sequence_bindings: &mut Vec<SequenceBinding>,
        mode: ModeContext,
        keys: Vec<KeyInput>,
        action: Action,
    ) {
        sequence_bindings.push(SequenceBinding {
            mode,
            keys,
            actions: ActionBinding::single(action),
        });
    }

    /// Get the action for a key press in the given mode
    /// Returns None if no binding exists (caller should handle specially for insert/command modes)
    #[cfg(test)]
    pub(crate) fn get_action(&self, key: Key, mode: &Mode) -> Option<Action> {
        match self.get_binding(key, mode) {
            Some(ActionBinding::Single(action)) => Some(*action),
            _ => None,
        }
    }

    /// Get the configured action binding for a key press in the given mode.
    pub(crate) fn get_binding(&self, key: Key, mode: &Mode) -> Option<&ActionBinding> {
        let context = ModeContext::from(mode);
        let key_input = KeyInput::from(key);
        self.bindings.get(&(context, key_input))
    }

    /// Check if a key can begin a known multi-key sequence in the given mode.
    pub(crate) fn starts_sequence_prefix(&self, mode: &Mode, key: &KeyInput) -> bool {
        let context = ModeContext::from(mode);
        self.sequence_bindings.iter().any(|binding| {
            binding.mode == context && binding.keys.len() > 1 && binding.keys.first() == Some(key)
        })
    }

    /// Match a sequence of keys against configured multi-key bindings.
    pub(crate) fn match_sequence(&self, mode: &Mode, keys: &[KeyInput]) -> SequenceMatch {
        let context = ModeContext::from(mode);
        let mut has_prefix = false;

        for binding in self
            .sequence_bindings
            .iter()
            .filter(|binding| binding.mode == context)
        {
            if binding.keys == keys {
                return SequenceMatch::Exact(binding.actions.clone());
            }
            if binding.keys.starts_with(keys) {
                has_prefix = true;
            }
        }

        if has_prefix {
            SequenceMatch::Prefix
        } else {
            SequenceMatch::NoMatch
        }
    }

    /// Return every configured continuation that remains valid for `keys`.
    pub(crate) fn continuations_for_prefix(
        &self,
        mode: &Mode,
        keys: &[KeyInput],
    ) -> Vec<SequenceContinuation> {
        let context = ModeContext::from(mode);

        // Discovery only lists bindings that need at least one more key.
        self.sequence_bindings
            .iter()
            .filter(|binding| {
                binding.mode == context
                    && binding.keys.len() > keys.len()
                    && binding.keys.starts_with(keys)
            })
            .map(|binding| SequenceContinuation {
                remaining_keys: binding.keys[keys.len()..].to_vec(),
                actions: binding.actions.clone(),
            })
            .collect()
    }

    /// Check if a key is a character that should be inserted/appended in the current mode
    /// This handles the case where typed characters aren't in the bindings map
    pub(crate) fn is_insertable_char(key: Key) -> Option<char> {
        if let Key::Char(c) = key {
            // Newline is handled specially
            if c != '\n' {
                return Some(c);
            }
        }
        None
    }

    /// Override or add a key binding with one or more actions at runtime.
    #[cfg(test)]
    pub(crate) fn set_binding_actions(
        &mut self,
        mode: ModeContext,
        key: KeyInput,
        actions: Vec<Action>,
    ) {
        let binding =
            ActionBinding::from_actions(actions).expect("binding actions must not be empty");
        self.set_binding_action_binding(mode, key, binding);
    }

    /// Override or add a key binding using a pre-built action binding.
    pub(crate) fn set_binding_action_binding(
        &mut self,
        mode: ModeContext,
        key: KeyInput,
        binding: ActionBinding,
    ) {
        self.bindings.insert((mode, key), binding);
    }

    /// Override or add a multi-key sequence binding with one or more actions.
    #[cfg(test)]
    pub(crate) fn set_sequence_binding_actions(
        &mut self,
        mode: ModeContext,
        keys: Vec<KeyInput>,
        actions: Vec<Action>,
    ) {
        let binding =
            ActionBinding::from_actions(actions).expect("sequence actions must not be empty");
        self.set_sequence_binding_action_binding(mode, keys, binding);
    }

    /// Override or add a multi-key sequence binding using a pre-built action binding.
    pub(crate) fn set_sequence_binding_action_binding(
        &mut self,
        mode: ModeContext,
        mut keys: Vec<KeyInput>,
        binding: ActionBinding,
    ) {
        if keys.len() == 1 {
            let key = keys.pop().expect("single-key path checked length");
            self.bindings.insert((mode, key), binding);
            return;
        }
        self.sequence_bindings
            .retain(|binding| !(binding.mode == mode && binding.keys == keys));
        self.sequence_bindings.push(SequenceBinding {
            mode,
            keys,
            actions: binding,
        });
    }
}

/// Parse a configuration mode name into a runtime mode context.
pub(crate) fn parse_mode_context(input: &str) -> Option<ModeContext> {
    match input.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(ModeContext::Normal),
        "visual" => Some(ModeContext::Visual),
        "insert" => Some(ModeContext::Insert),
        "command" => Some(ModeContext::Command),
        "search" => Some(ModeContext::Search),
        _ => None,
    }
}

/// Parse a textual key name from configuration into a key input value.
pub(crate) fn parse_key_input(input: &str) -> Option<KeyInput> {
    let normalized = input.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized.chars().count() == 1 {
        return normalized.chars().next().map(KeyInput::Char);
    }

    let lower = normalized.to_ascii_lowercase();
    if let Some(key) = parse_modified_named_key(&lower) {
        return Some(key);
    }
    if let Some(rest) = lower.strip_prefix("ctrl-") {
        if rest.chars().count() == 1 {
            return rest.chars().next().map(KeyInput::Ctrl);
        }
        return None;
    }
    if let Some(rest) = lower.strip_prefix("alt-") {
        if rest.chars().count() == 1 {
            return rest.chars().next().map(KeyInput::Alt);
        }
        return None;
    }

    parse_named_key(&lower)
}

/// Parse one non-modified named key token from configuration syntax.
fn parse_named_key(input: &str) -> Option<KeyInput> {
    match input {
        "backspace" => Some(KeyInput::Backspace),
        "escape" | "esc" => Some(KeyInput::Escape),
        "backtab" => Some(KeyInput::BackTab),
        "up" => Some(KeyInput::Up),
        "down" => Some(KeyInput::Down),
        "left" => Some(KeyInput::Left),
        "right" => Some(KeyInput::Right),
        "home" => Some(KeyInput::Home),
        "end" => Some(KeyInput::End),
        "pageup" => Some(KeyInput::PageUp),
        "pagedown" => Some(KeyInput::PageDown),
        "delete" | "del" => Some(KeyInput::Delete),
        "insert" | "ins" => Some(KeyInput::Insert),
        "space" => Some(KeyInput::Char(' ')),
        _ => None,
    }
}

/// Parse modifier-plus-named-key forms like `ctrl-home` or `shift-tab`.
///
/// The config syntax only accepts `-` as the separator between the modifier and
/// the named key.
fn parse_modified_named_key(input: &str) -> Option<KeyInput> {
    let (modifier, key) = input.split_once('-')?;
    match modifier {
        "shift" => match key {
            "tab" => Some(KeyInput::BackTab),
            "up" => Some(KeyInput::ShiftUp),
            "down" => Some(KeyInput::ShiftDown),
            "left" => Some(KeyInput::ShiftLeft),
            "right" => Some(KeyInput::ShiftRight),
            _ => None,
        },
        "alt" => match key {
            "up" => Some(KeyInput::AltUp),
            "down" => Some(KeyInput::AltDown),
            "left" => Some(KeyInput::AltLeft),
            "right" => Some(KeyInput::AltRight),
            _ => None,
        },
        "ctrl" => match key {
            "up" => Some(KeyInput::CtrlUp),
            "down" => Some(KeyInput::CtrlDown),
            "left" => Some(KeyInput::CtrlLeft),
            "right" => Some(KeyInput::CtrlRight),
            "home" => Some(KeyInput::CtrlHome),
            "end" => Some(KeyInput::CtrlEnd),
            _ => None,
        },
        _ => None,
    }
}

/// Detect modifier-like tokens that use an unsupported separator.
///
/// Forms such as `ctrl+home` should be rejected as invalid modifier syntax
/// instead of being reinterpreted as raw multi-key character sequences.
fn has_invalid_modifier_separator(input: &str) -> bool {
    ["ctrl", "alt", "shift"].into_iter().any(|modifier| {
        input
            .strip_prefix(modifier)
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(|separator| !separator.is_ascii_alphanumeric() && separator != '-')
    })
}

/// Parse a textual key mapping into one or more key inputs.
pub(crate) fn parse_key_sequence(input: &str) -> Option<Vec<KeyInput>> {
    let trimmed = input.trim();
    if let Some(single) = parse_key_input(trimmed) {
        return Some(vec![single]);
    }
    let lower = trimmed.to_ascii_lowercase();
    if has_invalid_modifier_separator(&lower) {
        return None;
    }
    if lower.starts_with("ctrl-") || lower.starts_with("alt-") || lower.starts_with("shift-") {
        return None;
    }
    let keys: Vec<KeyInput> = trimmed.chars().map(KeyInput::Char).collect();
    if keys.len() > 1 { Some(keys) } else { None }
}

/// Parse a textual action name from configuration into an editor action.
pub(crate) fn parse_action(input: &str) -> Option<Action> {
    let normalized = input.trim();
    if normalized.is_empty() {
        return None;
    }
    match normalized {
        "move-left" => Some(Action::MoveLeft),
        "move-right" => Some(Action::MoveRight),
        "move-up" => Some(Action::MoveUp),
        "move-down" => Some(Action::MoveDown),
        "move-word-forward" => Some(Action::MoveWordForward),
        "move-word-backward" => Some(Action::MoveWordBackward),
        "move-word-end" => Some(Action::MoveWordEnd),
        "move-paragraph-forward" => Some(Action::MoveParagraphForward),
        "move-paragraph-backward" => Some(Action::MoveParagraphBackward),
        "move-line-start" => Some(Action::MoveLineStart),
        "move-line-end" => Some(Action::MoveLineEnd),
        "move-past-line-end" => Some(Action::MovePastLineEnd),
        "move-first-non-blank" => Some(Action::MoveFirstNonBlank),
        "move-to-first-line" => Some(Action::MoveToFirstLine),
        "move-to-last-line" => Some(Action::MoveToLastLine),
        "page-up" => Some(Action::PageUp),
        "page-down" => Some(Action::PageDown),
        "half-page-up" => Some(Action::HalfPageUp),
        "half-page-down" => Some(Action::HalfPageDown),
        "find-forward" => Some(Action::FindForward),
        "find-backward" => Some(Action::FindBackward),
        "till-forward" => Some(Action::TillForward),
        "till-backward" => Some(Action::TillBackward),
        "repeat-find-forward" => Some(Action::RepeatFindForward),
        "repeat-find-backward" => Some(Action::RepeatFindBackward),
        "enter-insert-mode" => Some(Action::EnterInsertMode),
        "enter-visual-mode" => Some(Action::EnterVisualMode),
        "enter-visual-line-mode" => Some(Action::EnterVisualLineMode),
        "insert-after-cursor" => Some(Action::InsertAfterCursor),
        "open-line-below" => Some(Action::OpenLineBelow),
        "open-line-above" => Some(Action::OpenLineAbove),
        "enter-command-mode" => Some(Action::EnterCommandMode),
        "enter-search-mode" => Some(Action::EnterSearchMode),
        "exit-to-normal-mode" => Some(Action::ExitToNormalMode),
        "search-next" => Some(Action::SearchNext),
        "search-previous" => Some(Action::SearchPrevious),
        "save-current-file" => Some(Action::SaveCurrentFile),
        "save-current-file-and-quit" => Some(Action::SaveCurrentFileAndQuit),
        "delete-char-backward" => Some(Action::DeleteCharBackward),
        "delete-char-forward" => Some(Action::DeleteCharForward),
        "delete-char-at-cursor" => Some(Action::DeleteCharAtCursor),
        "delete-word-backward" => Some(Action::DeleteWordBackward),
        "delete-to-line-start" => Some(Action::DeleteToLineStart),
        "insert-newline" => Some(Action::InsertNewline),
        "delete-selection" => Some(Action::DeleteSelection),
        "change-selection" => Some(Action::ChangeSelection),
        "change-inner-word" => Some(Action::ChangeInnerWord),
        "delete-inner-word" => Some(Action::DeleteInnerWord),
        "delete-around-paren" => Some(Action::DeleteAroundParen),
        "execute-command" => Some(Action::ExecuteCommand),
        "cancel-command" => Some(Action::CancelCommand),
        "delete-input-char" => Some(Action::DeleteInputChar),
        "delete-input-char-forward" => Some(Action::DeleteInputCharForward),
        "delete-input-word-backward" => Some(Action::DeleteInputWordBackward),
        "delete-input-to-start" => Some(Action::DeleteInputToStart),
        "delete-input-to-end" => Some(Action::DeleteInputToEnd),
        "move-input-start" => Some(Action::MoveInputStart),
        "move-input-end" => Some(Action::MoveInputEnd),
        "move-input-left" => Some(Action::MoveInputLeft),
        "move-input-right" => Some(Action::MoveInputRight),
        "move-input-word-left" => Some(Action::MoveInputWordLeft),
        "move-input-word-right" => Some(Action::MoveInputWordRight),
        _ => None,
    }
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self::new()
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
        assert_eq!(bindings.get_action(Key::Up, &mode), Some(Action::MoveUp));
        assert_eq!(
            bindings.get_action(Key::Down, &mode),
            Some(Action::MoveDown)
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
            bindings.get_action(Key::Char('c'), &mode),
            Some(Action::ChangeSelection)
        );
        assert_eq!(
            bindings.get_action(Key::Char('v'), &mode),
            Some(Action::EnterVisualMode)
        );
        assert_eq!(
            bindings.get_action(Key::Esc, &mode),
            Some(Action::ExitToNormalMode)
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

        assert_eq!(labels, vec!["g", "$", "0"]);
        assert_eq!(
            actions,
            vec!["Move to first line", "Move line end", "Move line start"]
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

        assert_eq!(labels, vec!["u", "q"]);
        assert_eq!(
            actions,
            vec!["Move down -> Move right", "Save current file"]
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
    fn test_sequence_ciw_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![
            KeyInput::Char('c'),
            KeyInput::Char('i'),
            KeyInput::Char('w'),
        ];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::ChangeInnerWord))
        );
    }

    #[test]
    fn test_sequence_diw_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![
            KeyInput::Char('d'),
            KeyInput::Char('i'),
            KeyInput::Char('w'),
        ];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::DeleteInnerWord))
        );
    }

    #[test]
    fn test_sequence_da_paren_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![
            KeyInput::Char('d'),
            KeyInput::Char('a'),
            KeyInput::Char('('),
        ];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::DeleteAroundParen))
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
    fn test_sequence_space_q_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' '), KeyInput::Char('q')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(ActionBinding::Single(Action::SaveCurrentFileAndQuit))
        );
    }

    #[test]
    fn test_parse_key_input_complex_keys() {
        assert_eq!(parse_key_input("ctrl-f"), Some(KeyInput::Ctrl('f')));
        assert_eq!(parse_key_input("alt-b"), Some(KeyInput::Alt('b')));
        assert_eq!(parse_key_input("ctrl-home"), Some(KeyInput::CtrlHome));
        assert_eq!(parse_key_input("ctrl-end"), Some(KeyInput::CtrlEnd));
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
            parse_action("enter-visual-mode"),
            Some(Action::EnterVisualMode)
        );
        assert_eq!(
            parse_action("change-selection"),
            Some(Action::ChangeSelection)
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
