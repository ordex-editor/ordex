//! Editor mode management
//!
//! The Mode enum represents the current state of the editor, which determines
//! which key bindings are active and how user input is processed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Whitespace,
    Keyword,
    Punctuation,
}

/// Editable command/search input buffer with an in-line cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputBuffer {
    text: String,
    cursor: usize,
}

impl InputBuffer {
    pub(crate) fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_text(text: String) -> Self {
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn into_text(self) -> String {
        self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn insert_char(&mut self, c: char) {
        let byte_idx = Self::char_to_byte_idx(&self.text, self.cursor);
        self.text.insert(byte_idx, c);
        self.cursor += 1;
    }

    pub(crate) fn move_start(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    pub(crate) fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub(crate) fn move_right(&mut self) {
        let len = self.text.chars().count();
        if self.cursor < len {
            self.cursor += 1;
        }
    }

    pub(crate) fn move_word_left(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut idx = self.cursor.min(chars.len());

        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }

        if idx == 0 {
            self.cursor = 0;
            return;
        }

        let class = Self::word_class(chars[idx - 1]);
        while idx > 0 && Self::word_class(chars[idx - 1]) == class {
            idx -= 1;
        }

        self.cursor = idx;
    }

    pub(crate) fn move_word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut idx = self.cursor.min(chars.len());

        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }

        if idx >= chars.len() {
            self.cursor = chars.len();
            return;
        }

        let class = Self::word_class(chars[idx]);
        while idx < chars.len() && Self::word_class(chars[idx]) == class {
            idx += 1;
        }

        self.cursor = idx;
    }

    pub(crate) fn delete_backward_char(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let end = Self::char_to_byte_idx(&self.text, self.cursor);
        let start = Self::char_to_byte_idx(&self.text, self.cursor - 1);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub(crate) fn delete_forward_char(&mut self) {
        let len = self.text.chars().count();
        if self.cursor >= len {
            return;
        }

        let start = Self::char_to_byte_idx(&self.text, self.cursor);
        let end = Self::char_to_byte_idx(&self.text, self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    pub(crate) fn delete_word_backward(&mut self) {
        let start_idx = self.cursor;
        self.move_word_left();
        let end_idx = start_idx.min(self.text.chars().count());
        let start_idx = self.cursor.min(end_idx);

        if start_idx == end_idx {
            return;
        }

        let start_byte = Self::char_to_byte_idx(&self.text, start_idx);
        let end_byte = Self::char_to_byte_idx(&self.text, end_idx);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start_idx;
    }

    /// Delete one word forward from the current input cursor position.
    pub(crate) fn delete_word_forward(&mut self) {
        let start_idx = self.cursor.min(self.text.chars().count());
        let mut end_cursor = Self {
            text: self.text.clone(),
            cursor: start_idx,
        };
        // Reuse the existing word-motion rules so forward deletion matches the
        // same boundaries that Alt-f uses in picker and command prompts.
        end_cursor.move_word_right();
        let end_idx = end_cursor.cursor.min(self.text.chars().count());

        if start_idx == end_idx {
            return;
        }

        let start_byte = Self::char_to_byte_idx(&self.text, start_idx);
        let end_byte = Self::char_to_byte_idx(&self.text, end_idx);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start_idx;
    }

    pub(crate) fn delete_to_start(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let end = Self::char_to_byte_idx(&self.text, self.cursor);
        self.text.replace_range(0..end, "");
        self.cursor = 0;
    }

    pub(crate) fn delete_to_end(&mut self) {
        let len = self.text.chars().count();
        if self.cursor >= len {
            return;
        }

        let start = Self::char_to_byte_idx(&self.text, self.cursor);
        self.text.replace_range(start..self.text.len(), "");
    }

    fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
        if char_idx == 0 {
            return 0;
        }

        text.char_indices()
            .nth(char_idx)
            .map(|(idx, _)| idx)
            .unwrap_or(text.len())
    }

    fn word_class(c: char) -> WordClass {
        if c.is_whitespace() {
            WordClass::Whitespace
        } else if c.is_alphanumeric() || c == '_' {
            WordClass::Keyword
        } else {
            WordClass::Punctuation
        }
    }
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Visual-selection variants supported by the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualKind {
    /// Select from the anchor to the active cursor using character endpoints.
    Character,
    /// Select complete logical lines between the anchor and the active cursor.
    Line,
}

impl VisualKind {
    /// Return the stable status-bar label for this visual mode.
    pub(crate) fn mode_label(self) -> &'static str {
        match self {
            Self::Character => "VISUAL",
            Self::Line => "V-LINE",
        }
    }
}

/// Editor mode enum
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Normal mode - for navigation and commands
    Normal,
    /// Visual mode - for selecting text with normal-mode motions
    Visual(VisualKind),
    /// Insert mode - for typing text
    Insert,
    /// Command mode - for entering commands (started with ':')
    Command(InputBuffer),
    /// Search mode - for entering search patterns (started with '/')
    Search(InputBuffer),
    /// Buffer-switch mode - for filtering and selecting another open buffer
    BufferSwitch(InputBuffer),
    /// File-picker mode - for filtering and opening a file from disk
    FilePicker(InputBuffer),
}

impl Mode {
    pub(crate) fn command_empty() -> Self {
        Self::Command(InputBuffer::new())
    }

