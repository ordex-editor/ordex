//! Built-in keybinding tables and default registry construction.

use super::registry::KeyBindings;
use super::{Action, ActionBinding, KeyInput, ModeContext, OperatorBinding};

const NORMAL_VISUAL_MODES: &[ModeContext] = &[ModeContext::Normal, ModeContext::Visual];
const COMMAND_SEARCH_MODES: &[ModeContext] = &[ModeContext::Command, ModeContext::Search];

const NORMAL_VISUAL_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Char('h'), Action::MoveLeft),
    (KeyInput::Char('j'), Action::MoveDown),
    (KeyInput::Char('k'), Action::MoveUp),
    (KeyInput::Char('l'), Action::MoveRight),
    (KeyInput::Char('w'), Action::MoveWordForward),
    (KeyInput::Char('W'), Action::MoveBigWordForward),
    (KeyInput::Char('b'), Action::MoveWordBackward),
    (KeyInput::Char('B'), Action::MoveBigWordBackward),
    (KeyInput::Ctrl('f'), Action::PageDown),
    (KeyInput::Ctrl('b'), Action::PageUp),
    (KeyInput::Ctrl('d'), Action::HalfPageDown),
    (KeyInput::Ctrl('u'), Action::HalfPageUp),
    (KeyInput::Ctrl('y'), Action::ScrollLineUp),
    (KeyInput::Ctrl('e'), Action::ScrollLineDown),
    (KeyInput::Char('n'), Action::SearchNext),
    (KeyInput::Char('N'), Action::SearchPrevious),
    (KeyInput::Ctrl('o'), Action::JumpOlder),
    (KeyInput::Ctrl('i'), Action::JumpNewer),
    (KeyInput::Char('0'), Action::MoveLineStart),
    (KeyInput::Char('$'), Action::MoveLineEnd),
    (KeyInput::Char('^'), Action::MoveFirstNonBlank),
    (KeyInput::Char('_'), Action::MoveDownFirstNonBlank),
    (KeyInput::Char('e'), Action::MoveWordEnd),
    (KeyInput::Char('E'), Action::MoveBigWordEnd),
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
    (KeyInput::Ctrl('v'), Action::EnterVisualBlockMode),
];

const NORMAL_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Char('i'), Action::EnterInsertMode),
    (KeyInput::Char('a'), Action::InsertAfterCursor),
    (KeyInput::Char('d'), Action::BeginDeleteOperator),
    (KeyInput::Char('c'), Action::BeginChangeOperator),
    (KeyInput::Char('y'), Action::BeginYankOperator),
    (KeyInput::Char('='), Action::BeginReindentOperator),
    (KeyInput::Char('>'), Action::BeginIndentOperator),
    (KeyInput::Char('<'), Action::BeginDedentOperator),
    (KeyInput::Char('p'), Action::PasteAfterCursor),
    (KeyInput::Char('P'), Action::PasteBeforeCursor),
    (KeyInput::Char('x'), Action::DeleteCharAtCursor),
    (KeyInput::Char('~'), Action::ToggleCaseAtCursor),
    (KeyInput::Char('D'), Action::DeleteToLineEnd),
    (KeyInput::Char('C'), Action::ChangeToLineEnd),
    (KeyInput::Ctrl('a'), Action::IncrementNextNumber),
    (KeyInput::Ctrl('x'), Action::DecrementNextNumber),
    (KeyInput::Ctrl('l'), Action::RequestFullRedraw),
    (KeyInput::Char('J'), Action::JoinLines),
    (KeyInput::Char('r'), Action::BeginReplaceChar),
    (KeyInput::Char('*'), Action::SearchWordUnderCursor),
    (KeyInput::Char('u'), Action::Undo),
    (KeyInput::Ctrl('r'), Action::Redo),
    (KeyInput::Char('.'), Action::RepeatLastChange),
    (KeyInput::Char('K'), Action::ShowHover),
    (KeyInput::Char('o'), Action::OpenLineBelow),
    (KeyInput::Char('O'), Action::OpenLineAbove),
    (KeyInput::Char('q'), Action::BeginMacroRecord),
    (KeyInput::Char('@'), Action::BeginMacroPlayback),
    (KeyInput::Char(':'), Action::EnterCommandMode),
    (KeyInput::Char('/'), Action::EnterSearchMode),
];

const VISUAL_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Char('o'), Action::SwapVisualAnchor),
    (KeyInput::Char('d'), Action::DeleteSelection),
    (KeyInput::Char('~'), Action::ToggleCaseAtCursor),
    (KeyInput::Char('='), Action::ReindentSelection),
    (KeyInput::Char('>'), Action::IndentSelection),
    (KeyInput::Char('<'), Action::DedentSelection),
    (KeyInput::Char('y'), Action::YankSelection),
    (KeyInput::Char('c'), Action::ChangeSelection),
    (KeyInput::Char('I'), Action::VisualInsertBlockStart),
    (KeyInput::Char('A'), Action::VisualAppendBlockEnd),
    (KeyInput::Escape, Action::ExitToNormalMode),
];

