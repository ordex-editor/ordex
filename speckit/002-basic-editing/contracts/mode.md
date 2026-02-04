# Module Interface Contract: Mode

**Module**: `mode.rs`  
**Purpose**: Represent editor state and command/search input  
**Date**: 2025-02-04

## Public API

```rust
pub enum Mode {
    Normal,
    Insert,
    Command(String),
    Search(String),
}

impl Mode {
    /// Returns true if in normal mode
    pub fn is_normal(&self) -> bool;
    
    /// Returns true if in insert mode
    pub fn is_insert(&self) -> bool;
    
    /// Returns true if in command mode
    pub fn is_command(&self) -> bool;
    
    /// Returns true if in search mode
    pub fn is_search(&self) -> bool;
    
    /// Returns display string for status bar
    /// "NORMAL", "INSERT", ":{command}", "/{search}"
    pub fn get_prompt(&self) -> String;
    
    /// Appends character to command/search string (if applicable)
    pub fn append_char(&mut self, c: char);
    
    /// Removes last character from command/search string (if applicable)
    pub fn pop_char(&mut self);
    
    /// Returns current command string (if in command mode)
    pub fn command_string(&self) -> Option<&str>;
    
    /// Returns current search string (if in search mode)
    pub fn search_string(&self) -> Option<&str>;
}
```

## State Transitions

- From Normal: 'i' → Insert, ':' → Command, '/' → Search
- From any mode: Escape → Normal

## Testing Requirements

- State transition logic
- String building (append/pop)
- Prompt generation for each mode