    pub(crate) fn search_empty() -> Self {
        Self::Search(InputBuffer::new())
    }

    /// Create buffer-switch mode with an empty filter.
    pub(crate) fn buffer_switch_empty() -> Self {
        Self::BufferSwitch(InputBuffer::new())
    }

    /// Create file-picker mode with an empty filter.
    pub(crate) fn file_picker_empty() -> Self {
        Self::FilePicker(InputBuffer::new())
    }

    #[cfg(test)]
    /// Create characterwise visual mode.
    pub(crate) fn visual_character() -> Self {
        Self::Visual(VisualKind::Character)
    }

    #[cfg(test)]
    /// Create linewise visual mode.
    pub(crate) fn visual_line() -> Self {
        Self::Visual(VisualKind::Line)
    }

    #[cfg(test)]
    pub(crate) fn command_with_text(text: impl Into<String>) -> Self {
        Self::Command(InputBuffer::from_text(text.into()))
    }

    #[cfg(test)]
    pub(crate) fn search_with_text(text: impl Into<String>) -> Self {
        Self::Search(InputBuffer::from_text(text.into()))
    }

    /// Get the stable mode label used in status/logging output.
    pub(crate) fn mode_label(&self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Visual(kind) => kind.mode_label(),
            Mode::Insert => "INSERT",
            Mode::Command(_) => "COMMAND",
            Mode::Search(_) => "SEARCH",
            // Buffer switching should stay visually transparent in the status bar
            // so the user keeps the same normal-mode context while the overlay is open.
            Mode::BufferSwitch(_) | Mode::FilePicker(_) => "NORMAL",
        }
    }

    /// Check if the mode is Normal
    pub(crate) fn is_normal(&self) -> bool {
        matches!(self, Mode::Normal)
    }

    /// Check if the mode is Visual.
    pub(crate) fn is_visual(&self) -> bool {
        matches!(self, Mode::Visual(_))
    }

    /// Check if the mode is Insert
    #[cfg(test)]
    pub(crate) fn is_insert(&self) -> bool {
        matches!(self, Mode::Insert)
    }

    /// Return whether this mode should use the terminal beam cursor.
    pub(crate) fn uses_beam_cursor(&self) -> bool {
        matches!(
            self,
            Mode::Insert
                | Mode::Command(_)
                | Mode::Search(_)
                | Mode::BufferSwitch(_)
                | Mode::FilePicker(_)
        )
    }

    /// Check if the mode is Command
    #[cfg(test)]
    pub(crate) fn is_command(&self) -> bool {
        matches!(self, Mode::Command(_))
    }

    /// Check if the mode is Search
    #[cfg(test)]
    pub(crate) fn is_search(&self) -> bool {
        matches!(self, Mode::Search(_))
    }

    /// Get the prompt string for display in the status bar
    #[cfg(test)]
    pub(crate) fn get_prompt(&self) -> String {
        match self {
            Mode::Normal => "NORMAL".to_string(),
            Mode::Visual(kind) => kind.mode_label().to_string(),
            Mode::Insert => "INSERT".to_string(),
            Mode::Command(input) => format!(":{}", input.text()),
            Mode::Search(input) => format!("/{}", input.text()),
            Mode::BufferSwitch(input) => format!(">{}", input.text()),
            Mode::FilePicker(input) => format!(">{}", input.text()),
        }
    }

    /// Append a character to the Command or Search input.
    /// Does nothing if the mode is Normal or Insert.
    pub(crate) fn append_char(&mut self, c: char) {
        if let Some(input) = self.input_mut() {
            input.insert_char(c);
        }
    }

    /// Remove the character immediately before the input cursor.
    /// Does nothing if the mode is Normal or Insert.
    pub(crate) fn pop_char(&mut self) {
        if let Some(input) = self.input_mut() {
            input.delete_backward_char();
        }
    }

    pub(crate) fn move_input_start(&mut self) {
        if let Some(input) = self.input_mut() {
            input.move_start();
        }
    }

