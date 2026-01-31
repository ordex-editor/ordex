# Tasks: MVP Viewer

**Input**: Design documents from `.speckit/001-mvp-viewer/`
**Prerequisites**: plan.md (implementation structure), spec.md (functional requirements)

**Organization**: Tasks are organized by functional requirement to enable incremental implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which functional requirement this task belongs to (e.g., FR1, FR2, FR3, FR4)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Project Initialization)

**Purpose**: Create project structure and initialize dependencies

- [ ] T001 Create Cargo project at repository root with name "ordex"
- [ ] T002 Add termion 4.0.6 dependency to Cargo.toml
- [ ] T003 Verify dependency count (should be 3 runtime deps: termion, libc, numtoa)
- [ ] T004 Create src/ directory structure per plan.md

---

## Phase 2: Foundational (Core Infrastructure)

**Purpose**: Terminal handling infrastructure that all features depend on

**⚠️ CRITICAL**: Terminal infrastructure must be complete before implementing any functional requirements

- [ ] T005 [P] Implement terminal wrapper in src/tui.rs with RAII cleanup (Drop trait)
- [ ] T006 [P] Implement raw mode enter/exit functions in src/tui.rs
- [ ] T007 [P] Implement screen clear function in src/tui.rs
- [ ] T008 [P] Implement key reading function in src/tui.rs
- [ ] T009 [P] Implement write_at function for positioning in src/tui.rs
- [ ] T010 Add unit tests for tui module in src/tui.rs #[cfg(test)] section

**Checkpoint**: Terminal infrastructure ready - functional requirements can now be implemented

---

## Phase 3: FR-1 File Opening (Priority: P1) 🎯 MVP

**Goal**: Accept a file path as CLI argument and load its contents

**Independent Test**: Run `cargo run -- test.txt` with existing file, verify it loads without error

### Implementation for FR-1

- [ ] T011 [FR1] Implement CLI argument parsing in src/main.rs
- [ ] T012 [FR1] Add usage message display for no arguments in src/main.rs
- [ ] T013 [FR1] Add file existence check and error handling in src/main.rs
- [ ] T014 [FR1] Implement file reading into Vec<String> in src/main.rs
- [ ] T015 [FR1] Add unit tests for file loading in src/main.rs #[cfg(test)] section
- [ ] T016 [FR1] Add integration test for CLI args in tests/integration/cli_test.rs

**Checkpoint**: File loading complete and tested - can load and store file contents

---

## Phase 4: FR-2 Terminal Display (Priority: P1) 🎯 MVP

**Goal**: Render file content in terminal with proper viewport handling

**Independent Test**: Run with a file, verify content displays correctly within terminal bounds

### Implementation for FR-2

- [ ] T017 [P] [FR2] Create viewer module in src/viewer.rs
- [ ] T018 [P] [FR2] Implement get_visible_lines function for viewport in src/viewer.rs
- [ ] T019 [FR2] Implement render function for displaying lines in src/viewer.rs (depends on T017, T018)
- [ ] T020 [FR2] Add terminal width truncation logic in src/viewer.rs
- [ ] T021 [FR2] Reserve bottom line for command input in src/viewer.rs
- [ ] T022 [FR2] Add unit tests for viewport logic in src/viewer.rs #[cfg(test)] section
- [ ] T023 [FR2] Integrate viewer with main loop in src/main.rs

**Checkpoint**: Display rendering complete - file content visible in terminal

---

## Phase 5: FR-3 Command Input (Priority: P1) 🎯 MVP

**Goal**: Accept and parse vim-style colon commands

**Independent Test**: Press `:`, type characters, verify they appear; press Escape, verify command clears

### Implementation for FR-3

- [ ] T024 [P] [FR3] Create command module in src/command.rs
- [ ] T025 [P] [FR3] Implement CommandMode struct with buffer in src/command.rs
- [ ] T026 [FR3] Add colon detection in main event loop in src/main.rs (depends on T024)
- [ ] T027 [FR3] Implement character append to command buffer in src/command.rs
- [ ] T028 [FR3] Add command display at bottom line in src/command.rs
- [ ] T029 [FR3] Implement Enter key for command execution in src/command.rs
- [ ] T030 [FR3] Implement Escape key for command cancellation in src/command.rs
- [ ] T031 [FR3] Add unknown command error handling in src/command.rs
- [ ] T032 [FR3] Add unit tests for command parsing in src/command.rs #[cfg(test)] section

**Checkpoint**: Command input system complete - can enter and cancel commands

---

## Phase 6: FR-4 Quit Command (Priority: P1) 🎯 MVP

**Goal**: Exit cleanly when `:q` is entered

**Independent Test**: Open file, type `:q` and Enter, verify clean exit with status code 0

### Implementation for FR-4

