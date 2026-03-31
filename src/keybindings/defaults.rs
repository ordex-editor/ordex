//! Built-in keybinding tables and default registry construction.

use super::registry::KeyBindings;
use super::{Action, ActionBinding, KeyInput, ModeContext};

const NORMAL_VISUAL_MODES: &[ModeContext] = &[ModeContext::Normal, ModeContext::Visual];
const COMMAND_SEARCH_MODES: &[ModeContext] = &[ModeContext::Command, ModeContext::Search];

const NORMAL_VISUAL_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Char('h'), Action::MoveLeft),
    (KeyInput::Char('j'), Action::MoveDown),
    (KeyInput::Char('k'), Action::MoveUp),
    (KeyInput::Char('l'), Action::MoveRight),
    (KeyInput::Char('w'), Action::MoveWordForward),
    (KeyInput::Char('b'), Action::MoveWordBackward),
    (KeyInput::Ctrl('f'), Action::PageDown),
    (KeyInput::Ctrl('b'), Action::PageUp),
    (KeyInput::Ctrl('d'), Action::HalfPageDown),
    (KeyInput::Ctrl('u'), Action::HalfPageUp),
    (KeyInput::Ctrl('y'), Action::ScrollLineUp),
    (KeyInput::Ctrl('e'), Action::ScrollLineDown),
    (KeyInput::Char('n'), Action::SearchNext),
    (KeyInput::Char('N'), Action::SearchPrevious),
    (KeyInput::Char('0'), Action::MoveLineStart),
    (KeyInput::Char('$'), Action::MoveLineEnd),
    (KeyInput::Char('^'), Action::MoveFirstNonBlank),
    (KeyInput::Char('e'), Action::MoveWordEnd),
    (KeyInput::Char('{'), Action::MoveParagraphBackward),
    (KeyInput::Char('}'), Action::MoveParagraphForward),
    (KeyInput::Char('f'), Action::FindForward),
    (KeyInput::Char('F'), Action::FindBackward),
    (KeyInput::Char('t'), Action::TillForward),
    (KeyInput::Char('T'), Action::TillBackward),
    (KeyInput::Char(';'), Action::RepeatFindForward),
    (KeyInput::Char(','), Action::RepeatFindBackward),
    (KeyInput::Char('%'), Action::MatchBracket),
    (KeyInput::Char('G'), Action::MoveToLastLine),
    (KeyInput::Char('v'), Action::EnterVisualMode),
    (KeyInput::Char('V'), Action::EnterVisualLineMode),
];

const NORMAL_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Char('i'), Action::EnterInsertMode),
    (KeyInput::Char('a'), Action::InsertAfterCursor),
    (KeyInput::Char('p'), Action::PasteAfterCursor),
    (KeyInput::Char('P'), Action::PasteBeforeCursor),
    (KeyInput::Char('x'), Action::DeleteCharAtCursor),
    (KeyInput::Char('u'), Action::Undo),
    (KeyInput::Ctrl('r'), Action::Redo),
    (KeyInput::Char('o'), Action::OpenLineBelow),
    (KeyInput::Char('O'), Action::OpenLineAbove),
    (KeyInput::Char(':'), Action::EnterCommandMode),
    (KeyInput::Char('/'), Action::EnterSearchMode),
];

const VISUAL_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Char('o'), Action::SwapVisualAnchor),
    (KeyInput::Char('d'), Action::DeleteSelection),
    (KeyInput::Char('y'), Action::YankSelection),
    (KeyInput::Char('c'), Action::ChangeSelection),
    (KeyInput::Escape, Action::ExitToNormalMode),
];

const INSERT_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Escape, Action::ExitToNormalMode),
    (KeyInput::Backspace, Action::DeleteCharBackward),
    (KeyInput::Char('\n'), Action::InsertNewline),
    (KeyInput::Left, Action::MoveLeft),
    (KeyInput::Right, Action::MoveRight),
    (KeyInput::Up, Action::MoveUp),
    (KeyInput::Down, Action::MoveDown),
    (KeyInput::Home, Action::MoveLineStart),
    (KeyInput::End, Action::MovePastLineEnd),
    (KeyInput::Delete, Action::DeleteCharForward),
    (KeyInput::Ctrl('w'), Action::DeleteWordBackward),
    (KeyInput::Ctrl('h'), Action::DeleteCharBackward),
    (KeyInput::Ctrl('u'), Action::DeleteToLineStart),
];

const COMMAND_SEARCH_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Escape, Action::CancelCommand),
    (KeyInput::Char('\n'), Action::ExecuteCommand),
    (KeyInput::Backspace, Action::DeleteInputChar),
    (KeyInput::Ctrl('h'), Action::DeleteInputChar),
    (KeyInput::Delete, Action::DeleteInputCharForward),
    (KeyInput::Ctrl('d'), Action::DeleteInputCharForward),
    (KeyInput::Ctrl('w'), Action::DeleteInputWordBackward),
    (KeyInput::Ctrl('u'), Action::DeleteInputToStart),
    (KeyInput::Ctrl('k'), Action::DeleteInputToEnd),
    (KeyInput::Ctrl('a'), Action::MoveInputStart),
    (KeyInput::Ctrl('e'), Action::MoveInputEnd),
    (KeyInput::Home, Action::MoveInputStart),
    (KeyInput::End, Action::MoveInputEnd),
    (KeyInput::Ctrl('b'), Action::MoveInputLeft),
    (KeyInput::Ctrl('f'), Action::MoveInputRight),
    (KeyInput::Left, Action::MoveInputLeft),
    (KeyInput::Right, Action::MoveInputRight),
    (KeyInput::Alt('b'), Action::MoveInputWordLeft),
    (KeyInput::Alt('f'), Action::MoveInputWordRight),
];

