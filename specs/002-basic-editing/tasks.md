---
description: "Implementation tasks for Basic Editing Features"
---

# Tasks: Basic Editing Features (Phase 002)

**Input**: Design documents from `/speckit/002-basic-editing/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: Tests are included for each user story to ensure quality and correctness.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- Single Rust project at repository root
- Source: `src/`
- Tests: `tests/integration/`
- Unit tests: inline `#[cfg(test)]` modules

---

## Phase 1: Setup & Dependencies

**Purpose**: Add ropey dependency and verify constitutional compliance

- [X] T001 Add `ropey = "2.0.0-beta.1"` to Cargo.toml dependencies
- [X] T002 Run `cargo tree --edges normal` to verify exactly 5 total dependencies (termion, libc, numtoa, ropey, str_indices)
- [X] T003 Run `cargo build` to verify dependency resolution and compilation
- [X] T004 Run `cargo clippy` and `cargo fmt --check` to ensure code standards baseline

**Checkpoint**: Constitutional compliance verified - at dependency limit (5/5) ✅

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core data structures that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T005 [P] Create `src/text_buffer.rs` with TextBuffer struct wrapping ropey::Rope
- [X] T006 [P] Create `src/cursor.rs` with Cursor struct (line, column, desired_column fields)
- [X] T007 [P] Create `src/mode.rs` with Mode enum (Normal, Insert, Command, Search variants)
- [X] T008 [P] Create `src/viewport.rs` with Viewport struct (first_visible_line, height, scroll_margin)
- [X] T009 Implement TextBuffer::new() and TextBuffer::from_str() in src/text_buffer.rs
- [X] T010 Implement TextBuffer::insert() and TextBuffer::remove() with modification tracking in src/text_buffer.rs
- [X] T011 [P] Implement TextBuffer::line(), line_len(), len_lines(), len_chars() in src/text_buffer.rs
- [X] T012 [P] Implement TextBuffer::char_to_line() and line_to_char() coordinate conversion in src/text_buffer.rs
- [X] T013 [P] Implement TextBuffer::to_string(), is_modified(), clear_modified() in src/text_buffer.rs
- [X] T014 Implement Cursor::new(), line(), column() accessor methods in src/cursor.rs
- [X] T015 Implement Cursor::move_left(), move_right(), move_up(), move_down() with buffer validation in src/cursor.rs
- [X] T016 [P] Implement Cursor::move_to_line_start(), move_to_line_end(), clamp_to_line() in src/cursor.rs
- [X] T017 [P] Implement Cursor::to_char_index() and from_char_index() conversion in src/cursor.rs
- [X] T018 Implement Mode state predicates (is_normal, is_insert, is_command, is_search) in src/mode.rs
- [X] T019 [P] Implement Mode::get_prompt() returning display string in src/mode.rs
- [X] T020 [P] Implement Mode::append_char() and pop_char() for Command/Search modes in src/mode.rs
- [X] T021 [P] Implement Mode::command_string() and search_string() accessors in src/mode.rs
- [X] T022 Implement Viewport::new() and visible_range() in src/viewport.rs
- [X] T023 Implement Viewport::ensure_cursor_visible() with scroll_margin logic in src/viewport.rs
- [X] T024 [P] Implement Viewport::scroll_up() and scroll_down() in src/viewport.rs
- [X] T025 Update src/viewer.rs to accept TextBuffer reference instead of Vec&lt;String&gt;
- [X] T026 Update src/viewer.rs to render lines from TextBuffer using line() method
- [X] T027 [P] Create unit tests for TextBuffer in src/text_buffer.rs (insert, delete, coordinate conversion)
- [X] T028 [P] Create unit tests for Cursor in src/cursor.rs (movement, clamping, desired column)
- [X] T029 [P] Create unit tests for Mode in src/mode.rs (state transitions, string building)
- [X] T029a [P] Create `src/keybindings.rs` with in-memory KeyBindings struct mapping keys to actions (FR-041)
- [X] T029b Verify keybindings module is isolated and read-only during editor session (FR-042/FR-043)
- [X] T030 Run `cargo test` to verify foundational components work correctly

**Checkpoint**: Foundation ready - user story implementation can now begin

---

## Phase 3: User Story 1 - View File with Navigation (Priority: P1) 🎯 MVP

**Goal**: Enable vim-style navigation (hjkl, w/b, Ctrl+F/Ctrl+B) to move through file content without editing

