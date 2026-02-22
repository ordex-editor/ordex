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
#[expect(dead_code)]
pub enum Action {
    // Navigation actions
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordForward,
    MoveWordBackward,
    MoveWordEnd,
    MoveLineStart,
    MoveLineEnd,
    MovePastLineEnd,
    MoveFirstNonBlank,
    MoveToFirstLine,
    MoveToLastLine,
    PageUp,
    PageDown,

    // Mode switching
    EnterInsertMode,
    EnterCommandMode,
    EnterSearchMode,
    ExitToNormalMode,

    // Insert mode actions (parameterized actions handled specially)
    DeleteCharBackward,
    DeleteCharForward,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    InsertNewline,

    // Command/Search mode actions
    ExecuteCommand,
    CancelCommand,
    DeleteInputChar,

    // File operations
    SaveFile,

    // Editor control
    Quit,
}

/// Wrapper for Key that implements Hash (termion's Key doesn't implement Hash)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyInput {
    Char(char),
    Ctrl(char),
    Alt(char),
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
            _ => KeyInput::Escape, // Fallback for unsupported keys
        }
    }
}

/// Mode context for key binding lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModeContext {
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
pub struct KeyBindings {
    /// Bindings for each mode: (ModeContext, KeyInput) -> Action
    bindings: HashMap<(ModeContext, KeyInput), Action>,
}

impl KeyBindings {
    /// Create default key bindings
    pub fn new() -> Self {
        let mut bindings = HashMap::new();

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
            KeyInput::Char('i'),
            Action::EnterInsertMode,
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
            KeyInput::Char('G'),
            Action::MoveToLastLine,
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

        Self { bindings }
    }

    fn add_binding(
        bindings: &mut HashMap<(ModeContext, KeyInput), Action>,
        mode: ModeContext,
        key: KeyInput,
        action: Action,
    ) {
        bindings.insert((mode, key), action);
    }

    /// Get the action for a key press in the given mode
    /// Returns None if no binding exists (caller should handle specially for insert/command modes)
    pub fn get_action(&self, key: Key, mode: &Mode) -> Option<Action> {
        let context = ModeContext::from(mode);
        let key_input = KeyInput::from(key);
        self.bindings.get(&(context, key_input)).cloned()
    }

    /// Check if a key is a character that should be inserted/appended in the current mode
    /// This handles the case where typed characters aren't in the bindings map
    pub fn is_insertable_char(key: Key) -> Option<char> {
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
    }

    #[test]
    fn test_normal_mode_enter_insert() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        assert_eq!(
            bindings.get_action(Key::Char('i'), &mode),
            Some(Action::EnterInsertMode)
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
        let mode = Mode::Command(String::new());

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
    }

    #[test]
    fn test_search_mode() {
        let bindings = KeyBindings::new();
        let mode = Mode::Search(String::new());

        assert_eq!(
            bindings.get_action(Key::Char('\n'), &mode),
            Some(Action::ExecuteCommand)
        );
        assert_eq!(
            bindings.get_action(Key::Esc, &mode),
            Some(Action::CancelCommand)
        );
    }

    #[test]
    fn test_unbound_key_returns_none() {
        let bindings = KeyBindings::new();
        let mode = Mode::Normal;

        // 'z' is not bound in normal mode
        assert_eq!(bindings.get_action(Key::Char('z'), &mode), None);
    }
}
