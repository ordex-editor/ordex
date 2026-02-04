# Data Model: Basic Editing Features

**Date**: 2025-02-04  
**Context**: Phase 002 Basic Editing - Core data structures for text editing

## Overview

This document defines the key data structures and their relationships for implementing basic editing features in the Ordex text editor. The design emphasizes clean abstractions, separation of concerns, and testability.

---

## Core Entities

### 1. TextBuffer

**Purpose**: Manages the text content with efficient insertion/deletion operations.

**Implementation**: Wrapper around `ropey::Rope` to abstract the underlying data structure.

```rust
pub struct TextBuffer {
    rope: Rope,  // from ropey crate
    modified: bool,
}
```

**Responsibilities**:
- Store and manipulate text content
- Provide line-based and character-based access
- Convert between line/column coordinates and character indices
- Track modification state
- Serialize to String for saving

**Key Operations**:
- `from_str(text: &str) -> Self` - Create from string
- `insert(&mut self, char_idx: usize, text: &str)` - Insert text at position
- `remove(&mut self, start: usize, end: usize)` - Remove text range
- `line(&self, line_idx: usize) -> Option<RopeSlice>` - Get line content
- `line_len(&self, line_idx: usize) -> usize` - Get line length
- `len_lines(&self) -> usize` - Get total line count
- `len_chars(&self) -> usize` - Get total character count
- `char_to_line(&self, char_idx: usize) -> usize` - Convert char index to line
- `line_to_char(&self, line_idx: usize) -> usize` - Convert line to char index
- `to_string(&self) -> String` - Serialize for saving
- `is_modified(&self) -> bool` - Check modification state
- `clear_modified(&mut self)` - Mark as saved

**Invariants**:
- Character indices must be on valid UTF-8 boundaries
- Line indices are 0-based internally
- Rope maintains UTF-8 validity automatically

---

### 2. Cursor

**Purpose**: Represents the current editing position within the document.

```rust
pub struct Cursor {
    line: usize,         // 0-based line index
    column: usize,       // 0-based column (char offset within line)
    desired_column: usize, // Preserved for vertical movement
}
```

**Responsibilities**:
- Track current position in document
- Maintain desired column for vertical movement
- Validate movements stay within document bounds

**Key Operations**:
- `new(line: usize, column: usize) -> Self` - Create cursor
- `move_left(&mut self, buffer: &TextBuffer)` - Move one char left
- `move_right(&mut self, buffer: &TextBuffer)` - Move one char right
- `move_up(&mut self, buffer: &TextBuffer)` - Move one line up
- `move_down(&mut self, buffer: &TextBuffer)` - Move one line down
- `move_to_line_start(&mut self)` - Jump to column 0
- `move_to_line_end(&mut self, buffer: &TextBuffer)` - Jump to line end
- `clamp_to_line(&mut self, buffer: &TextBuffer)` - Ensure column is valid
- `to_char_index(&self, buffer: &TextBuffer) -> usize` - Convert to char index
- `from_char_index(buffer: &TextBuffer, char_idx: usize) -> Self` - Create from char index

**Invariants**:
- `line` must be < `buffer.len_lines()`
- `column` must be ≤ length of current line
- `desired_column` preserves horizontal position during vertical movement

**Example**:
```
Line 0: "Hello world"  (length 11)
Line 1: "Hi"           (length 2)
Line 2: "Goodbye"      (length 7)

Cursor at (0, 7) with desired_column=7
- Move down → (1, 2) but desired_column stays 7
- Move down → (2, 7) - cursor restored to desired column
```

---

### 3. Mode

**Purpose**: Represents the current editor state and determines which key bindings are active.

```rust
pub enum Mode {
    Normal,
    Insert,
    Command(String),  // String holds the command being typed
    Search(String),   // String holds the search pattern being typed
}
```

**State Transitions**:

```
Normal ←─────────────────────┐
  │                           │
  ├─ 'i' ────→ Insert ───────┤ (Escape from any mode returns to Normal)
  │                           │
  ├─ ':' ────→ Command ──────┤
  │                           │
  └─ '/' ────→ Search ───────┘
```

**Responsibilities**:
- Determine which key bindings are active
- Store partial command/search input
- Control status bar display

**Key Operations**:
- `is_normal(&self) -> bool`
- `is_insert(&self) -> bool`
- `is_command(&self) -> bool`
- `is_search(&self) -> bool`
- `get_prompt(&self) -> String` - Returns "NORMAL", "INSERT", ":", or "/"
- `append_char(&mut self, c: char)` - Add char to Command/Search string
- `pop_char(&mut self)` - Remove last char from Command/Search string