**Independent Test**: Open a multi-screen file, use hjkl to move character-by-character, w/b to jump between words, Ctrl+F/Ctrl+B to page through content. Verify cursor position updates correctly and stays within bounds.

### Tests for User Story 1

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T031 [P] [US1] Create tests/integration/navigation_test.rs with test setup helpers
- [ ] T032 [P] [US1] Write test for hjkl character navigation in tests/integration/navigation_test.rs
- [ ] T033 [P] [US1] Write test for w/b word navigation in tests/integration/navigation_test.rs
- [ ] T034 [P] [US1] Write test for Ctrl+F/Ctrl+B page navigation in tests/integration/navigation_test.rs
- [ ] T035 [P] [US1] Write test for boundary conditions (first/last line, start/end of line) in tests/integration/navigation_test.rs
- [ ] T036 [US1] Run `cargo test navigation_test` and verify all tests FAIL (not yet implemented)

### Implementation for User Story 1

- [ ] T037 [P] [US1] Create src/navigation.rs with word boundary detection logic
- [ ] T038 [P] [US1] Create src/keybindings.rs with Action enum defining all possible actions
- [ ] T039 [US1] Implement find_next_word_start() function in src/navigation.rs
- [ ] T040 [US1] Implement find_prev_word_start() function in src/navigation.rs
- [ ] T041 [US1] Define KeyBinding struct and Normal mode bindings (hjkl, w, b, Ctrl+F, Ctrl+B) in src/keybindings.rs
- [ ] T042 [US1] Implement get_action_for_key() lookup function in src/keybindings.rs
- [ ] T043 [US1] Implement Viewport::page_up() moving viewport and cursor up by height-1 lines in src/viewport.rs
- [ ] T044 [US1] Implement Viewport::page_down() moving viewport and cursor down by height-1 lines in src/viewport.rs
- [ ] T045 [US1] Create src/editor_state.rs with EditorState struct (buffer, cursor, mode, viewport, file_path, status_message)
- [ ] T046 [US1] Implement EditorState::new() and load_file() in src/editor_state.rs
- [ ] T047 [US1] Implement EditorState::handle_key() with Normal mode navigation dispatch in src/editor_state.rs
- [ ] T048 [US1] Add hjkl navigation key handling (move_left/right/up/down) to handle_key() in src/editor_state.rs
- [ ] T049 [US1] Add w/b word navigation key handling calling navigation module to handle_key() in src/editor_state.rs
- [ ] T050 [US1] Add Ctrl+F/Ctrl+B page navigation key handling calling viewport methods to handle_key() in src/editor_state.rs
- [ ] T051 [US1] Implement boundary checks preventing cursor from moving beyond file limits in src/editor_state.rs
- [ ] T052 [US1] Update src/main.rs to use EditorState instead of simple viewer loop
- [ ] T053 [P] [US1] Create unit tests for navigation module in src/navigation.rs (word boundaries)
- [ ] T054 [US1] Run `cargo test navigation_test` to verify all navigation tests now PASS

**Checkpoint**: User Story 1 complete - navigation fully functional and testable independently

---

## Phase 4: User Story 2 - Edit Text in Insert Mode (Priority: P2)

**Goal**: Enable insert mode for adding, modifying, or deleting text, then returning to normal mode

**Independent Test**: Open file, press 'i' to enter insert mode (status bar shows "INSERT"), type text, use backspace to delete, press Escape to return to normal mode (status bar shows "NORMAL"). Verify text modifications are reflected in buffer.

### Tests for User Story 2

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T055 [P] [US2] Create tests/integration/editing_test.rs with test setup
- [ ] T056 [P] [US2] Write test for entering/exiting insert mode in tests/integration/editing_test.rs
- [ ] T057 [P] [US2] Write test for typing characters in insert mode in tests/integration/editing_test.rs
- [ ] T058 [P] [US2] Write test for backspace deletion in tests/integration/editing_test.rs
- [ ] T059 [P] [US2] Write test for inserting newlines in tests/integration/editing_test.rs
- [ ] T060 [P] [US2] Write test for rapid typing (100+ chars) without lag in tests/integration/editing_test.rs
- [ ] T061 [US2] Run `cargo test editing_test` and verify all tests FAIL (not yet implemented)

### Implementation for User Story 2

