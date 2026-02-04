# Feature Specification: Basic Editing Features

**Feature Branch**: `002-basic-editing`
**Created**: 2025-02-04
**Status**: Draft
**Input**: User description: "Adding necessary basic features: Navigation (via hjkl, move by word, go to previous/next page), File saving, Insert mode to edit the text, Basic search, Go-to line, Status bar to show the current mode"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - View File with Navigation (Priority: P1)

A user opens a file and navigates through its content using keyboard commands to review different sections without editing.

**Why this priority**: Navigation is the most fundamental capability that enables all other features. Without the ability to move through the file efficiently, users cannot locate content to edit, search, or save.

**Independent Test**: Can be fully tested by opening a multi-screen file and using hjkl keys to move character by character, w/b to jump between words, and Ctrl+F/Ctrl+B to page through the document. Delivers immediate value by allowing users to review and locate content in files.

**Acceptance Scenarios**:

1. **Given** a file is open with cursor at line 5 column 10, **When** user presses 'h', **Then** cursor moves one character left to column 9
2. **Given** a file is open with cursor at line 5 column 10, **When** user presses 'l', **Then** cursor moves one character right to column 11
3. **Given** a file is open with cursor at line 5, **When** user presses 'j', **Then** cursor moves down one line to line 6
4. **Given** a file is open with cursor at line 5, **When** user presses 'k', **Then** cursor moves up one line to line 4
5. **Given** a file is open with cursor at beginning of word "example", **When** user presses 'w', **Then** cursor jumps to the beginning of the next word
6. **Given** a file is open with cursor at beginning of second word, **When** user presses 'b', **Then** cursor jumps to the beginning of the previous word
7. **Given** a multi-page file is open on first screen, **When** user presses Ctrl+F, **Then** content scrolls forward one page showing the next screen of lines
8. **Given** a multi-page file is open on second screen, **When** user presses Ctrl+B, **Then** content scrolls backward one page showing the previous screen of lines
9. **Given** cursor is at the last character of a line, **When** user presses 'l', **Then** cursor does not move beyond the line end
10. **Given** cursor is at the first character of a line, **When** user presses 'h', **Then** cursor does not move before the line start

---

### User Story 2 - Edit Text in Insert Mode (Priority: P2)

A user enters insert mode to add, modify, or delete text within the file, then returns to normal mode to navigate or execute commands.

**Why this priority**: Editing is the core value proposition that transforms the viewer into an editor. Must come after navigation since users need to position the cursor before editing.

**Independent Test**: Can be fully tested by opening a file, pressing 'i' to enter insert mode, typing text, using backspace to delete, pressing Escape to return to normal mode. Delivers value by enabling users to modify file contents.

**Acceptance Scenarios**:

1. **Given** file is open in normal mode, **When** user presses 'i', **Then** mode changes to insert mode and status bar shows "INSERT"
2. **Given** editor is in insert mode with cursor at position, **When** user types character 'a', **Then** character is inserted at cursor position and cursor advances
3. **Given** editor is in insert mode with text "hello" and cursor after 'o', **When** user types " world", **Then** text becomes "hello world"
4. **Given** editor is in insert mode with cursor after character 'x', **When** user presses backspace, **Then** character 'x' is deleted and cursor moves back
5. **Given** editor is in insert mode, **When** user presses Escape key, **Then** mode changes to normal mode and status bar shows "NORMAL"
6. **Given** editor is in insert mode with empty file, **When** user types multiple lines using Enter key, **Then** new lines are created and cursor moves to each new line
7. **Given** editor is in insert mode, **When** user types 100 characters rapidly, **Then** all characters appear without lag or freeze

---

### User Story 3 - Save Modified File (Priority: P3)

A user makes changes to a file and saves them persistently using the `:w` command.

**Why this priority**: Saving enables persistence of edits. Comes after editing capability since there's nothing to save without the ability to modify content.

**Independent Test**: Can be fully tested by opening a file, entering insert mode, making changes, returning to normal mode, typing `:w`, and verifying the file on disk reflects the changes. Delivers value by ensuring user work is not lost.

**Acceptance Scenarios**:

1. **Given** file is open with unsaved modifications, **When** user types `:w` and presses Enter, **Then** changes are written to disk and status bar confirms "File saved"
2. **Given** file is open with no modifications, **When** user types `:w` and presses Enter, **Then** status bar shows "No changes to save"
3. **Given** file is saved successfully, **When** user checks file timestamp, **Then** timestamp reflects the save operation time
4. **Given** file has write permissions, **When** user saves after editing, **Then** original file content is replaced with new content
5. **Given** a file with 10000 lines is modified, **When** user saves, **Then** save operation completes without freezing the editor

---

### User Story 4 - Search for Text (Priority: P4)

