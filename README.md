# Ordex

> **Note:** Significant portions of this project were designed and implemented with the help of advanced AI systems, blending automated code generation with human review and refinement.

A minimal TUI text editor written in Rust with vim-style keybindings.

## Features

### Navigation
- Character movement: `h` (left), `j` (down), `k` (up), `l` (right)
- Word movement: `w` (next word), `b` (previous word)
- Page scrolling: `Ctrl+F` (forward), `Ctrl+B` (backward)

### Text Editing
- Enter insert mode: `i`
- Exit insert mode: `Esc`
- Insert characters at cursor position
- Backspace to delete characters
- Enter to create new lines

### File Operations
- Open and display text files
- Save changes: `:w`
- Save and quit: `:wq`
- Quit: `:q`

### Search
- Search for text: `/pattern`
- Case-sensitive literal string search
- Wraps around to beginning if not found

### Navigation Commands
- Go to line: `:{number}` (e.g., `:50` to jump to line 50)
- Go to first line: `:1`

### Status Bar
- Shows current mode (NORMAL, INSERT, COMMAND, SEARCH)
- Displays file name
- Shows cursor position (line:column)
- Indicates modified state with `[+]`

## Installation

```bash
cargo build --release
```

The binary will be available at `target/release/ordex`.

## Usage

```bash
ordex <file>
```

### Example

```bash
ordex README.md
```

### Keybindings

#### Normal Mode
- `h`, `j`, `k`, `l` - Move cursor left, down, up, right
- `w` - Move to next word
- `b` - Move to previous word
- `Ctrl+F` - Page forward
- `Ctrl+B` - Page backward
- `i` - Enter insert mode
- `:` - Enter command mode
- `/` - Enter search mode

#### Insert Mode
- `Esc` - Return to normal mode
- Printable characters - Insert at cursor
- `Backspace` - Delete character before cursor
- `Enter` - Insert new line

#### Command Mode
- `:w` - Save file
- `:q` - Quit editor
- `:wq` - Save and quit
- `:{number}` - Go to line number
- `Esc` - Cancel command

#### Search Mode
- Type search pattern
- `Enter` - Execute search
- `Esc` - Cancel search

## Requirements

- Rust (stable)
- POSIX-compatible terminal with ANSI support

## Architecture

- `src/main.rs` - Entry point, CLI parsing, and main event loop
- `src/editor_state.rs` - Central editor state management
- `src/text_buffer.rs` - Text storage using rope data structure
- `src/cursor.rs` - Cursor position and movement
- `src/mode.rs` - Editor mode management (Normal/Insert/Command/Search)
- `src/viewport.rs` - Visible portion of document
- `src/navigation.rs` - Word boundary detection and navigation
- `src/keybindings.rs` - Key-to-action mapping
- `src/tui.rs` - Terminal handling (termion isolation)
- `src/viewer.rs` - Content rendering helpers
- `src/command.rs` - Command mode infrastructure (legacy)

## Dependencies

Runtime dependencies (5):
- `termion` 4.0.6 - Terminal handling
- `ropey` 2.0.0-beta.1 - Rope data structure for efficient text editing
- `str_indices` 0.4.4 - (transitive from ropey)
- `libc` 0.2.180 - (transitive from termion)
- `numtoa` 0.2.4 - (transitive from termion)

**Constitutional Limit Reached**: 5/5 runtime dependencies. No additional runtime dependencies can be added.

Dev dependencies:
- `test_utils` - Local crate for test fixtures

## Testing

```bash
cargo test
```

All tests passing: 120+ total (84 unit tests in modules, 36 integration tests)

Test coverage includes:
- Text buffer operations (insert, delete, search)
- Cursor movement and boundary protection
- Mode transitions
- Navigation (character, word, page)
- Viewport scrolling
- Key bindings

## Performance

Ordex is designed for performance:
- Responds to keyboard input within 16ms (60fps)
- Handles files > 1 GB without performance degradation
- Search completes within 2 seconds for 100k line files
- Uses rope data structure (ropey) for O(log n) insertions/deletions

## Development

This implementation focuses on:
- Clean architecture with isolated terminal code
- Modal editing (vim-style)
- Efficient text data structure (rope)
- Comprehensive test coverage
- Minimal dependency footprint (5 runtime dependencies)

## License

See LICENSE file for details.