- [ ] T062 [P] [US2] Add Insert mode bindings ('i' to enter, Escape to exit, printable chars, backspace) to src/keybindings.rs
- [ ] T063 [US2] Implement mode transition from Normal to Insert on 'i' key in EditorState::handle_key() in src/editor_state.rs
- [ ] T064 [US2] Implement mode transition from Insert to Normal on Escape key in EditorState::handle_key() in src/editor_state.rs
- [ ] T065 [US2] Implement character insertion at cursor position in Insert mode in src/editor_state.rs
- [ ] T066 [US2] Update cursor position after character insertion (move right) in src/editor_state.rs
- [ ] T067 [US2] Implement backspace handling (delete char before cursor, move left) in Insert mode in src/editor_state.rs
- [ ] T068 [US2] Implement Enter key handling (insert newline, move to next line) in Insert mode in src/editor_state.rs
- [ ] T069 [US2] Ensure viewport scrolls to keep cursor visible during typing in src/editor_state.rs
- [ ] T070 [US2] Verify buffer modification flag is set after any edit operation in src/editor_state.rs
- [ ] T071 [US2] Run `cargo test editing_test` to verify all editing tests now PASS

**Checkpoint**: User Stories 1 AND 2 both work independently - can navigate and edit

---

## Phase 5: User Story 3 - Save Modified File (Priority: P3)

**Goal**: Persist changes to disk using `:w` command

**Independent Test**: Open file, enter insert mode, make changes, return to normal mode, type `:w`, press Enter. Verify status bar shows "File saved", modification flag cleared, and file on disk reflects changes.

### Tests for User Story 3

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T072 [P] [US3] Create tests/integration/save_test.rs with test setup and temp file helpers
- [ ] T073 [P] [US3] Write test for saving modified file with :w in tests/integration/save_test.rs
- [ ] T074 [P] [US3] Write test for saving unmodified file in tests/integration/save_test.rs
- [ ] T075 [P] [US3] Write test for verifying file contents after save in tests/integration/save_test.rs
- [ ] T076 [P] [US3] Write test for file write permission errors in tests/integration/save_test.rs
- [ ] T077 [P] [US3] Write test for saving large file (10MB+) without freezing in tests/integration/save_test.rs
- [ ] T078 [US3] Run `cargo test save_test` and verify all tests FAIL (not yet implemented)

### Implementation for User Story 3

- [ ] T079 [P] [US3] Add Command mode bindings (':' to enter, Enter to execute, Escape to cancel) to src/keybindings.rs
- [ ] T080 [US3] Implement mode transition from Normal to Command(':') on ':' key in src/editor_state.rs
- [ ] T081 [US3] Implement character appending to Command string when in Command mode in src/editor_state.rs
- [ ] T082 [US3] Implement backspace handling in Command mode (pop_char from mode string) in src/editor_state.rs
- [ ] T083 [US3] Implement command parsing and execution on Enter key in Command mode in src/editor_state.rs
- [ ] T084 [US3] Implement EditorState::save() method writing buffer to file_path in src/editor_state.rs
- [ ] T085 [US3] Add :w command handler calling save() method in src/editor_state.rs
- [ ] T086 [US3] Set status_message to "File saved" after successful save in src/editor_state.rs
- [ ] T087 [US3] Call buffer.clear_modified() after successful save in src/editor_state.rs
- [ ] T088 [US3] Handle "No changes to save" case when buffer is not modified in src/editor_state.rs
- [ ] T089 [US3] Handle file write errors and display error message in status bar in src/editor_state.rs
- [ ] T090 [US3] Update src/command.rs to integrate with :w command handling (if needed)
- [ ] T091 [US3] Run `cargo test save_test` to verify all save tests now PASS

**Checkpoint**: User Stories 1, 2, AND 3 all work independently - can navigate, edit, and save

---

## Phase 6: User Story 4 - Search for Text (Priority: P4)

**Goal**: Search for text pattern and jump to first occurrence

**Independent Test**: Open file with known content, type `/searchterm`, press Enter. Verify cursor jumps to first occurrence and status bar confirms "Found at line X" or shows "Pattern not found" if not found.