    pub(crate) fn move_input_end(&mut self) {
        if let Some(input) = self.input_mut() {
            input.move_end();
        }
    }

    pub(crate) fn move_input_left(&mut self) {
        if let Some(input) = self.input_mut() {
            input.move_left();
        }
    }

    pub(crate) fn move_input_right(&mut self) {
        if let Some(input) = self.input_mut() {
            input.move_right();
        }
    }

    pub(crate) fn move_input_word_left(&mut self) {
        if let Some(input) = self.input_mut() {
            input.move_word_left();
        }
    }

    pub(crate) fn move_input_word_right(&mut self) {
        if let Some(input) = self.input_mut() {
            input.move_word_right();
        }
    }

    pub(crate) fn delete_input_char_forward(&mut self) {
        if let Some(input) = self.input_mut() {
            input.delete_forward_char();
        }
    }

    pub(crate) fn delete_input_word_backward(&mut self) {
        if let Some(input) = self.input_mut() {
            input.delete_word_backward();
        }
    }

    /// Delete one input word forward while keeping the cursor in place.
    pub(crate) fn delete_input_word_forward(&mut self) {
        if let Some(input) = self.input_mut() {
            input.delete_word_forward();
        }
    }

    pub(crate) fn delete_input_to_start(&mut self) {
        if let Some(input) = self.input_mut() {
            input.delete_to_start();
        }
    }

    pub(crate) fn delete_input_to_end(&mut self) {
        if let Some(input) = self.input_mut() {
            input.delete_to_end();
        }
    }

    pub(crate) fn input_cursor(&self) -> Option<usize> {
        self.input().map(InputBuffer::cursor)
    }

    /// Get the command string (for Command mode)
    /// Returns None if not in Command mode
    pub(crate) fn command_string(&self) -> Option<&str> {
        match self {
            Mode::Command(input) => Some(input.text()),
            _ => None,
        }
    }

    /// Get the search string (for Search mode)
    /// Returns None if not in Search mode
    pub(crate) fn search_string(&self) -> Option<&str> {
        match self {
            Mode::Search(input) => Some(input.text()),
            _ => None,
        }
    }

    /// Get the active picker query shared by buffer switching and file opening.
    pub(crate) fn picker_string(&self) -> Option<&str> {
        match self {
            Mode::BufferSwitch(input) | Mode::FilePicker(input) => Some(input.text()),
            _ => None,
        }
    }

    /// Get the active file-picker query.
    ///
    /// Returns `None` when the editor is not in file-picker mode.
    pub(crate) fn file_picker_string(&self) -> Option<&str> {
        match self {
            Mode::FilePicker(input) => Some(input.text()),
            _ => None,
        }
    }