A user searches for a specific text pattern within the file and jumps to the first occurrence.

**Why this priority**: Search enables quick navigation to specific content, enhancing the navigation capabilities. Useful for large files where manual navigation is inefficient.

**Independent Test**: Can be fully tested by opening a file with known content, typing `/searchterm`, pressing Enter, and verifying cursor jumps to the first occurrence. Delivers value by saving time locating specific content.

**Acceptance Scenarios**:

1. **Given** file is open in normal mode, **When** user types `/` followed by "example" and presses Enter, **Then** cursor jumps to first occurrence of "example"
2. **Given** file is open with search term "test", **When** user types `/test` and presses Enter, **Then** cursor moves to first line containing "test" and highlights the match
3. **Given** file is open, **When** user searches for text that doesn't exist, **Then** status bar shows "Pattern not found"
4. **Given** file is open with multiple occurrences of "bug", **When** user searches for "bug", **Then** cursor jumps to the first occurrence
5. **Given** file is open, **When** user types `/` and presses Escape, **Then** search is cancelled and cursor remains at current position
6. **Given** a file with 50000 lines, **When** user searches for a term in the last 1000 lines, **Then** search completes and jumps to result without freezing

---

### User Story 5 - Jump to Specific Line (Priority: P5)

A user wants to navigate directly to a specific line number using the `:` command followed by a line number.

**Why this priority**: Go-to-line is a convenience feature for developers working with compiler errors or log files that reference line numbers. Less critical than basic navigation.

**Independent Test**: Can be fully tested by opening a file with at least 100 lines, typing `:50` and pressing Enter, and verifying cursor is now on line 50. Delivers value by enabling precise navigation.

**Acceptance Scenarios**:

1. **Given** file with 100 lines is open, **When** user types `:50` and presses Enter, **Then** cursor jumps to line 50
2. **Given** file with 100 lines is open, **When** user types `:1` and presses Enter, **Then** cursor jumps to first line
3. **Given** file with 100 lines is open, **When** user types `:100` and presses Enter, **Then** cursor jumps to last line
4. **Given** file with 100 lines is open, **When** user types `:999` and presses Enter, **Then** cursor jumps to last line and status bar shows "Line number out of range, moved to last line"
5. **Given** file is open, **When** user types `:abc` and presses Enter, **Then** status bar shows "Invalid line number"

---

### User Story 6 - Monitor Current Mode (Priority: P6)

A user always knows which mode they are in (normal, insert, command) by glancing at the status bar.

**Why this priority**: Status bar provides essential context about the editor state. While important for usability, it supports other features rather than delivering standalone value.

**Independent Test**: Can be fully tested by switching between modes and verifying the status bar displays "NORMAL", "INSERT", or "COMMAND" accordingly. Delivers value by preventing mode confusion.

**Acceptance Scenarios**:

1. **Given** editor starts and opens a file, **When** file is displayed, **Then** status bar shows "NORMAL" mode
2. **Given** editor is in normal mode, **When** user presses 'i', **Then** status bar updates to show "INSERT"
3. **Given** editor is in insert mode, **When** user presses Escape, **Then** status bar updates to show "NORMAL"
4. **Given** editor is in normal mode, **When** user types `:`, **Then** status bar shows "COMMAND" and displays the `:` prompt
5. **Given** editor is in command mode, **When** user presses Escape, **Then** status bar returns to show "NORMAL"
6. **Given** editor is in any mode, **When** mode changes, **Then** status bar updates immediately without delay

---

### Edge Cases

- What happens when user tries to move cursor beyond file boundaries (first line, last line, line start, line end)?
- How does system handle attempting to save a read-only file?
- What happens when user searches for an empty string (types `/` and immediately presses Enter)?
- How does system handle cursor positioning when jumping to a line shorter than current cursor column?
- What happens when file is modified by external program while open in editor?
- How does system handle very long lines (> 1000 characters) during navigation?
- What happens when user presses backspace at the beginning of a line in insert mode?
- How does system handle navigation in an empty file?
- What happens when user tries to navigate past the last page using Ctrl+F?
- How does search behave with special characters or whitespace in the pattern?

## Requirements *(mandatory)*

### Functional Requirements

#### Navigation

- **FR-001**: System MUST support character-level navigation using h (left), j (down), k (up), l (right) keys in normal mode
- **FR-002**: System MUST support word-level navigation using w (next word) and b (previous word) keys in normal mode
- **FR-003**: System MUST support page navigation using Ctrl+F (forward page) and Ctrl+B (backward page) in normal mode
- **FR-004**: System MUST prevent cursor from moving beyond file boundaries (before first character, after last character, above first line, below last line)
- **FR-005**: System MUST maintain cursor position within valid line bounds when moving between lines of different lengths
- **FR-006**: Word navigation MUST treat whitespace, punctuation, and alphanumeric boundaries as word delimiters

