//! Command mode handling module
//!
//! Manages vim-style colon commands

use std::io;

/// Command mode state
///
/// Tracks the command buffer and provides command parsing
pub struct CommandMode {
    buffer: String,
    active: bool,
}

impl CommandMode {
    /// Create a new command mode instance
    pub fn new() -> Self {
        CommandMode {
            buffer: String::new(),
            active: false,
        }
    }

    /// Activate command mode (when ':' is pressed)
    pub fn activate(&mut self) {
        self.active = true;
        self.buffer.clear();
    }

    /// Deactivate command mode (when Escape is pressed)
    pub fn cancel(&mut self) {
        self.active = false;
        self.buffer.clear();
    }

    /// Check if command mode is active
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Append character to command buffer
    pub fn push_char(&mut self, c: char) {
        self.buffer.push(c);
    }

    /// Remove last character from command buffer (backspace)
    pub fn pop_char(&mut self) {
        self.buffer.pop();
    }

    /// Get current command buffer for display
    #[cfg(test)]
    pub fn get_buffer(&self) -> &str {
        &self.buffer
    }

    /// Execute the current command and return result
    ///
    /// Returns Ok(true) if should quit, Ok(false) otherwise
    pub fn execute(&mut self) -> io::Result<CommandResult> {
        let cmd = self.buffer.trim();

        let result = match cmd {
            "q" => CommandResult::Quit,
            "" => CommandResult::Continue, // Empty command does nothing
            _ => CommandResult::Error(format!("Unknown command: {}", cmd)),
        };

        self.active = false;
        self.buffer.clear();

        Ok(result)
    }

    /// Render command line at bottom of terminal
    pub fn render(&self, term: &mut crate::tui::Terminal, terminal_height: u16) -> io::Result<()> {
        if self.active {
            let display = format!(":{}", self.buffer);
            term.write_at(1, terminal_height, &display)?;
        }
        Ok(())
    }
}

/// Result of executing a command
pub enum CommandResult {
    /// Continue normal operation
    Continue,
    /// Exit the program
    Quit,
    /// Command error with message
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_mode_activation() {
        let mut cmd = CommandMode::new();
        assert!(!cmd.is_active());

        cmd.activate();
        assert!(cmd.is_active());
        assert_eq!(cmd.get_buffer(), "");
    }

    #[test]
    fn test_command_mode_cancel() {
        let mut cmd = CommandMode::new();
        cmd.activate();
        cmd.push_char('q');

        cmd.cancel();
        assert!(!cmd.is_active());
        assert_eq!(cmd.get_buffer(), "");
    }

    #[test]
    fn test_push_char() {
        let mut cmd = CommandMode::new();
        cmd.activate();

        cmd.push_char('q');
        assert_eq!(cmd.get_buffer(), "q");

        cmd.push_char('u');
        assert_eq!(cmd.get_buffer(), "qu");
    }

    #[test]
    fn test_pop_char() {
        let mut cmd = CommandMode::new();
        cmd.activate();

        cmd.push_char('q');
        cmd.push_char('u');
        cmd.push_char('i');
        assert_eq!(cmd.get_buffer(), "qui");

        cmd.pop_char();
        assert_eq!(cmd.get_buffer(), "qu");

        cmd.pop_char();
        assert_eq!(cmd.get_buffer(), "q");

        // Popping from empty buffer should not panic
        cmd.pop_char();
        cmd.pop_char();
        assert_eq!(cmd.get_buffer(), "");
    }

    #[test]
    fn test_execute_quit() {
        let mut cmd = CommandMode::new();
        cmd.activate();
        cmd.push_char('q');

        let result = cmd.execute().unwrap();
        assert!(matches!(result, CommandResult::Quit));
        assert!(!cmd.is_active());
    }

    #[test]
    fn test_execute_unknown() {
        let mut cmd = CommandMode::new();
        cmd.activate();
        cmd.push_char('x');

        let result = cmd.execute().unwrap();
        match result {
            CommandResult::Error(msg) => assert!(msg.contains("Unknown command")),
            _ => panic!("Expected error result"),
        }
        assert!(!cmd.is_active());
    }

    #[test]
    fn test_execute_empty() {
        let mut cmd = CommandMode::new();
        cmd.activate();

        let result = cmd.execute().unwrap();
        assert!(matches!(result, CommandResult::Continue));
        assert!(!cmd.is_active());
    }
}