    pub(crate) fn take_command_input(&mut self) -> Option<String> {
        match self {
            Mode::Command(_) => {
                let mode = std::mem::replace(self, Mode::Normal);
                if let Mode::Command(input) = mode {
                    Some(input.into_text())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn take_search_input(&mut self) -> Option<String> {
        match self {
            Mode::Search(_) => {
                let mode = std::mem::replace(self, Mode::Normal);
                if let Mode::Search(input) = mode {
                    Some(input.into_text())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn input(&self) -> Option<&InputBuffer> {
        match self {
            Mode::Command(input)
            | Mode::Search(input)
            | Mode::BufferSwitch(input)
            | Mode::FilePicker(input) => Some(input),
            _ => None,
        }
    }

    fn input_mut(&mut self) -> Option<&mut InputBuffer> {
        match self {
            Mode::Command(input)
            | Mode::Search(input)
            | Mode::BufferSwitch(input)
            | Mode::FilePicker(input) => Some(input),
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
        assert!(!normal.is_visual());
        assert!(!normal.is_insert());
        assert!(!normal.is_command());
        assert!(!normal.is_search());

        let visual = Mode::visual_character();
        assert!(visual.is_visual());
        assert!(!visual.is_normal());

        let insert = Mode::Insert;
        assert!(insert.is_insert());
        assert!(!insert.is_normal());

        let command = Mode::command_with_text("w");
        assert!(command.is_command());
        assert!(!command.is_normal());

        let search = Mode::search_with_text("test");
        assert!(search.is_search());
        assert!(!search.is_normal());
    }

    #[test]
    fn test_get_prompt() {
        assert_eq!(Mode::Normal.get_prompt(), "NORMAL");
        assert_eq!(Mode::visual_character().get_prompt(), "VISUAL");
        assert_eq!(Mode::visual_line().get_prompt(), "V-LINE");
        assert_eq!(Mode::Insert.get_prompt(), "INSERT");
        assert_eq!(Mode::command_with_text("w").get_prompt(), ":w");
        assert_eq!(Mode::search_with_text("hello").get_prompt(), "/hello");
        assert_eq!(Mode::buffer_switch_empty().get_prompt(), ">");
    }

    #[test]
    fn test_append_char() {
        let mut mode = Mode::command_empty();
        mode.append_char('w');
        assert_eq!(mode.command_string(), Some("w"));

        let mut mode = Mode::search_empty();
        mode.append_char('t');
        mode.append_char('e');
        mode.append_char('s');
        mode.append_char('t');
        assert_eq!(mode.search_string(), Some("test"));

        let mut mode = Mode::Normal;
        mode.append_char('x');
        assert!(mode.is_normal());
    }

    #[test]
    fn test_pop_char() {
        let mut mode = Mode::command_with_text("test");
        mode.pop_char();
        assert_eq!(mode.command_string(), Some("tes"));
        mode.pop_char();
        mode.pop_char();
        mode.pop_char();
        assert_eq!(mode.command_string(), Some(""));

        let mut mode = Mode::Normal;
        mode.pop_char();
        assert!(mode.is_normal());
    }

    #[test]
    fn test_command_and_search_strings() {
        let command = Mode::command_with_text("save");
        assert_eq!(command.command_string(), Some("save"));
        assert_eq!(command.search_string(), None);

        let search = Mode::search_with_text("pattern");
        assert_eq!(search.search_string(), Some("pattern"));
        assert_eq!(search.command_string(), None);

        let normal = Mode::Normal;
        assert_eq!(normal.command_string(), None);
        assert_eq!(normal.search_string(), None);
    }

    #[test]
    fn test_input_cursor_and_insertion_midline() {
        let mut mode = Mode::command_with_text("wq");
        mode.move_input_left();
        mode.append_char('!');
        assert_eq!(mode.command_string(), Some("w!q"));
        assert_eq!(mode.input_cursor(), Some(2));
    }

    #[test]
    fn test_input_word_motions() {
        let mut mode = Mode::search_with_text("foo_bar -baz");
        mode.move_input_start();
        mode.move_input_word_right();
        assert_eq!(mode.input_cursor(), Some(7));
        mode.move_input_word_right();
        assert_eq!(mode.input_cursor(), Some(9));
        mode.move_input_word_right();
        assert_eq!(mode.input_cursor(), Some(12));
        mode.move_input_word_left();
        assert_eq!(mode.input_cursor(), Some(9));
        mode.move_input_word_left();
        assert_eq!(mode.input_cursor(), Some(8));
    }

    #[test]
    fn test_delete_input_variants() {
        let mut mode = Mode::command_with_text("alpha beta");
        mode.delete_input_word_backward();
        assert_eq!(mode.command_string(), Some("alpha "));
        mode.delete_input_to_start();
        assert_eq!(mode.command_string(), Some(""));
        mode.append_char('x');
        mode.append_char('y');
        mode.move_input_left();
        mode.delete_input_char_forward();
        assert_eq!(mode.command_string(), Some("x"));
        mode.delete_input_to_end();
        assert_eq!(mode.command_string(), Some("x"));
    }

    #[test]
    fn test_delete_input_word_forward() {
        let mut mode = Mode::command_with_text("alpha beta gamma");
        mode.move_input_word_left();
        mode.move_input_word_left();

        mode.delete_input_word_forward();

        assert_eq!(mode.command_string(), Some("alpha  gamma"));
        assert_eq!(mode.input_cursor(), Some(6));
    }

    #[test]
    fn test_take_input_resets_mode() {
        let mut command = Mode::command_with_text("w");
        assert_eq!(command.take_command_input(), Some("w".to_string()));
        assert!(command.is_normal());

        let mut search = Mode::search_with_text("pat");
        assert_eq!(search.take_search_input(), Some("pat".to_string()));
        assert!(search.is_normal());
    }

    #[test]
    fn test_default_mode() {
        let mode = Mode::Normal;
        assert!(mode.is_normal());
    }

    #[test]
    fn test_mode_label() {
        assert_eq!(Mode::Normal.mode_label(), "NORMAL");
        assert_eq!(Mode::visual_character().mode_label(), "VISUAL");
        assert_eq!(Mode::visual_line().mode_label(), "V-LINE");
        assert_eq!(Mode::Insert.mode_label(), "INSERT");
        assert_eq!(Mode::command_empty().mode_label(), "COMMAND");
        assert_eq!(Mode::search_empty().mode_label(), "SEARCH");
        assert_eq!(Mode::buffer_switch_empty().mode_label(), "NORMAL");
    }
}
