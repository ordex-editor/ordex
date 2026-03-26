//! Built-in keybinding tables and default registry construction.

use super::registry::KeyBindings;
use super::{Action, ActionBinding, KeyInput, ModeContext};

const SINGLE_BINDINGS: &[(ModeContext, KeyInput, Action)] = &[
    // Normal mode.
    (ModeContext::Normal, KeyInput::Char('h'), Action::MoveLeft),
    (ModeContext::Normal, KeyInput::Char('j'), Action::MoveDown),
    (ModeContext::Normal, KeyInput::Char('k'), Action::MoveUp),
    (ModeContext::Normal, KeyInput::Char('l'), Action::MoveRight),
    (
        ModeContext::Normal,
        KeyInput::Char('w'),
        Action::MoveWordForward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('b'),
        Action::MoveWordBackward,
    ),
    (ModeContext::Normal, KeyInput::Ctrl('f'), Action::PageDown),
    (ModeContext::Normal, KeyInput::Ctrl('b'), Action::PageUp),
    (
        ModeContext::Normal,
        KeyInput::Ctrl('d'),
        Action::HalfPageDown,
    ),
    (ModeContext::Normal, KeyInput::Ctrl('u'), Action::HalfPageUp),
    (
        ModeContext::Normal,
        KeyInput::Ctrl('y'),
        Action::ScrollLineUp,
    ),
    (
        ModeContext::Normal,
        KeyInput::Ctrl('e'),
        Action::ScrollLineDown,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('i'),
        Action::EnterInsertMode,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('v'),
        Action::EnterVisualMode,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('V'),
        Action::EnterVisualLineMode,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('a'),
        Action::InsertAfterCursor,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('p'),
        Action::PasteAfterCursor,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('P'),
        Action::PasteBeforeCursor,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('x'),
        Action::DeleteCharAtCursor,
    ),
    (ModeContext::Normal, KeyInput::Char('u'), Action::Undo),
    (ModeContext::Normal, KeyInput::Ctrl('r'), Action::Redo),
    (
        ModeContext::Normal,
        KeyInput::Char('o'),
        Action::OpenLineBelow,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('O'),
        Action::OpenLineAbove,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char(':'),
        Action::EnterCommandMode,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('/'),
        Action::EnterSearchMode,
    ),
    (ModeContext::Normal, KeyInput::Char('n'), Action::SearchNext),
    (
        ModeContext::Normal,
        KeyInput::Char('N'),
        Action::SearchPrevious,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('0'),
        Action::MoveLineStart,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('$'),
        Action::MoveLineEnd,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('^'),
        Action::MoveFirstNonBlank,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('e'),
        Action::MoveWordEnd,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('{'),
        Action::MoveParagraphBackward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('}'),
        Action::MoveParagraphForward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('f'),
        Action::FindForward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('F'),
        Action::FindBackward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('t'),
        Action::TillForward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('T'),
        Action::TillBackward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char(';'),
        Action::RepeatFindForward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char(','),
        Action::RepeatFindBackward,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('%'),
        Action::MatchBracket,
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('G'),
        Action::MoveToLastLine,
    ),
    // Visual mode.
    (ModeContext::Visual, KeyInput::Char('h'), Action::MoveLeft),
    (ModeContext::Visual, KeyInput::Char('j'), Action::MoveDown),
    (ModeContext::Visual, KeyInput::Char('k'), Action::MoveUp),
    (ModeContext::Visual, KeyInput::Char('l'), Action::MoveRight),
    (
        ModeContext::Visual,
        KeyInput::Char('w'),
        Action::MoveWordForward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('b'),
        Action::MoveWordBackward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('e'),
        Action::MoveWordEnd,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('{'),
        Action::MoveParagraphBackward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('}'),
        Action::MoveParagraphForward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('0'),
        Action::MoveLineStart,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('$'),
        Action::MoveLineEnd,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('^'),
        Action::MoveFirstNonBlank,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('G'),
        Action::MoveToLastLine,
    ),
    (ModeContext::Visual, KeyInput::Ctrl('f'), Action::PageDown),
    (ModeContext::Visual, KeyInput::Ctrl('b'), Action::PageUp),
    (
        ModeContext::Visual,
        KeyInput::Ctrl('d'),
        Action::HalfPageDown,
    ),
    (ModeContext::Visual, KeyInput::Ctrl('u'), Action::HalfPageUp),
    (
        ModeContext::Visual,
        KeyInput::Ctrl('y'),
        Action::ScrollLineUp,
    ),
    (
        ModeContext::Visual,
        KeyInput::Ctrl('e'),
        Action::ScrollLineDown,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('f'),
        Action::FindForward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('F'),
        Action::FindBackward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('t'),
        Action::TillForward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('T'),
        Action::TillBackward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char(';'),
        Action::RepeatFindForward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char(','),
        Action::RepeatFindBackward,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('%'),
        Action::MatchBracket,
    ),
    (ModeContext::Visual, KeyInput::Char('n'), Action::SearchNext),
    (
        ModeContext::Visual,
        KeyInput::Char('N'),
        Action::SearchPrevious,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('v'),
        Action::EnterVisualMode,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('V'),
        Action::EnterVisualLineMode,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('o'),
        Action::SwapVisualAnchor,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('d'),
        Action::DeleteSelection,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('y'),
        Action::YankSelection,
    ),
    (
        ModeContext::Visual,
        KeyInput::Char('c'),
        Action::ChangeSelection,
    ),
    (
        ModeContext::Visual,
        KeyInput::Escape,
        Action::ExitToNormalMode,
    ),
    // Insert mode.
    (
        ModeContext::Insert,
        KeyInput::Escape,
        Action::ExitToNormalMode,
    ),
    (
        ModeContext::Insert,
        KeyInput::Backspace,
        Action::DeleteCharBackward,
    ),
    (
        ModeContext::Insert,
        KeyInput::Char('\n'),
        Action::InsertNewline,
    ),
    (ModeContext::Insert, KeyInput::Left, Action::MoveLeft),
    (ModeContext::Insert, KeyInput::Right, Action::MoveRight),
    (ModeContext::Insert, KeyInput::Up, Action::MoveUp),
    (ModeContext::Insert, KeyInput::Down, Action::MoveDown),
    (ModeContext::Insert, KeyInput::Home, Action::MoveLineStart),
    (ModeContext::Insert, KeyInput::End, Action::MovePastLineEnd),
    (
        ModeContext::Insert,
        KeyInput::Delete,
        Action::DeleteCharForward,
    ),
    (
        ModeContext::Insert,
        KeyInput::Ctrl('w'),
        Action::DeleteWordBackward,
    ),
    (
        ModeContext::Insert,
        KeyInput::Ctrl('h'),
        Action::DeleteCharBackward,
    ),
    (
        ModeContext::Insert,
        KeyInput::Ctrl('u'),
        Action::DeleteToLineStart,
    ),
    // Command mode.
    (
        ModeContext::Command,
        KeyInput::Escape,
        Action::CancelCommand,
    ),
    (
        ModeContext::Command,
        KeyInput::Char('\n'),
        Action::ExecuteCommand,
    ),
    (
        ModeContext::Command,
        KeyInput::Backspace,
        Action::DeleteInputChar,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('h'),
        Action::DeleteInputChar,
    ),
    (
        ModeContext::Command,
        KeyInput::Delete,
        Action::DeleteInputCharForward,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('d'),
        Action::DeleteInputCharForward,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('w'),
        Action::DeleteInputWordBackward,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('u'),
        Action::DeleteInputToStart,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('k'),
        Action::DeleteInputToEnd,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('a'),
        Action::MoveInputStart,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('e'),
        Action::MoveInputEnd,
    ),
    (ModeContext::Command, KeyInput::Home, Action::MoveInputStart),
    (ModeContext::Command, KeyInput::End, Action::MoveInputEnd),
    (
        ModeContext::Command,
        KeyInput::Ctrl('b'),
        Action::MoveInputLeft,
    ),
    (
        ModeContext::Command,
        KeyInput::Ctrl('f'),
        Action::MoveInputRight,
    ),
    (ModeContext::Command, KeyInput::Left, Action::MoveInputLeft),
    (
        ModeContext::Command,
        KeyInput::Right,
        Action::MoveInputRight,
    ),
    (
        ModeContext::Command,
        KeyInput::Alt('b'),
        Action::MoveInputWordLeft,
    ),
    (
        ModeContext::Command,
        KeyInput::Alt('f'),
        Action::MoveInputWordRight,
    ),
    // Search mode.
    (ModeContext::Search, KeyInput::Escape, Action::CancelCommand),
    (
        ModeContext::Search,
        KeyInput::Char('\n'),
        Action::ExecuteCommand,
    ),
    (
        ModeContext::Search,
        KeyInput::Backspace,
        Action::DeleteInputChar,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('h'),
        Action::DeleteInputChar,
    ),
    (
        ModeContext::Search,
        KeyInput::Delete,
        Action::DeleteInputCharForward,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('d'),
        Action::DeleteInputCharForward,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('w'),
        Action::DeleteInputWordBackward,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('u'),
        Action::DeleteInputToStart,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('k'),
        Action::DeleteInputToEnd,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('a'),
        Action::MoveInputStart,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('e'),
        Action::MoveInputEnd,
    ),
    (ModeContext::Search, KeyInput::Home, Action::MoveInputStart),
    (ModeContext::Search, KeyInput::End, Action::MoveInputEnd),
    (
        ModeContext::Search,
        KeyInput::Ctrl('b'),
        Action::MoveInputLeft,
    ),
    (
        ModeContext::Search,
        KeyInput::Ctrl('f'),
        Action::MoveInputRight,
    ),
    (ModeContext::Search, KeyInput::Left, Action::MoveInputLeft),
    (ModeContext::Search, KeyInput::Right, Action::MoveInputRight),
    (
        ModeContext::Search,
        KeyInput::Alt('b'),
        Action::MoveInputWordLeft,
    ),
    (
        ModeContext::Search,
        KeyInput::Alt('f'),
        Action::MoveInputWordRight,
    ),
];