### Tests for User Story 4

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T092 [P] [US4] Create tests/integration/search_test.rs with test setup and sample files
- [ ] T093 [P] [US4] Write test for successful search and cursor jump in tests/integration/search_test.rs
- [ ] T094 [P] [US4] Write test for pattern not found in tests/integration/search_test.rs
- [ ] T095 [P] [US4] Write test for search with special characters in tests/integration/search_test.rs
- [ ] T096 [P] [US4] Write test for search with whitespace in pattern in tests/integration/search_test.rs
- [ ] T097 [P] [US4] Write test for search in large file (100k lines) completing within 2 seconds in tests/integration/search_test.rs
- [ ] T098 [P] [US4] Write test for canceling search with Escape in tests/integration/search_test.rs
- [ ] T099 [US4] Run `cargo test search_test` and verify all tests FAIL (not yet implemented)

### Implementation for User Story 4

- [ ] T100 [P] [US4] Create src/search.rs with search function signature
- [ ] T101 [P] [US4] Add Search mode bindings ('/' to enter, Enter to execute, Escape to cancel) to src/keybindings.rs
- [ ] T102 [US4] Implement literal string search returning Option&lt;char_idx&gt; in src/search.rs
- [ ] T103 [US4] Optimize search for large files (100k+ lines) to avoid freezing in src/search.rs
- [ ] T104 [US4] Implement mode transition from Normal to Search('/') on '/' key in src/editor_state.rs
- [ ] T105 [US4] Implement character appending to Search string when in Search mode in src/editor_state.rs
- [ ] T106 [US4] Implement backspace handling in Search mode (pop_char from mode string) in src/editor_state.rs
- [ ] T107 [US4] Implement search execution on Enter key in Search mode calling search module in src/editor_state.rs
- [ ] T108 [US4] Update cursor position and viewport when match found in src/editor_state.rs
- [ ] T109 [US4] Display "Pattern not found" status message when no match in src/editor_state.rs
- [ ] T110 [US4] Display "Found at line X" status message when match found in src/editor_state.rs
- [ ] T111 [US4] Handle special characters and whitespace in search patterns in src/search.rs
- [ ] T112 [P] [US4] Create unit tests for search module in src/search.rs (found, not found, edge cases)
- [ ] T113 [US4] Run `cargo test search_test` to verify all search tests now PASS

**Checkpoint**: User Stories 1-4 all work independently - full navigation, editing, saving, and search

---

## Phase 7: User Story 5 - Jump to Specific Line (Priority: P5)

**Goal**: Navigate directly to a specific line number using `:` command

**Independent Test**: Open file with 100+ lines, type `:50`, press Enter. Verify cursor jumps to line 50 and status bar confirms position. Test boundary cases (`:1`, `:999`).

### Tests for User Story 5

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T114 [P] [US5] Create tests/integration/goto_line_test.rs with test setup and multi-line files
- [ ] T115 [P] [US5] Write test for jumping to valid line number in tests/integration/goto_line_test.rs
- [ ] T116 [P] [US5] Write test for jumping to first line (:1) in tests/integration/goto_line_test.rs
- [ ] T117 [P] [US5] Write test for jumping to last line in tests/integration/goto_line_test.rs
- [ ] T118 [P] [US5] Write test for line number exceeding file length in tests/integration/goto_line_test.rs
- [ ] T119 [P] [US5] Write test for invalid input (non-numeric) in tests/integration/goto_line_test.rs
- [ ] T120 [US5] Run `cargo test goto_line_test` and verify all tests FAIL (not yet implemented)

### Implementation for User Story 5

- [ ] T121 [US5] Implement command parsing for `:{number}` pattern in src/editor_state.rs
- [ ] T122 [US5] Implement EditorState::goto_line() method setting cursor to target line in src/editor_state.rs
- [ ] T123 [US5] Add :{number} command handler calling goto_line() in src/editor_state.rs
- [ ] T124 [US5] Handle line number validation (parse numeric string) in src/editor_state.rs
- [ ] T125 [US5] Clamp line number to valid range (1 to len_lines) in src/editor_state.rs
- [ ] T126 [US5] Display "Invalid line number" for non-numeric input in src/editor_state.rs
- [ ] T127 [US5] Display "Line number out of range, moved to last line" when exceeding file length in src/editor_state.rs
- [ ] T128 [US5] Update viewport to ensure target line is visible after jump in src/editor_state.rs
- [ ] T129 [US5] Run `cargo test goto_line_test` to verify all goto_line tests now PASS

**Checkpoint**: User Stories 1-5 all work independently - complete navigation suite

---

## Phase 8: User Story 6 - Monitor Current Mode (Priority: P6)

**Goal**: Always display current mode in status bar for user awareness