- [ ] T033 [FR4] Implement quit command handler in src/command.rs
- [ ] T034 [FR4] Add terminal restoration on exit in src/main.rs
- [ ] T035 [FR4] Verify terminal restoration on panic in src/tui.rs Drop implementation
- [ ] T036 [FR4] Test exit status code in tests/integration/cli_test.rs
- [ ] T037 [FR4] Add integration test for full quit workflow in tests/integration/cli_test.rs

**Checkpoint**: MVP complete - all functional requirements implemented and working

---

## Phase 7: Polish & Quality Assurance

**Purpose**: Final validation, documentation, and quality improvements

- [ ] T038 [P] Add error handling for IO operations throughout codebase
- [ ] T039 [P] Add comprehensive documentation comments to all public functions
- [ ] T040 [P] Create README.md with usage instructions at repository root
- [ ] T041 Run cargo clippy and fix all warnings
- [ ] T042 Run cargo fmt to ensure consistent formatting
- [ ] T043 Manual testing: Test with various file sizes and terminal dimensions
- [ ] T044 Manual testing: Test error cases (missing file, invalid args, etc.)
- [ ] T045 Verify dependency count hasn't exceeded constitution limit (≤5)
- [ ] T046 Final integration test run with cargo test
- [ ] T047 Update .speckit/001-mvp-viewer/plan.md with any architecture changes

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup (Phase 1) completion - BLOCKS all functional requirements
- **FR-1 File Opening (Phase 3)**: Depends on Foundational (Phase 2) - can start once terminal infrastructure ready
- **FR-2 Terminal Display (Phase 4)**: Depends on FR-1 (Phase 3) - needs file content loaded
- **FR-3 Command Input (Phase 5)**: Depends on FR-2 (Phase 4) - needs display infrastructure
- **FR-4 Quit Command (Phase 6)**: Depends on FR-3 (Phase 5) - needs command parsing
- **Polish (Phase 7)**: Depends on FR-4 (Phase 6) completion - can only polish when MVP is feature-complete

### Critical Path

```
Setup → Foundational → File Opening → Display → Command Input → Quit → Polish
(P1)    (P2)           (P3)           (P4)      (P5)           (P6)    (P7)
```

### Parallel Opportunities Within Phases

**Phase 2 (Foundational)**: Tasks T005-T009 can run in parallel (different functions in same file, independent implementations)

**Phase 4 (Terminal Display)**: Tasks T017-T018 can run in parallel (different functions)

**Phase 5 (Command Input)**: Tasks T024-T025 can run in parallel (different functions)

**Phase 7 (Polish)**: Tasks T038-T040 can run in parallel (different files/aspects)

### Within Each Functional Requirement

- Core infrastructure before feature implementation
- Feature logic before integration with main loop
- Unit tests alongside implementation
- Integration tests after feature completion
- Each FR should be independently testable before moving to next

---

## Parallel Example: Foundational Phase

```bash
# Launch all terminal infrastructure functions in parallel:
Task: "Implement raw mode enter/exit functions in src/tui.rs"
Task: "Implement screen clear function in src/tui.rs"
Task: "Implement key reading function in src/tui.rs"
Task: "Implement write_at function for positioning in src/tui.rs"

# Then integrate with RAII wrapper:
Task: "Implement terminal wrapper in src/tui.rs with RAII cleanup"
```

---

## Parallel Example: Terminal Display Phase

```bash
# Launch viewer functions in parallel:
Task: "Create viewer module in src/viewer.rs"
Task: "Implement get_visible_lines function for viewport in src/viewer.rs"

# Then implement render function that uses both:
Task: "Implement render function for displaying lines in src/viewer.rs"
```

---

## Implementation Strategy

### MVP First (All Functional Requirements)

This MVP is atomic - all functional requirements are needed for a usable product:

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all features)
3. Complete Phase 3: File Opening (FR-1)
4. Complete Phase 4: Terminal Display (FR-2)
5. Complete Phase 5: Command Input (FR-3)
6. Complete Phase 6: Quit Command (FR-4)
7. **VALIDATE**: Full end-to-end test - open file, view it, quit cleanly
8. Complete Phase 7: Polish

### Validation Checkpoints

After each phase, verify:

- **Phase 2**: Terminal can enter/exit raw mode cleanly
- **Phase 3**: Files load successfully, errors handled gracefully
- **Phase 4**: File content displays correctly in terminal
- **Phase 5**: Can enter commands, see input, cancel with Escape
- **Phase 6**: `:q` exits cleanly, terminal restored
- **Phase 7**: All tests pass, code is clean and documented

---

## Notes

- [P] tasks = independent implementations, can be done in parallel
- [FR#] label maps task to specific functional requirement for traceability
- MVP is intentionally minimal - no editing, cursor movement, or advanced features
- Focus on clean terminal handling and proper cleanup
- All termion code isolated in src/tui.rs for future library changes
- Constitution compliance: exactly 3 runtime dependencies (termion, libc, numtoa)
- Commit after each task or logical group of related tasks
- Integration tests in tests/integration/ verify end-to-end behavior
- Unit tests in #[cfg(test)] modules verify individual components
