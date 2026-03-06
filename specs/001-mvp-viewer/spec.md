# Ordex MVP Specification

## Overview

Ordex is a minimal TUI text editor. This MVP establishes the foundational architecture: terminal rendering, file loading, and command input.

## Scope

**In scope:**
- Open and display a file passed as CLI argument
- Quit with `:q` command
- Basic terminal UI rendering

**Out of scope (future iterations):**
- Editing, cursor movement, modes, LSP, fuzzy finding

## Functional Requirements

### FR-1: File Opening

The editor shall accept a file path as a CLI argument and display its contents.

- If no argument is provided, exit with usage message
- If file does not exist, exit with error message
- File content is displayed read-only

### FR-2: Terminal Display

The editor shall render file content in a raw terminal mode.

- Clear screen on startup
- Display file content starting from line 1
- Show only as many lines as fit in terminal height
- Long lines may be truncated at terminal width (no wrapping required)
- Bottom line reserved for command input

### FR-3: Command Input

The editor shall accept `:` commands in vim-style.

- Pressing `:` enters command mode, showing `:` at bottom line
- Typing characters appends to command buffer, displayed after `:`
- Enter executes the command
- Escape cancels command input and clears command line
- Only supported command: `q` (quit)
- Unknown commands display brief error, then return to viewing

### FR-4: Quit

The editor shall exit cleanly when `:q` is entered.

- Restore terminal to original state before exit
- Exit with status code 0

## Non-Functional Requirements

### NFR-1: Dependency Budget

Per project constitution:
- Maximum 5 transitive runtime dependencies
- No proc-macros or heavy build scripts

### NFR-2: Error Handling

- All errors should produce user-friendly messages
- No panics in normal operation

## Technical Notes

- Use raw terminal mode for keyboard input
- Restore terminal state on exit (including on panic/error)