**Independent Test**: Switch between modes (Normal, Insert, Command, Search) and verify status bar immediately updates to show "NORMAL", "INSERT", ":{command}", or "/{search}".

### Tests for User Story 6

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T130 [P] [US6] Create tests/integration/status_bar_test.rs with test setup
- [ ] T131 [P] [US6] Write test for mode display updates in tests/integration/status_bar_test.rs
- [ ] T132 [P] [US6] Write test for cursor position display in tests/integration/status_bar_test.rs
- [ ] T133 [P] [US6] Write test for modification indicator in tests/integration/status_bar_test.rs
- [ ] T134 [P] [US6] Write test for command input display in tests/integration/status_bar_test.rs
- [ ] T135 [P] [US6] Write test for search input display in tests/integration/status_bar_test.rs
- [ ] T136 [US6] Run `cargo test status_bar_test` and verify all tests FAIL (not yet implemented)

### Implementation for User Story 6

- [ ] T137 [P] [US6] Create src/status_bar.rs with format_status_line() function
- [ ] T138 [US6] Implement status bar layout: mode | file_path | line,col | modified flag in src/status_bar.rs
- [ ] T139 [US6] Implement mode display logic calling Mode::get_prompt() in src/status_bar.rs
- [ ] T140 [US6] Implement cursor position display (1-indexed line and column) in src/status_bar.rs
- [ ] T141 [US6] Implement modification indicator ("[+]" when modified) in src/status_bar.rs
- [ ] T142 [US6] Implement command/search input display in status bar in src/status_bar.rs
- [ ] T143 [US6] Implement EditorState::render() method generating (content_lines, status_line) in src/editor_state.rs
- [ ] T144 [US6] Integrate status_bar module into render() method in src/editor_state.rs
- [ ] T145 [US6] Update src/main.rs event loop to display status bar at bottom of screen
- [ ] T146 [US6] Ensure status bar updates immediately (within 16ms) when mode changes in src/editor_state.rs
- [ ] T147 [US6] Reserve last line of terminal for status bar in rendering logic in src/viewer.rs
- [ ] T148 [US6] Run `cargo test status_bar_test` to verify all status bar tests now PASS

**Checkpoint**: All 6 user stories complete and independently functional

---

## Phase 9: Polish & Integration

**Purpose**: Final refinements and validation across all features

- [ ] T149 [P] Add comprehensive inline documentation to all public APIs in src/
- [ ] T150 [P] Add module-level documentation explaining architecture in src/
- [ ] T151 Run `cargo clippy` and fix all warnings
- [ ] T152 Run `cargo fmt` to format all code
- [ ] T153 Run full test suite `cargo test` and verify all tests pass
- [ ] T154 Test navigation with large file (10k+ lines) and verify no performance degradation
- [ ] T155 Test editing with rapid typing (120+ chars/min) and verify no lag
- [ ] T156 Test saving large file (100+ MB) and verify no freezing
- [ ] T157 Test search in large file (100k+ lines) and verify completes within 2 seconds
- [ ] T158 Update README.md with new features and usage examples
- [ ] T159 Validate all acceptance scenarios from spec.md manually
- [ ] T160 Run quickstart.md validation to ensure examples work correctly
- [ ] T161 Create release build with `cargo build --release`
- [ ] T162 Perform manual testing session covering all user stories

**Checkpoint**: Phase 002 complete and ready for production use

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3-8)**: All depend on Foundational phase completion
  - User stories CAN proceed in parallel (if staffed)
  - Or sequentially in priority order (US1 → US2 → US3 → US4 → US5 → US6)
- **Polish (Phase 9)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Depends on Foundational - No dependencies on other stories
- **User Story 2 (P2)**: Depends on Foundational - No dependencies on other stories (but typically done after US1)
- **User Story 3 (P3)**: Depends on Foundational and US2 (needs modification tracking from editing)
- **User Story 4 (P4)**: Depends on Foundational and US1 (uses navigation to jump to results)
- **User Story 5 (P5)**: Depends on Foundational and US1 (uses navigation to jump to line)
- **User Story 6 (P6)**: Depends on all other user stories (integrates with all modes)

### Within Each User Story

- **Tests FIRST**: Write all tests before implementation (TDD approach)
- Verify tests FAIL initially (red phase)
- Implement features to make tests pass (green phase)
- Refactor and optimize (refactor phase)
- Core module creation before implementation
- Unit tests can run in parallel with implementation
- Integration tests verify complete functionality

