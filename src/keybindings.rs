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

/// Result of matching a typed key sequence against configured multi-key bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SequenceMatch {
    /// Sequence fully matches a binding and should execute the action now.
    Exact(Action),
    /// Sequence is a valid prefix; wait for additional keys.
    Prefix,
    /// Sequence doesn't match any configured multi-key binding.
    NoMatch,
}

/// Multi-key sequence binding.
#[derive(Debug, Clone)]
struct SequenceBinding {
    mode: ModeContext,
    keys: Vec<KeyInput>,
    action: Action,
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

/// Mode context for key binding lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ModeContext {
    Normal,
    Insert,
    Command,
    Search,
}

impl From<&Mode> for ModeContext {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Normal => ModeContext::Normal,
            Mode::Insert => ModeContext::Insert,
            Mode::Command(_) => ModeContext::Command,
            Mode::Search(_) => ModeContext::Search,
        }
    }
}

/// Key bindings configuration
/// Uses HashMaps to store bindings, making it easy to load from config file later
pub(crate) struct KeyBindings {
    /// Bindings for each mode: (ModeContext, KeyInput) -> Action
    bindings: HashMap<(ModeContext, KeyInput), Action>,
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
        bindings: &mut HashMap<(ModeContext, KeyInput), Action>,
        mode: ModeContext,
        key: KeyInput,
        action: Action,
    ) {
        bindings.insert((mode, key), action);
    }

    fn add_sequence_binding(
        sequence_bindings: &mut Vec<SequenceBinding>,
        mode: ModeContext,
        keys: Vec<KeyInput>,
        action: Action,
    ) {
        sequence_bindings.push(SequenceBinding { mode, keys, action });
    }

    /// Get the action for a key press in the given mode
    /// Returns None if no binding exists (caller should handle specially for insert/command modes)
    pub(crate) fn get_action(&self, key: Key, mode: &Mode) -> Option<Action> {
        let context = ModeContext::from(mode);
        let key_input = KeyInput::from(key);
        self.bindings.get(&(context, key_input)).cloned()
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
                return SequenceMatch::Exact(binding.action);
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

    /// Override or add a key binding at runtime.
    pub(crate) fn set_binding(&mut self, mode: ModeContext, key: KeyInput, action: Action) {
        self.bindings.insert((mode, key), action);
    }

    /// Override or add a multi-key sequence binding at runtime.
    pub(crate) fn set_sequence_binding(
        &mut self,
        mode: ModeContext,
        keys: Vec<KeyInput>,
        action: Action,
    ) {
        if keys.len() == 1 {
            if let Some(key) = keys.first() {
                self.set_binding(mode, key.clone(), action);
            }
            return;
        }
        self.sequence_bindings
            .retain(|binding| !(binding.mode == mode && binding.keys == keys));
        self.sequence_bindings
            .push(SequenceBinding { mode, keys, action });
    }
}