---

### 4. Viewport

**Purpose**: Manages which portion of the document is visible on screen.

```rust
pub struct Viewport {
    first_visible_line: usize,  // Top line currently visible
    height: usize,              // Number of lines that fit on screen
    scroll_margin: usize,       // Lines to keep above/below cursor (e.g., 3)
}
```

**Responsibilities**:
- Track visible region of document
- Scroll to keep cursor in view
- Calculate visible line range

**Key Operations**:
- `new(height: usize) -> Self` - Create viewport
- `ensure_cursor_visible(&mut self, cursor: &Cursor, buffer: &TextBuffer)` - Scroll if needed
- `scroll_up(&mut self, lines: usize)` - Scroll viewport up
- `scroll_down(&mut self, lines: usize, buffer: &TextBuffer)` - Scroll down
- `page_up(&mut self, cursor: &mut Cursor, buffer: &TextBuffer)` - Page up
- `page_down(&mut self, cursor: &mut Cursor, buffer: &TextBuffer)` - Page down
- `visible_range(&self) -> Range<usize>` - Get visible line range

**Invariants**:
- `first_visible_line` must be < `buffer.len_lines()`
- Cursor must remain within `[first_visible_line, first_visible_line + height)` after scrolling

**Scroll Behavior**:
- When cursor moves near top/bottom, viewport scrolls to maintain `scroll_margin`
- Page up/down moves viewport by `height - 1` lines (keeps one line of context)

---

### 5. KeyBinding

**Purpose**: Maps keys to actions based on current mode.

```rust
pub struct KeyBinding {
    mode: Mode,
    key: Key,        // from termion::event::Key
    action: Action,
}

pub enum Action {
    // Navigation
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordForward,
    MoveWordBackward,
    PageUp,
    PageDown,
    
    // Mode changes
    EnterInsertMode,
    EnterCommandMode,
    EnterSearchMode,
    EnterNormalMode,
    
    // Editing
    InsertChar(char),
    DeleteCharBefore,
    InsertNewline,
    
    // Commands
    ExecuteCommand,
    CancelCommand,
    AppendToCommand(char),
    DeleteFromCommand,
    
    // System
    Quit,
    Save,
    
    // Search
    ExecuteSearch,
    CancelSearch,
}
```

**Responsibilities**:
- Define mapping between keys and actions
- Filter bindings by current mode
- Execute actions

**Key Operations**:
- `get_bindings_for_mode(mode: &Mode) -> Vec<KeyBinding>` - Get active bindings
- `find_action(key: Key, mode: &Mode) -> Option<Action>` - Look up action
- `execute_action(action: Action, editor_state: &mut EditorState)` - Perform action

**Configuration**:
- Bindings stored in-memory as constants/static data this phase
- Organized by mode for efficient lookup
- Future: load from config file (out of scope for this phase)

---

### 6. EditorState

**Purpose**: Top-level state container that owns all editor state.

```rust
pub struct EditorState {
    buffer: TextBuffer,
    cursor: Cursor,
    mode: Mode,
    viewport: Viewport,
    file_path: Option<PathBuf>,
    status_message: Option<String>,
}
```

**Responsibilities**:
- Own all mutable editor state
- Coordinate operations across components
- Provide high-level operations (save, load, search)

**Key Operations**:
- `new(terminal_height: usize) -> Self` - Create empty editor
- `load_file(path: &Path) -> Result<Self, io::Error>` - Load from file
- `save(&mut self) -> Result<(), io::Error>` - Save to file
- `handle_key(&mut self, key: Key)` - Process keyboard input
- `insert_char(&mut self, c: char)` - Insert at cursor
- `delete_char_before(&mut self)` - Backspace
- `search(&mut self, pattern: &str) -> bool` - Find and jump to pattern
- `goto_line(&mut self, line: usize)` - Jump to line number
- `render(&self) -> Vec<String>` - Generate screen content

**Invariants**:
- Cursor position always valid for current buffer
- Cursor always visible in viewport
- Mode consistent with available operations

---

## Data Flow Examples

### Example 1: User Types 'i' in Normal Mode

```
1. EditorState receives Key::Char('i')
2. Lookup binding: Normal + 'i' → EnterInsertMode
3. EditorState.mode = Mode::Insert
4. Status bar updates to show "INSERT"
5. Render loop redraws screen
```

### Example 2: User Types 'hello' in Insert Mode

```
For each key press:
1. EditorState receives Key::Char(c)
2. Lookup binding: Insert + char → InsertChar(c)
3. cursor_pos = cursor.to_char_index(buffer)
4. buffer.insert(cursor_pos, &c.to_string())
5. cursor.move_right(buffer)
6. viewport.ensure_cursor_visible(cursor, buffer)
7. buffer.modified = true
8. Render loop redraws affected lines
```

