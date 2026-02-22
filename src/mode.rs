//! Editor mode management
//!
//! The Mode enum represents the current state of the editor, which determines
//! which key bindings are active and how user input is processed.

/// Editor mode enum
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Normal mode - for navigation and commands
    Normal,
    /// Insert mode - for typing text
    Insert,
    /// Command mode - for entering commands (started with ':')
    Command(String),
    /// Search mode - for entering search patterns (started with '/')
    Search(String),
}

#[cfg_attr(not(test), expect(dead_code))]
impl Mode {
    /// Check if the mode is Normal
    pub fn is_normal(&self) -> bool {
        matches!(self, Mode::Normal)
    }

    /// Check if the mode is Insert
    pub fn is_insert(&self) -> bool {
        matches!(self, Mode::Insert)
    }

    /// Check if the mode is Command
    pub fn is_command(&self) -> bool {
        matches!(self, Mode::Command(_))
    }

    /// Check if the mode is Search
    pub fn is_search(&self) -> bool {
        matches!(self, Mode::Search(_))
    }

    /// Get the prompt string for display in the status bar
    pub fn get_prompt(&self) -> String {
        match self {
            Mode::Normal => "NORMAL".to_string(),
            Mode::Insert => "INSERT".to_string(),
            Mode::Command(s) => format!(":{}", s),
            Mode::Search(s) => format!("/{}", s),
        }
    }

    /// Append a character to the Command or Search string
    /// Does nothing if the mode is Normal or Insert
    pub fn append_char(&mut self, c: char) {
        match self {
            Mode::Command(s) | Mode::Search(s) => s.push(c),
            _ => {}
        }
    }

    /// Remove the last character from the Command or Search string
    /// Does nothing if the mode is Normal or Insert
    pub fn pop_char(&mut self) {
        match self {
            Mode::Command(s) | Mode::Search(s) => {
                s.pop();
            }
            _ => {}
        }
    }

    /// Get the command string (for Command mode)
    /// Returns None if not in Command mode
    pub fn command_string(&self) -> Option<&str> {
        match self {
            Mode::Command(s) => Some(s),
            _ => None,
        }
    }

    /// Get the search string (for Search mode)
    /// Returns None if not in Search mode
    pub fn search_string(&self) -> Option<&str> {
        match self {
            Mode::Search(s) => Some(s),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_predicates() {
        let normal = Mode::Normal;
        assert!(normal.is_normal());
        assert!(!normal.is_insert());
        assert!(!normal.is_command());
        assert!(!normal.is_search());

        let insert = Mode::Insert;
        assert!(insert.is_insert());
        assert!(!insert.is_normal());

        let command = Mode::Command("w".to_string());
        assert!(command.is_command());
        assert!(!command.is_normal());

        let search = Mode::Search("test".to_string());
        assert!(search.is_search());
        assert!(!search.is_normal());
    }

    #[test]
    fn test_get_prompt() {
        assert_eq!(Mode::Normal.get_prompt(), "NORMAL");
        assert_eq!(Mode::Insert.get_prompt(), "INSERT");
        assert_eq!(Mode::Command("w".to_string()).get_prompt(), ":w");
        assert_eq!(Mode::Search("hello".to_string()).get_prompt(), "/hello");
    }

    #[test]
    fn test_append_char() {
        let mut mode = Mode::Command(String::new());
        mode.append_char('w');
        assert_eq!(mode.command_string(), Some("w"));

        let mut mode = Mode::Search(String::new());
        mode.append_char('t');
        mode.append_char('e');
        mode.append_char('s');
        mode.append_char('t');
        assert_eq!(mode.search_string(), Some("test"));

        let mut mode = Mode::Normal;
        mode.append_char('x'); // Should do nothing
        assert!(mode.is_normal());
    }

    #[test]
    fn test_pop_char() {
        let mut mode = Mode::Command("test".to_string());
        mode.pop_char();
        assert_eq!(mode.command_string(), Some("tes"));
        mode.pop_char();
        mode.pop_char();
        mode.pop_char();
        assert_eq!(mode.command_string(), Some(""));

        let mut mode = Mode::Normal;
        mode.pop_char(); // Should do nothing
        assert!(mode.is_normal());
    }

    #[test]
    fn test_command_and_search_strings() {
        let command = Mode::Command("save".to_string());
        assert_eq!(command.command_string(), Some("save"));
        assert_eq!(command.search_string(), None);

        let search = Mode::Search("pattern".to_string());
        assert_eq!(search.search_string(), Some("pattern"));
        assert_eq!(search.command_string(), None);

        let normal = Mode::Normal;
        assert_eq!(normal.command_string(), None);
        assert_eq!(normal.search_string(), None);
    }

    #[test]
    fn test_default_mode() {
        let mode = Mode::Normal;
        assert!(mode.is_normal());
    }
}
