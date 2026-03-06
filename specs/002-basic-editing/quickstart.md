# Quickstart Guide: Basic Editing Features

**Phase**: 002 Basic Editing  
**Date**: 2025-02-04  
**Audience**: Developers implementing or extending this phase

## Overview

This phase extends the MVP viewer (Phase 001) into a functional text editor with vim-style navigation, insert mode editing, file saving, search, and status bar. The key addition is the `ropey` crate for efficient text storage.

---

## Key Concepts

### Text Storage: Rope Data Structure

**What is a Rope?**
A rope is a binary tree where leaf nodes contain text chunks. This enables O(log n) insert/delete operations compared to O(n) for strings.

**Why Ropey?**
- Battle-tested (used by Helix, Lapce editors)
- Built-in line/char indexing
- UTF-8 safe
- Minimal dependencies (only adds `str_indices`)

**Abstraction Layer**
All ropey code is isolated in `text_buffer.rs`. This allows future swapping if needed:

```rust
// text_buffer.rs - ONLY module importing ropey
use ropey::Rope;

pub struct TextBuffer {
    rope: Rope,  // internal detail
    modified: bool,
}

// Other modules use TextBuffer, not Rope directly
```

---

### Architecture Overview

```
┌──────────────────────────────────────────────────────┐
│                   EditorState                        │
│  (Owns: TextBuffer, Cursor, Mode, Viewport)         │
│  (Coordinates: All operations)                       │
└──────────────────────────────────────────────────────┘
         │         │           │            │
    ┌────┘    ┌───┘      ┌────┘       ┌────┘
    ▼         ▼          ▼            ▼
┌────────┐ ┌───────┐ ┌──────┐ ┌─────────────┐
│TextBuf │ │Cursor │ │Mode  │ │  Viewport   │
│(ropey) │ │       │ │      │ │             │
└────────┘ └───────┘ └──────┘ └─────────────┘
    │
    └─> Uses ropey crate (only place)
```

---

## Module Dependency Graph

```
main.rs
  └─> editor_state.rs
        ├─> text_buffer.rs (uses ropey)
        ├─> cursor.rs
        ├─> mode.rs
        ├─> viewport.rs
        ├─> keybindings.rs
        ├─> navigation.rs
        ├─> search.rs
        └─> status_bar.rs

tui.rs (from Phase 001, extended)
viewer.rs (from Phase 001, extended)
command.rs (from Phase 001, extended)
```

**Key Rule**: Only `text_buffer.rs` imports ropey. All other modules use `TextBuffer` abstraction.

---

## Building the Code

### 1. Add Dependency

```toml
# Cargo.toml
[dependencies]
termion = "4.0.6"
ropey = "2.0.0-beta.1"
```

**Dependency Budget Check**:
```bash
cargo tree --edges normal
# Should show 5 total: termion, libc, numtoa, ropey, str_indices
```

### 2. Module Structure

Create these new modules in `src/`:
- `text_buffer.rs` - Text storage wrapper
- `cursor.rs` - Position tracking
- `mode.rs` - State machine
- `viewport.rs` - Scrolling
- `navigation.rs` - Word/page movements
- `search.rs` - Pattern finding
- `status_bar.rs` - Status display
- `keybindings.rs` - Key-to-action mapping
- `editor_state.rs` - Top-level coordinator

Extend existing:
- `main.rs` - Use EditorState instead of simple viewer
- `tui.rs` - Add cursor positioning helpers
- `viewer.rs` - Render from TextBuffer instead of Vec<String>
- `command.rs` - Add :w and :{number} commands

### 3. Build & Test

```bash
# Build
cargo build

# Run unit tests
cargo test

# Run integration tests
cargo test --test navigation_test
cargo test --test editing_test
cargo test --test save_test
cargo test --test search_test

# Run editor
cargo run -- testfile.txt
```

---

## Implementation Order

Follow this sequence to minimize rework:

### Stage 1: Core Data Structures (Bottom-Up)

1. **text_buffer.rs** - Implement TextBuffer wrapper around ropey
   - Start with `new()`, `from_str()`, `len_lines()`, `len_chars()`
   - Add `insert()`, `remove()`, test thoroughly
   - Add `line()`, `line_len()` for rendering
   - Add `char_to_line()`, `line_to_char()` for cursor
   - Add modification tracking
   
2. **cursor.rs** - Implement Cursor with validation
   - Basic position (line, column, desired_column)
   - Movement methods (hjkl)
   - Coordinate conversion (to/from char_index)
   - Test edge cases (empty file, boundaries)

3. **mode.rs** - Implement Mode enum
   - Define variants (Normal, Insert, Command, Search)
   - Implement state predicates (is_normal, etc.)
   - Add string building (append_char, pop_char)
   - Test transitions

4. **viewport.rs** - Implement Viewport scrolling
   - Track first_visible_line, height
   - Implement ensure_cursor_visible
   - Add page_up/page_down
   - Test scroll boundaries