const SEQUENCE_BINDINGS: &[(ModeContext, &[KeyInput], Action)] = &[
    // Normal mode.
    (
        ModeContext::Normal,
        &[KeyInput::Char('g'), KeyInput::Char('g')],
        Action::MoveToFirstLine,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('g'), KeyInput::Char('$')],
        Action::MoveLineEnd,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('g'), KeyInput::Char('0')],
        Action::MoveLineStart,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('g'), KeyInput::Char('v')],
        Action::RecreateLastSelection,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('y'), KeyInput::Char('y')],
        Action::YankCurrentLine,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('z'), KeyInput::Char('t')],
        Action::AlignViewportTop,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('z'), KeyInput::Char('z')],
        Action::AlignViewportCenter,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char('z'), KeyInput::Char('b')],
        Action::AlignViewportBottom,
    ),
    (
        ModeContext::Normal,
        &[
            KeyInput::Char('c'),
            KeyInput::Char('i'),
            KeyInput::Char('w'),
        ],
        Action::ChangeInnerWord,
    ),
    (
        ModeContext::Normal,
        &[
            KeyInput::Char('d'),
            KeyInput::Char('i'),
            KeyInput::Char('w'),
        ],
        Action::DeleteInnerWord,
    ),
    (
        ModeContext::Normal,
        &[
            KeyInput::Char('d'),
            KeyInput::Char('a'),
            KeyInput::Char('('),
        ],
        Action::DeleteAroundParen,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char(' '), KeyInput::Char('w')],
        Action::SaveCurrentFile,
    ),
    (
        ModeContext::Normal,
        &[KeyInput::Char(' '), KeyInput::Char('q')],
        Action::UpdateCurrentFileAndQuit,
    ),
    // Visual mode.
    (
        ModeContext::Visual,
        &[KeyInput::Char('g'), KeyInput::Char('g')],
        Action::MoveToFirstLine,
    ),
    (
        ModeContext::Visual,
        &[KeyInput::Char('g'), KeyInput::Char('$')],
        Action::MoveLineEnd,
    ),
    (
        ModeContext::Visual,
        &[KeyInput::Char('g'), KeyInput::Char('0')],
        Action::MoveLineStart,
    ),
    (
        ModeContext::Visual,
        &[KeyInput::Char('z'), KeyInput::Char('t')],
        Action::AlignViewportTop,
    ),
    (
        ModeContext::Visual,
        &[KeyInput::Char('z'), KeyInput::Char('z')],
        Action::AlignViewportCenter,
    ),
    (
        ModeContext::Visual,
        &[KeyInput::Char('z'), KeyInput::Char('b')],
        Action::AlignViewportBottom,
    ),
];

