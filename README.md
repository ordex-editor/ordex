# Ordex

A minimal TUI text viewer written in Rust.

## Features

- Open and display text files in the terminal
- Vim-style `:q` command to quit
- Clean terminal handling with automatic restoration
- Lightweight with only 3 runtime dependencies

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

### Commands

- `:q` - Quit the viewer
- `Esc` - Cancel command input

## Requirements

- Rust (stable)
- POSIX-compatible terminal

## Architecture

- `src/main.rs` - Entry point and CLI argument parsing
- `src/tui.rs` - Terminal handling (all termion code isolated here)
- `src/viewer.rs` - File content rendering and viewport management
- `src/command.rs` - Command mode handling

## Dependencies

Runtime dependencies (3):
- `termion` 4.0.6 - Terminal handling
- `libc` - System library
- `numtoa` - Number formatting

Dev dependencies:
- `tempfile` - Test fixtures

## Testing

```bash
cargo test
```

All tests passing: 21 total (16 unit tests, 5 integration tests)

## Development

This is an MVP implementation focusing on:
- Clean architecture with isolated terminal code
- Proper terminal restoration via RAII
- Comprehensive test coverage
- Minimal dependency footprint

Future enhancements could include:
- Text editing capabilities
- Cursor movement
- Search functionality
- Syntax highlighting

## License

See LICENSE file for details.
