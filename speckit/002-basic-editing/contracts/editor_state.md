# Module Interface Contract: EditorState

**Module**: `editor_state.rs`  
**Purpose**: Top-level state container coordinating all editor components  
**Date**: 2025-02-04

## Public API

```rust
pub struct EditorState {
    // Private fields
}

impl EditorState {
    /// Creates empty editor with specified terminal height
    pub fn new(terminal_height: usize) -> Self;
    
    /// Loads a file from disk into the editor
    pub fn load_file(path: &Path, terminal_height: usize) -> Result<Self, io::Error>;
    
    /// Saves the current buffer to disk
    pub fn save(&mut self) -> Result<(), io::Error>;
    
    /// Processes a keyboard input and updates state
    pub fn handle_key(&mut self, key: termion::event::Key);
    
    /// Renders the current editor state to screen lines
    /// Returns (content_lines, status_line)
    pub fn render(&self, terminal_height: usize) -> (Vec<String>, String);
    
    /// Returns true if there are unsaved changes
    pub fn is_modified(&self) -> bool;
    
    /// Returns current status message (if any)
    pub fn status_message(&self) -> Option<&str>;
}
```

## Responsibilities

- Own all mutable editor state (buffer, cursor, mode, viewport)
- Coordinate operations across components
- Implement high-level commands (save, search, goto_line)
- Handle mode transitions
- Generate rendered output

## Key Interactions

- Uses `TextBuffer` for text storage
- Uses `Cursor` for position tracking
- Uses `Mode` for state machine
- Uses `Viewport` for scrolling
- Calls `keybindings` module to map keys to actions
- Calls `navigation` module for word motions
- Calls `search` module for pattern finding

## Testing Requirements

- Mode transitions via keyboard input
- File load/save operations
- Command execution (:w, :{number})
- Search execution
- Rendering output format
