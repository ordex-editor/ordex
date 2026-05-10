//! Configuration parsers for keybinding modes, keys, sequences, and actions.

use super::{Action, KeyInput, ModeContext, OperatorBinding};

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

/// Parse a textual operator binding name from configuration.
pub(crate) fn parse_operator_binding(input: &str) -> Option<OperatorBinding> {
    match input.trim() {
        "word-forward" => Some(OperatorBinding::WordForward),
        "big-word-forward" => Some(OperatorBinding::WordForwardBig),
        "word-end" => Some(OperatorBinding::WordEnd),
        "big-word-end" => Some(OperatorBinding::WordEndBig),
        "word-backward" => Some(OperatorBinding::WordBackward),
        "big-word-backward" => Some(OperatorBinding::WordBackwardBig),
        "paragraph-forward" => Some(OperatorBinding::ParagraphForward),
        "paragraph-backward" => Some(OperatorBinding::ParagraphBackward),
        "find-forward" => Some(OperatorBinding::FindForward),
        "find-backward" => Some(OperatorBinding::FindBackward),
        "till-forward" => Some(OperatorBinding::TillForward),
        "till-backward" => Some(OperatorBinding::TillBackward),
        "jump-to-matching-delimiter" => Some(OperatorBinding::MatchDelimiter),
        "text-object-inner" => Some(OperatorBinding::TextObjectInner),
        "text-object-around" => Some(OperatorBinding::TextObjectAround),
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

    // Prefer named modified keys before falling back to character modifiers.
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
        "tab" => Some(KeyInput::Ctrl('i')),
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

    // Modified navigation keys map to dedicated runtime variants.
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

    // Reject malformed modifier syntax before falling back to raw character sequences.
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
        // Navigation actions.
        "move-left" => Some(Action::MoveLeft),
        "move-right" => Some(Action::MoveRight),
        "move-up" => Some(Action::MoveUp),
        "move-down" => Some(Action::MoveDown),
        "move-down-first-non-blank" => Some(Action::MoveDownFirstNonBlank),
        "move-word-forward" => Some(Action::MoveWordForward),
        "move-big-word-forward" => Some(Action::MoveBigWordForward),
        "move-word-backward" => Some(Action::MoveWordBackward),
        "move-big-word-backward" => Some(Action::MoveBigWordBackward),
        "move-word-end" => Some(Action::MoveWordEnd),
        "move-big-word-end" => Some(Action::MoveBigWordEnd),
        "move-word-end-backward" => Some(Action::MoveWordEndBackward),
        "move-big-word-end-backward" => Some(Action::MoveBigWordEndBackward),
        "move-paragraph-forward" => Some(Action::MoveParagraphForward),
        "move-paragraph-backward" => Some(Action::MoveParagraphBackward),
        "move-line-start" => Some(Action::MoveLineStart),
        "move-line-end" => Some(Action::MoveLineEnd),
        "move-past-line-end" => Some(Action::MovePastLineEnd),
        "move-first-non-blank" => Some(Action::MoveFirstNonBlank),
        "move-to-first-line" => Some(Action::MoveToFirstLine),
        "move-to-last-line" => Some(Action::MoveToLastLine),
        "align-viewport-top" => Some(Action::AlignViewportTop),
        "align-viewport-center" => Some(Action::AlignViewportCenter),
        "align-viewport-bottom" => Some(Action::AlignViewportBottom),
        "scroll-line-up" => Some(Action::ScrollLineUp),
        "scroll-line-down" => Some(Action::ScrollLineDown),
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
        "repeat-last-change" => Some(Action::RepeatLastChange),
        "jump-older" => Some(Action::JumpOlder),
        "jump-newer" => Some(Action::JumpNewer),
        "jump-to-matching-delimiter" => Some(Action::MatchBracket),
        "goto-definition" => Some(Action::GotoDefinition),
        "goto-references" => Some(Action::GotoReferences),
        "goto-file-under-cursor" => Some(Action::GotoFileUnderCursor),
        "goto-file-under-cursor-at-position" => Some(Action::GotoFileUnderCursorAtPosition),
        "goto-alternate-file" => Some(Action::GotoAlternateFile),
        "goto-last-modification" => Some(Action::GotoLastModification),
        "show-hover" => Some(Action::ShowHover),
        "open-code-actions" => Some(Action::OpenCodeActions),
        "open-diagnostics-picker" => Some(Action::OpenDiagnosticsPicker),
        "next-diagnostic" => Some(Action::NextDiagnostic),
        "prev-diagnostic" => Some(Action::PrevDiagnostic),
        "prompt-rename-symbol" => Some(Action::PromptRenameSymbol),
        "begin-macro-record" => Some(Action::BeginMacroRecord),
        "begin-macro-playback" => Some(Action::BeginMacroPlayback),
        // Mode and file actions.
        "enter-insert-mode" => Some(Action::EnterInsertMode),
        "enter-visual-mode" => Some(Action::EnterVisualMode),
        "enter-visual-line-mode" => Some(Action::EnterVisualLineMode),
        "swap-visual-anchor" => Some(Action::SwapVisualAnchor),
        "recreate-last-selection" => Some(Action::RecreateLastSelection),
        "insert-after-cursor" => Some(Action::InsertAfterCursor),
        "open-line-below" => Some(Action::OpenLineBelow),
        "open-line-above" => Some(Action::OpenLineAbove),
        "enter-command-mode" => Some(Action::EnterCommandMode),
        "enter-search-mode" => Some(Action::EnterSearchMode),
        "open-buffer-switcher" => Some(Action::OpenBufferSwitcher),
        "open-file-picker" => Some(Action::OpenFilePicker),
        "exit-to-normal-mode" => Some(Action::ExitToNormalMode),
        "hide-search-highlighting" => Some(Action::HideSearchHighlighting),
        "search-next" => Some(Action::SearchNext),
        "search-previous" => Some(Action::SearchPrevious),
        "undo" => Some(Action::Undo),
        "redo" => Some(Action::Redo),
        "save-current-file" => Some(Action::SaveCurrentFile),
        "save-current-file-and-quit" => Some(Action::SaveCurrentFileAndQuit),
        "update-current-file-and-quit" => Some(Action::UpdateCurrentFileAndQuit),
        "request-full-redraw" => Some(Action::RequestFullRedraw),
        // Editing actions.
        "toggle-case-at-cursor" => Some(Action::ToggleCaseAtCursor),
        "delete-to-line-end" => Some(Action::DeleteToLineEnd),
        "change-to-line-end" => Some(Action::ChangeToLineEnd),
        "increment-next-number" => Some(Action::IncrementNextNumber),
        "decrement-next-number" => Some(Action::DecrementNextNumber),
        "join-lines" => Some(Action::JoinLines),
        "begin-replace-char" => Some(Action::BeginReplaceChar),
        "search-word-under-cursor" => Some(Action::SearchWordUnderCursor),
        "delete-char-backward" => Some(Action::DeleteCharBackward),
        "delete-char-forward" => Some(Action::DeleteCharForward),
        "completion-select-up" => Some(Action::CompletionSelectUp),
        "completion-select-down" => Some(Action::CompletionSelectDown),
        "delete-char-at-cursor" => Some(Action::DeleteCharAtCursor),
        "delete-word-backward" => Some(Action::DeleteWordBackward),
        "delete-to-line-start" => Some(Action::DeleteToLineStart),
        "insert-newline" => Some(Action::InsertNewline),
        "delete-selection" => Some(Action::DeleteSelection),
        "indent-selection" => Some(Action::IndentSelection),
        "change-selection" => Some(Action::ChangeSelection),
        "yank-selection" => Some(Action::YankSelection),
        "yank-current-line" => Some(Action::YankCurrentLine),
        "paste-after-cursor" => Some(Action::PasteAfterCursor),
        "paste-before-cursor" => Some(Action::PasteBeforeCursor),
        "begin-delete-operator" => Some(Action::BeginDeleteOperator),
        "begin-change-operator" => Some(Action::BeginChangeOperator),
        "begin-yank-operator" => Some(Action::BeginYankOperator),
        "begin-indent-operator" => Some(Action::BeginIndentOperator),
        "indent-current-line" => Some(Action::IndentCurrentLine),
        "dedent-current-line" => Some(Action::DedentCurrentLine),
        // Command and search input actions.
        "execute-command" => Some(Action::ExecuteCommand),
        "cancel-command" => Some(Action::CancelCommand),
        "prompt-history-prev" => Some(Action::PromptHistoryPrev),
        "prompt-history-next" => Some(Action::PromptHistoryNext),
        "prompt-history-prev-full" => Some(Action::PromptHistoryPrevFull),
        "prompt-history-next-full" => Some(Action::PromptHistoryNextFull),
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
