# Module Interface Contract: Cursor

**Module**: `cursor.rs`  
**Purpose**: Track and manage cursor position with validation  
**Date**: 2025-02-04

## Public API

```rust
pub struct Cursor {
    // Private fields
}

impl Cursor {
    /// Creates cursor at specified position
    pub fn new(line: usize, column: usize) -> Self;
    
    /// Moves cursor left by one character (if possible)
    pub fn move_left(&mut self, buffer: &TextBuffer);
    
    /// Moves cursor right by one character (if possible)
    pub fn move_right(&mut self, buffer: &TextBuffer);
    
    /// Moves cursor up by one line (if possible)
    pub fn move_up(&mut self, buffer: &TextBuffer);
    
    /// Moves cursor down by one line (if possible)
    pub fn move_down(&mut self, buffer: &TextBuffer);
    
    /// Moves cursor to start of current line
    pub fn move_to_line_start(&mut self);
    
    /// Moves cursor to end of current line
    pub fn move_to_line_end(&mut self, buffer: &TextBuffer);
    
    /// Ensures column is valid for current line
    pub fn clamp_to_line(&mut self, buffer: &TextBuffer);
    
    /// Converts cursor position to character index
    pub fn to_char_index(&self, buffer: &TextBuffer) -> usize;
    
    /// Creates cursor from character index
    pub fn from_char_index(buffer: &TextBuffer, char_idx: usize) -> Self;
    
    /// Returns current line number (0-based)
    pub fn line(&self) -> usize;
    
    /// Returns current column (0-based)
    pub fn column(&self) -> usize;
}
```

## Invariants

- `line` always < `buffer.len_lines()`
- `column` always ≤ length of current line
- `desired_column` preserved during vertical movement

## Testing Requirements

- Movement in all directions
- Boundary conditions (first/last line, start/end of line)
- Desired column preservation
- Coordinate conversion round-trips
