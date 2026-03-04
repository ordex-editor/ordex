//! Key bindings configuration
//!
//! This module provides a mapping from key inputs to editor actions.
//! The configuration is read-only during editor session, allowing for
//! future file-based configuration support.

use crate::mode::Mode;
use std::collections::HashMap;
use termion::event::Key;

/// Actions that can be triggered by key bindings
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
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
            Key::Up => KeyInput::Up,
            Key::Down => KeyInput::Down,
            Key::Left => KeyInput::Left,
            Key::Right => KeyInput::Right,
            Key::Home => KeyInput::Home,
            Key::End => KeyInput::End,
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
                return SequenceMatch::Exact(binding.action.clone());
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
}