const MULTI_ACTION_BINDINGS: &[(ModeContext, KeyInput, &[Action])] = &[
    (
        ModeContext::Normal,
        KeyInput::Char('I'),
        &[Action::MoveFirstNonBlank, Action::EnterInsertMode],
    ),
    (
        ModeContext::Normal,
        KeyInput::Char('A'),
        &[Action::MoveLineEnd, Action::InsertAfterCursor],
    ),
];

impl KeyBindings {
    /// Create the built-in key binding registry.
    pub(crate) fn new() -> Self {
        let mut bindings = Self::empty();

        // Register single-key bindings before loading sequence and macro-style bindings.
        register_single_bindings(&mut bindings, SINGLE_BINDINGS);
        register_sequence_bindings(&mut bindings, SEQUENCE_BINDINGS);

        // Register bindings whose payload executes multiple actions in order.
        register_multi_action_bindings(&mut bindings, MULTI_ACTION_BINDINGS);
        bindings
    }
}

impl Default for KeyBindings {
    /// Build the default key binding registry.
    fn default() -> Self {
        Self::new()
    }
}

/// Register one table of single-key bindings into the runtime registry.
fn register_single_bindings(
    bindings: &mut KeyBindings,
    entries: &[(ModeContext, KeyInput, Action)],
) {
    for (mode, key, action) in entries.iter().cloned() {
        bindings.insert_action(mode, key, action);
    }
}

/// Register one table of sequence bindings into the runtime registry.
fn register_sequence_bindings(
    bindings: &mut KeyBindings,
    entries: &[(ModeContext, &[KeyInput], Action)],
) {
    for (mode, keys, action) in entries.iter().cloned() {
        bindings.insert_sequence_action(mode, keys.to_vec(), action);
    }
}

/// Register one table of multi-action bindings into the runtime registry.
fn register_multi_action_bindings(
    bindings: &mut KeyBindings,
    entries: &[(ModeContext, KeyInput, &[Action])],
) {
    for (mode, key, actions) in entries.iter().cloned() {
        let binding = ActionBinding::from_actions(actions.to_vec())
            .expect("built-in binding actions must not be empty");
        bindings.set_binding_action_binding(mode, key, binding);
    }
}