### Stage 2: Operations (Middle Layer)

5. **navigation.rs** - Implement word motion logic
   - `find_next_word_start(buffer, cursor) -> usize`
   - `find_prev_word_start(buffer, cursor) -> usize`
   - Define word boundaries (whitespace, punctuation)
   - Test various text patterns

6. **search.rs** - Implement literal search
   - `search(buffer, pattern) -> Option<char_idx>`
   - Linear scan for now (O(n) acceptable)
   - Test found/not found cases

7. **keybindings.rs** - Define key-to-action mapping
   - Define Action enum
   - Create binding tables for each mode
   - Implement lookup function
   - Keep organized by mode

### Stage 3: Integration (Top Layer)

8. **editor_state.rs** - Assemble everything
   - Create struct owning all state
   - Implement `handle_key()` dispatching to actions
   - Implement high-level commands (save, goto_line)
   - Implement `render()` generating screen lines

9. **status_bar.rs** - Implement status line
   - Format: `{mode} | {file_path} | Line {line}, Col {col} | {modified}`
   - Show command/search input when applicable

10. **Extend existing modules**:
    - Update `main.rs` to use EditorState
    - Update `viewer.rs` to render from TextBuffer
    - Update `command.rs` to execute :w and :{number}
    - Update `tui.rs` with any new terminal helpers

### Stage 4: Testing

11. Write integration tests for each user story (see spec.md)

---

## Code Examples

### Example 1: Using TextBuffer

```rust
use crate::text_buffer::TextBuffer;

// Create buffer
let mut buffer = TextBuffer::from_str("Hello\nWorld");

// Query
assert_eq!(buffer.len_lines(), 2);
assert_eq!(buffer.len_chars(), 11);  // "Hello\nWorld" = 11 chars

// Modify
buffer.insert(5, " there");  // "Hello there\nWorld"
assert!(buffer.is_modified());

// Line access
let line1 = buffer.line(1).unwrap();
println!("{}", line1.to_string());  // "World\n"

// Coordinate conversion
let line_idx = buffer.char_to_line(8);  // Which line is char 8 on?
let char_idx = buffer.line_to_char(1);  // Where does line 1 start? (6)
```

### Example 2: Cursor Movement

```rust
use crate::{cursor::Cursor, text_buffer::TextBuffer};

let buffer = TextBuffer::from_str("Hello\nWorld\nTest");
let mut cursor = Cursor::new(0, 0);  // Start of file

// Move right 5 characters: (0, 0) → (0, 5)
for _ in 0..5 {
    cursor.move_right(&buffer);
}

// Move down: (0, 5) → (1, 5) but line 1 is "World" (5 chars)
// Cursor clamps to (1, 5) - end of line
cursor.move_down(&buffer);

// Move down: (1, 5) → (2, 4) - "Test" is 4 chars
// Desired column stays 5, actual column becomes 4
cursor.move_down(&buffer);

// Move up: (2, 4) → (1, 5) - restored to desired column
cursor.move_up(&buffer);
```

### Example 3: Mode Transitions

```rust
use crate::mode::Mode;

let mut mode = Mode::Normal;

// User presses 'i'
mode = Mode::Insert;
assert_eq!(mode.get_prompt(), "INSERT");

// User presses Escape
mode = Mode::Normal;

// User presses ':'
mode = Mode::Command(String::new());
assert_eq!(mode.get_prompt(), ":");

// User types "w"
mode.append_char('w');
assert_eq!(mode.command_string(), Some("w"));

// User presses Enter → execute :w command
// (handled in editor_state.rs)
```

### Example 4: EditorState Handle Key

```rust
impl EditorState {
    pub fn handle_key(&mut self, key: Key) {
        use termion::event::Key::*;
        
        match (&self.mode, key) {
            // Normal mode bindings
            (Mode::Normal, Char('i')) => {
                self.mode = Mode::Insert;
            }
            (Mode::Normal, Char('h')) => {
                self.cursor.move_left(&self.buffer);
                self.viewport.ensure_cursor_visible(&self.cursor, &self.buffer);
            }
            // ... more bindings
            
            // Insert mode bindings
            (Mode::Insert, Char(c)) => {
                let char_idx = self.cursor.to_char_index(&self.buffer);
                self.buffer.insert(char_idx, &c.to_string());
                self.cursor.move_right(&self.buffer);
            }
            (Mode::Insert, Backspace) => {
                if self.cursor.column() > 0 {
                    let char_idx = self.cursor.to_char_index(&self.buffer);
                    self.buffer.remove(char_idx - 1, char_idx);
                    self.cursor.move_left(&self.buffer);
                }
            }
            (Mode::Insert, Esc) => {
                self.mode = Mode::Normal;
            }
            
            // ... more modes
        }
    }
}
```