const NORMAL_VISUAL_SEQUENCE_BINDINGS: &[(&[KeyInput], Action)] = &[
    (
        &[KeyInput::Char('g'), KeyInput::Char('g')],
        Action::MoveToFirstLine,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('$')],
        Action::MoveLineEnd,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('0')],
        Action::MoveLineStart,
    ),
    (
        &[KeyInput::Char('z'), KeyInput::Char('t')],
        Action::AlignViewportTop,
    ),
    (
        &[KeyInput::Char('z'), KeyInput::Char('z')],
        Action::AlignViewportCenter,
    ),
    (
        &[KeyInput::Char('z'), KeyInput::Char('b')],
        Action::AlignViewportBottom,
    ),
];

const NORMAL_SEQUENCE_BINDINGS: &[(&[KeyInput], Action)] = &[
    (
        &[KeyInput::Char('g'), KeyInput::Char('v')],
        Action::RecreateLastSelection,
    ),
    (
        &[KeyInput::Char('y'), KeyInput::Char('y')],
        Action::YankCurrentLine,
    ),
    (
        &[
            KeyInput::Char('c'),
            KeyInput::Char('i'),
            KeyInput::Char('w'),
        ],
        Action::ChangeInnerWord,
    ),
    (
        &[
            KeyInput::Char('d'),
            KeyInput::Char('i'),
            KeyInput::Char('w'),
        ],
        Action::DeleteInnerWord,
    ),
    (
        &[
            KeyInput::Char('d'),
            KeyInput::Char('a'),
            KeyInput::Char('('),
        ],
        Action::DeleteAroundParen,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('w')],
        Action::SaveCurrentFile,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('q')],
        Action::UpdateCurrentFileAndQuit,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('b')],
        Action::OpenBufferSwitcher,
    ),
];

const NORMAL_MULTI_ACTION_BINDINGS: &[(KeyInput, &[Action])] = &[
    (
        KeyInput::Char('I'),
        &[Action::MoveFirstNonBlank, Action::EnterInsertMode],
    ),
    (
        KeyInput::Char('A'),
        &[Action::MoveLineEnd, Action::InsertAfterCursor],
    ),
];

impl KeyBindings {
    /// Create the built-in key binding registry.
    pub(crate) fn new() -> Self {
        let mut bindings = Self::empty();

        // Register shared mode groups before layering on mode-specific overrides.
        register_single_bindings_for_modes(
            &mut bindings,
            NORMAL_VISUAL_MODES,
            NORMAL_VISUAL_SINGLE_BINDINGS,
        );
        register_single_bindings_for_modes(
            &mut bindings,
            COMMAND_SEARCH_MODES,
            COMMAND_SEARCH_SINGLE_BINDINGS,
        );
        register_sequence_bindings_for_modes(
            &mut bindings,
            NORMAL_VISUAL_MODES,
            NORMAL_VISUAL_SEQUENCE_BINDINGS,
        );

        register_single_bindings_for_mode(
            &mut bindings,
            ModeContext::Normal,
            NORMAL_SINGLE_BINDINGS,
        );
        register_single_bindings_for_mode(
            &mut bindings,
            ModeContext::Visual,
            VISUAL_SINGLE_BINDINGS,
        );
        register_single_bindings_for_mode(
            &mut bindings,
            ModeContext::Insert,
            INSERT_SINGLE_BINDINGS,
        );
        register_sequence_bindings_for_mode(
            &mut bindings,
            ModeContext::Normal,
            NORMAL_SEQUENCE_BINDINGS,
        );

        // Register bindings whose payload executes multiple actions in order.
        register_multi_action_bindings_for_mode(
            &mut bindings,
            ModeContext::Normal,
            NORMAL_MULTI_ACTION_BINDINGS,
        );
        bindings
    }
}

impl Default for KeyBindings {
    /// Build the default key binding registry.
    fn default() -> Self {
        Self::new()
    }
}

/// Register one table of single-key bindings for every mode in `modes`.
fn register_single_bindings_for_modes(
    bindings: &mut KeyBindings,
    modes: &[ModeContext],
    entries: &[(KeyInput, Action)],
) {
    for mode in modes.iter().copied() {
        register_single_bindings_for_mode(bindings, mode, entries);
    }
}

/// Register one table of single-key bindings for a specific mode.
fn register_single_bindings_for_mode(
    bindings: &mut KeyBindings,
    mode: ModeContext,
    entries: &[(KeyInput, Action)],
) {
    for (key, action) in entries.iter().cloned() {
        bindings.insert_action(mode, key, action);
    }
}

/// Register one table of sequence bindings for every mode in `modes`.
fn register_sequence_bindings_for_modes(
    bindings: &mut KeyBindings,
    modes: &[ModeContext],
    entries: &[(&[KeyInput], Action)],
) {
    for mode in modes.iter().copied() {
        register_sequence_bindings_for_mode(bindings, mode, entries);
    }
}

/// Register one table of sequence bindings for a specific mode.
fn register_sequence_bindings_for_mode(
    bindings: &mut KeyBindings,
    mode: ModeContext,
    entries: &[(&[KeyInput], Action)],
) {
    for (keys, action) in entries.iter().cloned() {
        bindings.insert_sequence_action(mode, keys.to_vec(), action);
    }
}

/// Register one table of multi-action bindings for a specific mode.
fn register_multi_action_bindings_for_mode(
    bindings: &mut KeyBindings,
    mode: ModeContext,
    entries: &[(KeyInput, &[Action])],
) {
    for (key, actions) in entries.iter().cloned() {
        let binding = ActionBinding::from_actions(actions.to_vec())
            .expect("built-in binding actions must not be empty");
        bindings.set_binding_action_binding(mode, key, binding);
    }
}