#### Insert Mode

- **FR-007**: System MUST allow user to enter insert mode by pressing 'i' key in normal mode
- **FR-008**: System MUST allow user to exit insert mode by pressing Escape key
- **FR-009**: System MUST insert typed characters at current cursor position in insert mode
- **FR-010**: System MUST advance cursor position after each character insertion
- **FR-011**: System MUST support backspace key to delete character before cursor in insert mode
- **FR-012**: System MUST support Enter key to create new line at cursor position in insert mode
- **FR-013**: System MUST track file modification state (modified or unmodified)
- **FR-014**: System MUST maintain visual cursor position on screen during text insertion

#### File Saving

- **FR-015**: System MUST support `:w` command to save file changes to disk
- **FR-016**: System MUST write complete file contents to original file path on save
- **FR-017**: System MUST provide confirmation message after successful save
- **FR-018**: System MUST provide error message if save operation fails (e.g., permission denied, disk full)
- **FR-019**: System MUST handle save of large files (> 100 MB) without freezing
- **FR-020**: System MUST reset modification state to unmodified after successful save

#### Search

- **FR-021**: System MUST support `/` key to enter search mode in normal mode
- **FR-022**: System MUST display search prompt at bottom of screen showing `/` and typed pattern
- **FR-023**: System MUST execute search when user presses Enter in search mode
- **FR-024**: System MUST find first occurrence of exact text match (literal search, case-sensitive)
- **FR-025**: System MUST move cursor to first character of match when found
- **FR-026**: System MUST display "Pattern not found" message when search has no matches
- **FR-027**: System MUST allow cancelling search by pressing Escape
- **FR-028**: System MUST handle searches in files with > 100,000 lines without freezing
- **FR-029**: System MUST support searching for patterns containing spaces and special characters

#### Go-to Line

- **FR-030**: System MUST support `:{number}` command to jump to specific line number
- **FR-031**: System MUST move cursor to beginning of target line when line number is valid
- **FR-032**: System MUST move cursor to last line when requested line number exceeds file length
- **FR-033**: System MUST display error message for invalid line numbers (non-numeric input)
- **FR-034**: System MUST move cursor to first line when line number is less than 1

#### Status Bar

- **FR-035**: System MUST display status bar at bottom of screen at all times
- **FR-036**: System MUST show current mode (NORMAL, INSERT, COMMAND) in status bar
- **FR-037**: System MUST update status bar immediately when mode changes
- **FR-038**: System MUST display current line number and column number in status bar
- **FR-039**: System MUST display file modification indicator (e.g., "[+]" for modified) in status bar
- **FR-040**: Status bar MUST remain visible and not interfere with file content display

#### Configuration Foundation

- **FR-041**: System MUST use an in-memory configuration structure to map keys to actions
- **FR-042**: Configuration structure MUST be read-only during editor session
- **FR-043**: Key binding configuration MUST be isolated in a dedicated module to facilitate future file-based configuration

#### Text Data Structure

- **FR-044**: System MUST use an efficient data structure for text storage that supports:
  - Efficient insertion and deletion at arbitrary positions
  - Efficient line-based access for rendering
  - Efficient character and line indexing
  - Ability to handle files > 1 GB without performance degradation
- **FR-045**: Text data structure selection MUST be based on research comparing rope, piece table, gap buffer, and other alternatives [NEEDS CLARIFICATION: Which data structure is best? Requires research into existing crates that meet dependency constraints]
- **FR-046**: Text data structure MUST support future regex search capability
- **FR-047**: Text data structure implementation MUST either come from an existing crate that satisfies the constitution (max 5 runtime dependencies) or be implemented in-project

### Key Entities

- **Document**: Represents the file content in memory, including:
  - Text content (using efficient data structure)
  - Modification state (clean or modified)
  - File path
  - Cursor position (line, column)
  
- **Cursor**: Represents the current editing position, including:
  - Line number (1-indexed for display, 0-indexed internally)
  - Column number (1-indexed for display, 0-indexed internally)
  - Desired column (for maintaining horizontal position when moving vertically)

- **Mode**: Represents the current editor state, one of:
  - Normal mode (navigation and commands)
  - Insert mode (text editing)
  - Command mode (executing : commands)
  - Search mode (entering / search patterns)

- **KeyBinding**: Represents the mapping between key presses and actions:
  - Key input (character or key code)
  - Associated action (command to execute)
  - Mode context (which mode the binding applies to)