### Example 3: User Saves File with `:w`

```
1. User presses ':' in Normal mode
   → mode = Mode::Command(String::new())
2. User types 'w'
   → mode = Mode::Command("w".to_string())
3. User presses Enter
   → ExecuteCommand action
4. Parse command: "w" → Save action
5. buffer.to_string() → file contents
6. Write to file_path
7. buffer.clear_modified()
8. status_message = Some("File saved")
9. mode = Mode::Normal
```

### Example 4: User Searches with `/test`

```
1. User presses '/' in Normal mode
   → mode = Mode::Search(String::new())
2. User types "test"
   → mode = Mode::Search("test".to_string())
3. User presses Enter
   → ExecuteSearch action
4. Convert buffer to string, find "test"
5. If found: char_idx = position
   → cursor = Cursor::from_char_index(buffer, char_idx)
   → viewport.ensure_cursor_visible(cursor, buffer)
   → status_message = Some("Found at line X")
6. If not found:
   → status_message = Some("Pattern not found")
7. mode = Mode::Normal
```

---

## Module Boundaries

### text_buffer.rs
- Owns: `TextBuffer` struct
- Imports: `ropey::Rope`
- Exports: Public `TextBuffer` API

### cursor.rs
- Owns: `Cursor` struct
- Imports: `TextBuffer` (for validation)
- Exports: Public `Cursor` API

### mode.rs
- Owns: `Mode` enum
- Imports: None
- Exports: `Mode` and its methods

### viewport.rs
- Owns: `Viewport` struct
- Imports: `Cursor`, `TextBuffer`
- Exports: Public `Viewport` API

### keybindings.rs
- Owns: `KeyBinding`, `Action` types
- Imports: `Mode`, termion keys
- Exports: Binding lookup functions

### navigation.rs
- Owns: Navigation logic (word boundaries, page scrolling)
- Imports: `TextBuffer`, `Cursor`, `Viewport`
- Exports: Helper functions for complex navigation

### search.rs
- Owns: Search implementation
- Imports: `TextBuffer`
- Exports: `search(buffer, pattern) -> Option<char_idx>`

### editor_state.rs
- Owns: `EditorState` struct
- Imports: All other modules
- Exports: High-level editor operations

---

## Testing Strategy

### Unit Tests (in-module)

**TextBuffer**:
- Insert at start/middle/end
- Delete ranges
- Line/char conversions
- UTF-8 boundary handling
- Modification tracking

**Cursor**:
- Movement in all directions
- Clamping to valid positions
- Desired column preservation
- Edge cases (empty file, single line)

**Mode**:
- State transitions
- Command/search string building

**Viewport**:
- Scrolling logic
- Cursor visibility maintenance
- Page up/down behavior

**KeyBindings**:
- Binding lookups for each mode
- Action execution

**Navigation**:
- Word boundary detection
- Page scrolling math

**Search**:
- Pattern found/not found
- Multiple occurrences (find first)

### Integration Tests (tests/)

**navigation_test.rs**:
- hjkl navigation sequences
- w/b word jumping
- Ctrl+F/B paging

**editing_test.rs**:
- Insert mode: typing, backspace, newlines
- Mode transitions (i → type → Escape)

**save_test.rs**:
- Save modified file
- Save unmodified file
- File permissions errors

**search_test.rs**:
- Search and jump to result
- Pattern not found
- Search with special characters

---

## Performance Considerations

### TextBuffer (Rope)
- Insert/delete: O(log n) where n = document size
- Line access: O(log n)
- Acceptable for documents up to millions of lines

### Cursor Movement
- O(1) for simple moves (hjkl)
- O(log n) for position validation via TextBuffer

### Viewport Scrolling
- O(1) for scroll calculations
- O(log n) for fetching visible lines from TextBuffer

### Search
- O(n) linear scan for literal string match
- Acceptable for files up to 100k lines (2-second target)
- Future: Could optimize with rope-aware search or index

---

## Future Extensions (Out of Scope)

- **Undo/Redo**: Would add command history and inverse operations
- **Multiple Cursors**: Would change Cursor to `Vec<Cursor>`
- **Visual Selection**: Would add `Selection { start: Cursor, end: Cursor }`
- **Syntax Highlighting**: Would add `SyntaxHighlighter` component
- **Line Numbers**: Would add column to viewport rendering
- **Configuration**: Would load `KeyBinding` from file

---

**Data Model Complete**: 2025-02-04