const INSERT_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Escape, Action::ExitToNormalMode),
    (KeyInput::Backspace, Action::DeleteCharBackward),
    (KeyInput::Char('\n'), Action::InsertNewline),
    (KeyInput::Left, Action::MoveLeft),
    (KeyInput::Right, Action::MoveRight),
    (KeyInput::Up, Action::CompletionSelectUp),
    (KeyInput::Down, Action::CompletionSelectDown),
    (KeyInput::Ctrl('p'), Action::CompletionSelectUp),
    (KeyInput::Ctrl('n'), Action::CompletionSelectDown),
    (KeyInput::Home, Action::MoveLineStart),
    (KeyInput::End, Action::MovePastLineEnd),
    (KeyInput::Delete, Action::DeleteCharForward),
    (KeyInput::Ctrl('w'), Action::DeleteWordBackward),
    (KeyInput::Ctrl('h'), Action::DeleteCharBackward),
    (KeyInput::Ctrl('u'), Action::DeleteToLineStart),
    (KeyInput::Ctrl('t'), Action::IndentCurrentLine),
    (KeyInput::Ctrl('d'), Action::DedentCurrentLine),
];

const COMMAND_SEARCH_SINGLE_BINDINGS: &[(KeyInput, Action)] = &[
    (KeyInput::Escape, Action::CancelCommand),
    (KeyInput::Char('\n'), Action::ExecuteCommand),
    (KeyInput::Up, Action::PromptHistoryPrev),
    (KeyInput::Down, Action::PromptHistoryNext),
    (KeyInput::Ctrl('p'), Action::PromptHistoryPrevFull),
    (KeyInput::Ctrl('n'), Action::PromptHistoryNextFull),
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
        &[KeyInput::Char('g'), KeyInput::Char('e')],
        Action::MoveWordEndBackward,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('E')],
        Action::MoveBigWordEndBackward,
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
        &[KeyInput::Char('g'), KeyInput::Char('d')],
        Action::GotoDefinition,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('r')],
        Action::GotoReferences,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('f')],
        Action::GotoFileUnderCursor,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('F')],
        Action::GotoFileUnderCursorAtPosition,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('a')],
        Action::GotoAlternateFile,
    ),
    (
        &[KeyInput::Char('g'), KeyInput::Char('.')],
        Action::GotoLastModification,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('a')],
        Action::OpenCodeActions,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('d')],
        Action::OpenDiagnosticsPicker,
    ),
    (
        &[KeyInput::Char(']'), KeyInput::Char('d')],
        Action::NextDiagnostic,
    ),
    (
        &[KeyInput::Char('['), KeyInput::Char('d')],
        Action::PrevDiagnostic,
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
    (
        &[KeyInput::Char(' '), KeyInput::Char('f')],
        Action::OpenFilePicker,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('/')],
        Action::PromptGrep,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('*')],
        Action::GrepWordUnderCursor,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('l')],
        Action::HideSearchHighlighting,
    ),
    (
        &[KeyInput::Char(' '), KeyInput::Char('r')],
        Action::PromptRenameSymbol,
    ),
    (
        &[
            KeyInput::Char(' '),
            KeyInput::Char('-'),
            KeyInput::Char('p'),
        ],
        Action::PasteClipboardAfterCursor,
    ),
    (
        &[
            KeyInput::Char(' '),
            KeyInput::Char('-'),
            KeyInput::Char('P'),
        ],
        Action::PasteClipboardBeforeCursor,
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

const OPERATOR_BINDINGS: &[(KeyInput, OperatorBinding)] = &[
    (KeyInput::Char('w'), OperatorBinding::WordForward),
    (KeyInput::Char('W'), OperatorBinding::WordForwardBig),
    (KeyInput::Char('e'), OperatorBinding::WordEnd),
    (KeyInput::Char('E'), OperatorBinding::WordEndBig),
    (KeyInput::Char('b'), OperatorBinding::WordBackward),
    (KeyInput::Char('B'), OperatorBinding::WordBackwardBig),
    (KeyInput::Char('{'), OperatorBinding::ParagraphBackward),
    (KeyInput::Char('}'), OperatorBinding::ParagraphForward),
    (KeyInput::Char('f'), OperatorBinding::FindForward),
    (KeyInput::Char('F'), OperatorBinding::FindBackward),
    (KeyInput::Char('t'), OperatorBinding::TillForward),
    (KeyInput::Char('T'), OperatorBinding::TillBackward),
    (KeyInput::Char('%'), OperatorBinding::MatchDelimiter),
    (KeyInput::Char('i'), OperatorBinding::TextObjectInner),
    (KeyInput::Char('a'), OperatorBinding::TextObjectAround),
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
        register_operator_bindings(&mut bindings, OPERATOR_BINDINGS);
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

/// Register the built-in operator-pending bindings.
fn register_operator_bindings(bindings: &mut KeyBindings, entries: &[(KeyInput, OperatorBinding)]) {
    for (key, binding) in entries {
        bindings.set_operator_binding(key.clone(), *binding);
    }
}