### Parallel Opportunities

- **Setup Phase**: All tasks except T003-T004 (which depend on T001-T002)
- **Foundational Phase**: 
  - T005-T008 can run in parallel (creating new modules)
  - T011, T012, T013 can run after T010 in parallel
  - T016, T017 can run after T015 in parallel
  - T019, T020, T021 can run after T018 in parallel
  - T027-T029 unit tests can run in parallel
- **Within Each User Story**:
  - Test creation tasks marked [P] can run in parallel
  - File creation tasks marked [P] can run in parallel
  - Implementation tasks often sequential (build on each other)
- **Polish Phase**: Most tasks marked [P] can run in parallel

---

## Parallel Example: User Story 1 (Navigation)

```bash
# Launch all tests together (BEFORE implementation):
Task T031: "Create tests/integration/navigation_test.rs"
Task T032: "Test hjkl navigation"
Task T033: "Test w/b word navigation"
Task T034: "Test Ctrl+F/B page navigation"
Task T035: "Test boundary conditions"

# Verify tests fail: cargo test navigation_test

# Launch initial modules together:
Task T037: "Create src/navigation.rs"
Task T038: "Create src/keybindings.rs"

# After implementation complete, verify tests pass:
Task T054: "Run cargo test navigation_test" → should now PASS
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1 (Navigation)
4. **STOP and VALIDATE**: Test navigation independently
5. Demo/validate before proceeding

### Incremental Delivery (TDD Approach)

1. Complete Setup + Foundational → Foundation ready
2. User Story 1:
   - Write ALL tests → Verify FAIL → Implement → Verify PASS → Demo (Navigation MVP!)
3. User Story 2:
   - Write ALL tests → Verify FAIL → Implement → Verify PASS → Demo (Can edit!)
4. User Story 3:
   - Write ALL tests → Verify FAIL → Implement → Verify PASS → Demo (Can save!)
5. User Story 4:
   - Write ALL tests → Verify FAIL → Implement → Verify PASS → Demo (Can search!)
6. User Story 5:
   - Write ALL tests → Verify FAIL → Implement → Verify PASS → Demo (Go to line!)
7. User Story 6:
   - Write ALL tests → Verify FAIL → Implement → Verify PASS → Demo (Full status bar!)
8. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers (after Foundational phase):

**Option 1: Sequential (Safer)**
1. Complete User Story 1 first (foundation for others)
2. Then parallelize:
   - Developer A: User Story 2 (Insert mode)
   - Developer B: User Story 4 (Search)
   - Developer C: User Story 6 (Status bar foundation)

**Option 2: Maximum Parallelization**
1. Developer A: User Story 1 (Navigation)
2. Developer B: User Story 2 (Insert mode) + User Story 3 (Save)
3. Developer C: User Story 4 (Search) + User Story 5 (Go-to-line)
4. Developer D: User Story 6 (Status bar)

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- **TDD Approach**: Tests written BEFORE implementation for each user story
- Each user story should be independently completable and testable
- Verify tests fail before implementing (red → green → refactor cycle)
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Constitutional compliance: 5/5 dependencies - NO MORE can be added
- Performance critical: No operation should freeze the editor (60fps target)
- All coordinate conversions go through TextBuffer abstraction
- Only text_buffer.rs imports ropey - abstraction maintained

---

**Total Tasks**: 162 tasks
- Setup: 4 tasks
- Foundational: 26 tasks  
- User Story 1 (Navigation): 24 tasks (6 test + 18 implementation)
- User Story 2 (Insert Mode): 17 tasks (7 test + 10 implementation)
- User Story 3 (File Saving): 20 tasks (7 test + 13 implementation)
- User Story 4 (Search): 22 tasks (8 test + 14 implementation)
- User Story 5 (Go-to-Line): 16 tasks (7 test + 9 implementation)
- User Story 6 (Status Bar): 19 tasks (7 test + 12 implementation)
- Polish: 14 tasks

**Test Tasks**: 42 test tasks (ensuring quality and correctness)
**Implementation Tasks**: 120 implementation tasks

**Estimated Parallel Opportunities**: ~45 tasks can run in parallel with proper team coordination

**Suggested MVP Scope**: Phase 1 + Phase 2 + Phase 3 (User Story 1 - Navigation only) = 54 tasks

**Test Coverage**: All 6 user stories have comprehensive integration tests + unit tests for core modules
