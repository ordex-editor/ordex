# Module Interface Contract: TextBuffer

**Module**: `text_buffer.rs`  
**Purpose**: Abstract wrapper around ropey::Rope for text storage and manipulation  
**Date**: 2025-02-04

## Public API

### Struct

```rust
pub struct TextBuffer {
    // Private fields - implementation detail
}
```

### Constructor Methods

```rust
impl TextBuffer {
    /// Creates an empty text buffer
    pub fn new() -> Self;
    
    /// Creates a text buffer from a string
    pub fn from_str(text: &str) -> Self;
}
```

### Modification Methods

```rust
impl TextBuffer {
    /// Inserts text at the specified character index
    /// Panics if char_idx is not on a UTF-8 boundary or exceeds buffer length
    pub fn insert(&mut self, char_idx: usize, text: &str);
    
    /// Removes a range of characters
    /// Panics if indices not on UTF-8 boundaries or out of range
    pub fn remove(&mut self, start: usize, end: usize);
}
```

### Query Methods

```rust
impl TextBuffer {
    /// Returns the content of a specific line (includes newline)
    pub fn line(&self, line_idx: usize) -> Option<RopeSlice>;
    
    /// Returns the length of a specific line in characters (excluding newline)
    pub fn line_len(&self, line_idx: usize) -> usize;
    
    /// Returns the total number of lines in the buffer
    pub fn len_lines(&self) -> usize;
    
    /// Returns the total number of characters in the buffer
    pub fn len_chars(&self) -> usize;
    
    /// Returns true if the buffer is empty
    pub fn is_empty(&self) -> bool;
    
    /// Converts a character index to a line index
    pub fn char_to_line(&self, char_idx: usize) -> usize;
    
    /// Converts a line index to the character index of that line's first character
    pub fn line_to_char(&self, line_idx: usize) -> usize;
    
    /// Returns true if the buffer has been modified since last save
    pub fn is_modified(&self) -> bool;
}
```

### Serialization & State Methods

```rust
impl TextBuffer {
    /// Converts the entire buffer to a String (O(n) - use for saving only)
    pub fn to_string(&self) -> String;
    
    /// Marks the buffer as unmodified (called after saving)
    pub fn clear_modified(&mut self);
}
```

## Invariants

1. UTF-8 validity maintained at all times
2. `len_lines()` always ≥ 1, even for empty buffer
3. Any `insert()` or `remove()` sets modified flag
4. Character indices must be on UTF-8 boundaries
5. Lines stored with '\n' terminators

## Dependencies

- ropey 2.0.0-beta.1 (only used internally)

## Testing Requirements

- Insert/delete at start/middle/end
- Multi-line operations
- UTF-8 multi-byte characters
- Coordinate conversion round-trips
- Large buffers (10k+ lines)