/// Parse a configuration mode name into a runtime mode context.
pub(crate) fn parse_mode_context(input: &str) -> Option<ModeContext> {
    match input.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(ModeContext::Normal),
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
    let key = normalized.to_ascii_lowercase().replace(['_', '-'], "");
    match key.as_str() {
        "moveleft" => Some(Action::MoveLeft),
        "moveright" => Some(Action::MoveRight),
        "moveup" => Some(Action::MoveUp),
        "movedown" => Some(Action::MoveDown),
        "movewordforward" => Some(Action::MoveWordForward),
        "movewordbackward" => Some(Action::MoveWordBackward),
        "movewordend" => Some(Action::MoveWordEnd),
        "moveparagraphforward" => Some(Action::MoveParagraphForward),
        "moveparagraphbackward" => Some(Action::MoveParagraphBackward),
        "movelinestart" => Some(Action::MoveLineStart),
        "movelineend" => Some(Action::MoveLineEnd),
        "movepastlineend" => Some(Action::MovePastLineEnd),
        "movefirstnonblank" => Some(Action::MoveFirstNonBlank),
        "movetofirstline" => Some(Action::MoveToFirstLine),
        "movetolastline" => Some(Action::MoveToLastLine),
        "pageup" => Some(Action::PageUp),
        "pagedown" => Some(Action::PageDown),
        "halfpageup" => Some(Action::HalfPageUp),
        "halfpagedown" => Some(Action::HalfPageDown),
        "findforward" => Some(Action::FindForward),
        "findbackward" => Some(Action::FindBackward),
        "tillforward" => Some(Action::TillForward),
        "tillbackward" => Some(Action::TillBackward),
        "repeatfindforward" => Some(Action::RepeatFindForward),
        "repeatfindbackward" => Some(Action::RepeatFindBackward),
        "enterinsertmode" => Some(Action::EnterInsertMode),
        "insertaftercursor" => Some(Action::InsertAfterCursor),
        "openlinebelow" => Some(Action::OpenLineBelow),
        "openlineabove" => Some(Action::OpenLineAbove),
        "entercommandmode" => Some(Action::EnterCommandMode),
        "entersearchmode" => Some(Action::EnterSearchMode),
        "exittonormalmode" => Some(Action::ExitToNormalMode),
        "searchnext" => Some(Action::SearchNext),
        "searchprevious" => Some(Action::SearchPrevious),
        "savecurrentfile" => Some(Action::SaveCurrentFile),
        "savecurrentfileandquit" => Some(Action::SaveCurrentFileAndQuit),
        "deletecharbackward" => Some(Action::DeleteCharBackward),
        "deletecharforward" => Some(Action::DeleteCharForward),
        "deletecharatcursor" => Some(Action::DeleteCharAtCursor),
        "deletewordbackward" => Some(Action::DeleteWordBackward),
        "deletetolinestart" => Some(Action::DeleteToLineStart),
        "insertnewline" => Some(Action::InsertNewline),
        "changeinnerword" => Some(Action::ChangeInnerWord),
        "deleteinnerword" => Some(Action::DeleteInnerWord),
        "deletearoundparen" => Some(Action::DeleteAroundParen),
        "executecommand" => Some(Action::ExecuteCommand),
        "cancelcommand" => Some(Action::CancelCommand),
        "deleteinputchar" => Some(Action::DeleteInputChar),
        "deleteinputcharforward" => Some(Action::DeleteInputCharForward),
        "deleteinputwordbackward" => Some(Action::DeleteInputWordBackward),
        "deleteinputtostart" => Some(Action::DeleteInputToStart),
        "deleteinputtoend" => Some(Action::DeleteInputToEnd),
        "moveinputstart" => Some(Action::MoveInputStart),
        "moveinputend" => Some(Action::MoveInputEnd),
        "moveinputleft" => Some(Action::MoveInputLeft),
        "moveinputright" => Some(Action::MoveInputRight),
        "moveinputwordleft" => Some(Action::MoveInputWordLeft),
        "moveinputwordright" => Some(Action::MoveInputWordRight),
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
    fn test_sequence_gg_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('g')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(Action::MoveToFirstLine)
        );
    }

    #[test]
    fn test_sequence_g_dollar_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('$')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(Action::MoveLineEnd)
        );
    }

    #[test]
    fn test_sequence_g_zero_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char('g'), KeyInput::Char('0')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(Action::MoveLineStart)
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
            SequenceMatch::Exact(Action::ChangeInnerWord)
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
            SequenceMatch::Exact(Action::DeleteInnerWord)
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
            SequenceMatch::Exact(Action::DeleteAroundParen)
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
            SequenceMatch::Exact(Action::SaveCurrentFile)
        );
    }

    #[test]
    fn test_sequence_space_q_exact() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;
        let sequence = vec![KeyInput::Char(' '), KeyInput::Char('q')];

        assert_eq!(
            bindings.match_sequence(&mode, &sequence),
            SequenceMatch::Exact(Action::SaveCurrentFileAndQuit)
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
}