---

## Testing Strategy

### Unit Tests (In-Module)

Each module has `#[cfg(test)] mod tests { ... }` at the bottom:

```rust
// text_buffer.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn insert_at_start() {
        let mut buf = TextBuffer::from_str("World");
        buf.insert(0, "Hello ");
        assert_eq!(buf.to_string(), "Hello World");
    }
    
    // ... more tests
}
```

### Integration Tests (tests/)

```rust
// tests/navigation_test.rs
use ordex::editor_state::EditorState;
use termion::event::Key;

#[test]
fn hjkl_navigation() {
    let mut editor = EditorState::load_file(
        Path::new("testdata/sample.txt"), 
        24
    ).unwrap();
    
    // Move right 3 times
    for _ in 0..3 {
        editor.handle_key(Key::Char('l'));
    }
    
    // Verify cursor position
    // (access via public method or rendering output)
    let (_, status) = editor.render(24);
    assert!(status.contains("Col 3"));
}
```

---

## Common Pitfalls

### 1. Character vs Byte Indices

**Problem**: Ropey uses character indices (Unicode scalar values), not byte offsets.

**Solution**: Always think in terms of chars, not bytes. Ropey handles UTF-8 for you.

```rust
// WRONG: Treating as bytes
let text = "Hello 😀";  // 😀 is 4 bytes
let byte_idx = 6;  // Would be in middle of emoji!

// RIGHT: Use char indices
let char_idx = 6;  // Correctly refers to position after emoji
```

### 2. Forgetting Desired Column

**Problem**: Cursor jumps horizontally when moving vertically through short lines.

**Solution**: Cursor struct maintains `desired_column` that's preserved during vertical movement.

### 3. Off-by-One Errors

**Problem**: Line/column displayed as 1-based but stored 0-based internally.

**Solution**: Only convert to 1-based for display:
```rust
let status = format!("Line {}, Col {}", cursor.line() + 1, cursor.column() + 1);
```

### 4. Modifying TextBuffer Without Marking Dirty

**Problem**: Changes not saved because modified flag not set.

**Solution**: TextBuffer automatically sets flag in `insert()` and `remove()`.

### 5. Viewport Not Updating After Cursor Move

**Problem**: Cursor moves off-screen.

**Solution**: Always call `viewport.ensure_cursor_visible()` after cursor movements.

---

## Performance Tips

### 1. Avoid to_string() in Loops

```rust
// BAD: Serializes entire buffer on every render
fn render_all_lines(buffer: &TextBuffer) {
    let text = buffer.to_string();  // O(n) - expensive!
    for line in text.lines() {
        // ...
    }
}

// GOOD: Access lines individually
fn render_visible_lines(buffer: &TextBuffer, viewport: &Viewport) {
    for line_idx in viewport.visible_range() {
        if let Some(line) = buffer.line(line_idx) {
            // ... render this line only
        }
    }
}
```

### 2. Batch Operations When Possible

```rust
// Less efficient: Multiple small inserts
for c in "Hello".chars() {
    buffer.insert(pos, &c.to_string());
    pos += 1;
}

// Better: Single insert
buffer.insert(pos, "Hello");
```

### 3. Use RopeSlice Instead of String

```rust
// Avoid converting to String if you just need to iterate
let line = buffer.line(idx).unwrap();
for c in line.chars() {  // Direct iteration, no allocation
    // ...
}
```

---

## Debugging Tips

### 1. Print Cursor Position

```rust
println!("Cursor: ({}, {}), desired: {}", 
    cursor.line(), cursor.column(), cursor.desired_column);
```

### 2. Visualize Buffer State

```rust
println!("Buffer: {} lines, {} chars, modified: {}", 
    buffer.len_lines(), buffer.len_chars(), buffer.is_modified());
for i in 0..buffer.len_lines() {
    println!("{}: {:?}", i, buffer.line(i).unwrap().to_string());
}
```

### 3. Trace Key Handling

```rust
pub fn handle_key(&mut self, key: Key) {
    eprintln!("Key: {:?}, Mode: {:?}", key, self.mode);
    // ... handle key
}
```

---

## Next Steps

After completing this phase, the editor will support:
- ✅ Navigation (hjkl, w/b, Ctrl+F/B)
- ✅ Insert mode (type, backspace, newlines)
- ✅ File saving (:w)
- ✅ Search (/)
- ✅ Go-to-line (:{number})
- ✅ Status bar (mode, position, modified flag)

Future phases might add:
- Undo/redo
- Visual selection
- Copy/paste/yank
- Syntax highlighting
- Multiple buffers
- Split windows

---

## References

- See `data-model.md` for detailed entity descriptions
- See `contracts/` for module interface specifications
- See `research.md` for rope vs alternatives analysis
- See `spec.md` for full requirements and acceptance criteria

---

**Quickstart Guide Complete**: 2025-02-04