- **Viewport**: Represents the visible portion of the document:
  - First visible line
  - Number of visible lines (based on terminal height)
  - Horizontal scroll offset (for long lines)

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can navigate through a 10,000-line file using all navigation commands (hjkl, w/b, Ctrl+F/Ctrl+B) with cursor responding within 16ms (60fps) of keypress
- **SC-002**: Users can type 120 characters per minute in insert mode without any dropped keystrokes or visible lag
- **SC-003**: Users can save a modified 500 MB file with the `:w` command completing within 5 seconds and without editor freeze
- **SC-004**: Users can search for a term in a 100,000-line file with results appearing within 2 seconds
- **SC-005**: Users can jump to any line in a 50,000-line file using `:{number}` command with cursor positioning completing within 100ms
- **SC-006**: Status bar updates mode indicator within 16ms of mode change, providing immediate visual feedback
- **SC-007**: Users can edit files containing 1 GB of text with all operations (navigation, editing, search, save) completing without freezing the editor interface
- **SC-008**: All keyboard navigation commands work consistently in normal mode with 100% reliability (no missed keypresses)
- **SC-009**: Users can successfully save modified files 100% of the time when file has write permissions
- **SC-010**: New users can determine current editor mode by glancing at status bar without consulting documentation

## Assumptions *(mandatory)*

- Users are familiar with basic vim-style modal editing concepts
- Terminal supports standard ANSI escape sequences for cursor positioning and screen clearing
- Files use UTF-8 encoding (or ASCII subset)
- Word boundaries are defined by whitespace and punctuation characters (standard definition)
- Users have read/write permissions for files they open
- Terminal has minimum dimensions of 24 rows × 80 columns
- System has sufficient memory to load entire file content (streaming not required for this phase)
- Search patterns are literal strings (no regex support in this phase)
- Single cursor editing (no multiple cursors or visual selection in this phase)
- Line endings are normalized on load (CRLF → LF) for internal representation
- All operations complete synchronously (no async/background operations)
- File content fits within available system memory
- Key bindings use default configuration (no custom user key mappings in this phase)

## Dependencies *(mandatory)*

### Previous Phases

- **Phase 001 (MVP Viewer)**: This phase builds upon:
  - Terminal handling infrastructure (termion integration, raw mode, terminal restoration)
  - File loading capability
  - Basic command mode infrastructure (`:` prefix commands)
  - Basic rendering framework
  - Viewport management for displaying content

### External Dependencies

- Must maintain constitution constraint of max 5 total runtime dependencies
- Text data structure crate selection depends on research outcome (see FR-045) - may add 1-2 dependencies depending on crate chosen
- Current dependency count from Phase 001: 3 runtime dependencies (termion, libc, numtoa)
- Remaining budget: 2 additional runtime dependencies for text data structure and potential utility crates

## Non-Functional Requirements *(mandatory)*

### Performance

- **NFR-001**: No user-visible operation shall cause editor to freeze or become unresponsive (critical constraint)
- **NFR-002**: All keyboard input shall be processed within 16ms (60fps) to maintain responsive feel
- **NFR-003**: File save operations for files < 10 MB shall complete within 1 second
- **NFR-004**: Search operations shall provide progress indication for searches taking > 500ms
- **NFR-005**: Editor shall maintain 60fps rendering during continuous scrolling

### Reliability

- **NFR-006**: System shall not lose user edits during normal operation
- **NFR-007**: System shall restore terminal state even if editor crashes during operation
- **NFR-008**: System shall validate all file write operations and report errors clearly

### Usability

- **NFR-009**: Status bar shall always display current mode to prevent user confusion
- **NFR-010**: All error messages shall be displayed in status bar area without obscuring file content
- **NFR-011**: Cursor position shall be always visible on screen during all operations

### Maintainability

- **NFR-012**: Key binding configuration shall be isolated to facilitate future file-based configuration
- **NFR-013**: Text data structure shall be abstracted behind clean interface to allow future swapping if needed
- **NFR-014**: All code shall follow project constitution style guidelines (comments explain "why", cargo fmt/clippy compliance)

## Out of Scope *(mandatory)*

The following features are explicitly NOT included in this phase and are deferred to future phases:

- Visual/visual-line selection modes
- Multiple cursors
- Undo/redo functionality
- Syntax highlighting
- Line numbers display
- File-based configuration for key bindings (only in-memory config this phase)
- Regex search patterns (only literal search this phase)
- Search and replace
- Case-insensitive search
- Find next/previous occurrence (repeat search)
- Copy/paste/yank operations
- Advanced vim motions (e.g., gg, G, $, 0, ^)
- Split windows
- Buffer management (multiple open files)
- Auto-save
- File recovery after crash
- Integration with external tools or LSP
- Mouse support
- Unicode combining characters or right-to-left text
- Macro recording and playback
- Marks and bookmarks
- Folding
- Diff view
- Custom color schemes
- Status line customization beyond mode/position display
