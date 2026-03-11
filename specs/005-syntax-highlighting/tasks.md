# Tasks: Syntax Highlighting

**Input**: Design documents from `/specs/005-syntax-highlighting/`  
**Prerequisites**: `plan.md` (required), `spec.md` (required), `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: Include test tasks because `plan.md`, `contracts/`, and `quickstart.md` explicitly require unit and integration coverage for risky lexer, rendering, and large-file behavior.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Task can run in parallel (different files, no dependency on incomplete tasks)
- **[Story]**: User story label (`[US1]`, `[US2]`, `[US3]`) for story-phase tasks only
- All tasks include explicit file paths

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the syntax-highlighting module layout, fixtures, and test entry points.

- [ ] T001 Create syntax subsystem entry points in `src/syntax.rs` and add module declarations in `src/main.rs`
- [ ] T002 [P] Create syntax subsystem module files in `src/syntax/profile.rs`, `src/syntax/helpers.rs`, `src/syntax/engine.rs`, `src/syntax/profile_tests.rs`, `src/syntax/profiles/mod.rs`, `src/syntax/profiles/rust.rs`, `src/syntax/profiles/toml.rs`, `src/syntax/profiles/markdown.rs`, and `src/syntax/profiles/d.rs`
- [ ] T003 [P] Create syntax integration test files in `tests/syntax_highlighting_test.rs` and `tests/syntax_large_file_test.rs`
- [ ] T004 [P] Create representative syntax fixtures for supported, unsupported, and irregular documents in `tests/fixtures/syntax/`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build the shared syntax metadata, engine scaffolding, and editor plumbing required by all user stories.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T005 Implement shared syntax enums, modifiers, comment-style types, and `LanguageProfile` data structures in `src/syntax/profile.rs`
- [ ] T006 [P] Implement shared helper predicates for delimiter boundaries and context-sensitive matches in `src/syntax/helpers.rs`
- [ ] T007 [P] Implement built-in profile registry and filename/extension detection in `src/syntax/profiles/mod.rs`
- [ ] T008 [P] Implement built-in Rust and D comment/doc-comment metadata in `src/syntax/profiles/rust.rs` and `src/syntax/profiles/d.rs`
- [ ] T009 [P] Implement built-in config/TOML and Markdown profile metadata in `src/syntax/profiles/toml.rs` and `src/syntax/profiles/markdown.rs`
- [ ] T010 Implement `HighlightSpan`, `LineLexState`, `DocumentHighlightState`, `BufferEdit`, and `SyntaxEngine` scaffolding in `src/syntax/engine.rs`
- [ ] T011 Implement editor-owned syntax state plumbing and syntax-generation accessors in `src/editor_state.rs`, `src/main.rs`, and `src/tui.rs`

**Checkpoint**: Shared syntax infrastructure is ready; user stories can now build on one engine and one profile-per-file layout.

---

## Phase 3: User Story 1 - Read supported code faster (Priority: P1) 🎯 MVP

**Goal**: Automatically highlight supported Rust, config/TOML, Markdown, and D files on open so users can scan them immediately.

**Independent Test**: Open representative supported files and verify ANSI-rendered comments, strings, numbers, keywords, punctuation, and core Markdown constructs are visually distinct before any edits.

### Tests for User Story 1

- [ ] T012 [P] [US1] Add shared unit tests for language detection, profile metadata, and doc-comment classification in `src/syntax/profile_tests.rs`
- [ ] T013 [P] [US1] Add integration tests for open-time ANSI highlighting of Rust, config/TOML, Markdown, and D fixtures in `tests/syntax_highlighting_test.rs`

### Implementation for User Story 1

- [ ] T014 [P] [US1] Implement Rust and config/TOML lex rules for keywords, strings, numbers, punctuation, and comments in `src/syntax/profiles/rust.rs` and `src/syntax/profiles/toml.rs`
- [ ] T015 [P] [US1] Implement D and conservative-core Markdown lex rules, including documentation comments, in `src/syntax/profiles/d.rs` and `src/syntax/profiles/markdown.rs`
- [ ] T016 [US1] Implement full-document open-time lexing and span-cache population in `src/syntax/engine.rs`
- [ ] T017 [US1] Initialize syntax state during file load and config reload in `src/editor_state.rs`
- [ ] T018 [US1] Apply syntax spans to wrapped and unwrapped row rendering in `src/main.rs` and `src/tui.rs`

**Checkpoint**: User Story 1 is fully functional and independently testable.

---

## Phase 4: User Story 2 - Keep highlighting correct while editing large files (Priority: P2)

**Goal**: Keep full-document highlighting correct and responsive while editing and scrolling large supported files.

**Independent Test**: Open a supported 50,000-line file, perform inserts/deletes around multiline constructs, scroll through the document, and verify full-document correctness without freezing the editor.

### Tests for User Story 2

- [ ] T019 [P] [US2] Add unit tests for line-state stabilization and multiline delimiter recovery in `src/syntax/engine.rs`
- [ ] T020 [P] [US2] Add integration tests for 50,000-line open/edit/scroll correctness in `tests/syntax_large_file_test.rs`
- [ ] T021 [P] [US2] Add edit-triggered invalidation and wrapped-row boundary regressions in `tests/editing_test.rs` and `tests/soft_wrap_test.rs`

### Implementation for User Story 2

- [ ] T022 [P] [US2] Emit precise dirty-line and edit-range updates from insert/delete workflows in `src/editor_state.rs`
- [ ] T023 [US2] Implement synchronous forward-to-stability relexing for edited regions in `src/syntax/engine.rs`
- [ ] T024 [US2] Trigger redraw decisions from syntax-generation changes in `src/main.rs` and `src/editor_state.rs`
- [ ] T025 [US2] Preserve correct highlight clipping through soft-wrap and horizontal-scroll rendering paths in `src/main.rs` and `src/tui.rs`
- [ ] T026 [US2] Optimize large-file lex hot paths and cache reuse to meet the phase targets in `src/syntax/engine.rs` and `tests/syntax_large_file_test.rs`

**Checkpoint**: User Stories 1 and 2 both work independently, including the large-file editing path.

---

## Phase 5: User Story 3 - Fail safely on mixed or unsupported documents (Priority: P3)

**Goal**: Fall back conservatively on unsupported, mixed, and irregular documents so the buffer stays readable and trustworthy.

**Independent Test**: Open unsupported file types, Markdown edge cases, and irregular syntax fixtures, then verify that unsupported regions remain plain/readable instead of being aggressively miscolored.

### Tests for User Story 3

- [ ] T027 [P] [US3] Add fallback-rendering tests for unsupported files and detection misses in `tests/syntax_highlighting_test.rs`
- [ ] T028 [P] [US3] Add shared Markdown edge-case unit tests plus integration coverage for unsupported constructs and punctuation-heavy prose in `src/syntax/profile_tests.rs` and `tests/syntax_highlighting_test.rs`

### Implementation for User Story 3

- [ ] T029 [P] [US3] Implement plain-text fallback and unsupported-profile handling in `src/syntax/engine.rs` and `src/syntax/profiles/mod.rs`
- [ ] T030 [P] [US3] Implement helper-driven boundary guards for conservative Markdown constructs in `src/syntax/helpers.rs` and `src/syntax/profiles/markdown.rs`
- [ ] T031 [US3] Keep unsupported advanced Markdown constructs and mixed embedded regions in plain spans in `src/syntax/profiles/markdown.rs` and `src/syntax/engine.rs`
- [ ] T032 [US3] Ensure fallback rendering stays visually identical to unhighlighted text in `src/main.rs`, `src/tui.rs`, and `tests/syntax_highlighting_test.rs`
- [ ] T033 [US3] Add irregular-syntax and mixed-document fixtures plus validation coverage in `tests/fixtures/syntax/` and `tests/syntax_highlighting_test.rs`

**Checkpoint**: All user stories are independently functional and safe fallback behavior is covered.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Finish documentation, validation, and cross-story cleanup.

- [ ] T034 [P] Document supported languages, documentation comments, fallback behavior, and Markdown limits in `docs/src/syntax-highlighting.md`
- [ ] T035 [P] Update syntax-highlighting navigation and overview in `docs/src/SUMMARY.md` and `docs/src/index.md`
- [ ] T036 [P] Validate and align implementation guidance in `specs/005-syntax-highlighting/quickstart.md`
- [ ] T037 Verify that no new runtime dependencies were introduced in `Cargo.toml`, `Cargo.lock`, and `specs/005-syntax-highlighting/plan.md`
- [ ] T038 Run repository validation commands and fix remaining syntax-highlighting issues in `src/syntax/*.rs`, `src/syntax/profiles/*.rs`, `src/editor_state.rs`, `src/main.rs`, `src/tui.rs`, and `tests/*.rs`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies
- **Phase 2 (Foundational)**: Depends on Phase 1 and blocks all user stories
- **Phase 3 (US1)**: Depends on Phase 2 and delivers the MVP
- **Phase 4 (US2)**: Depends on Phase 3 because edit-time correctness extends the baseline highlighting pipeline
- **Phase 5 (US3)**: Depends on Phase 3 because safe fallback builds on the baseline highlighting pipeline
- **Phase 6 (Polish)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1 (P1)**: First independently deliverable slice after foundational work
- **US2 (P2)**: Builds on US1's open-time highlighting pipeline and adds edit/large-file correctness
- **US3 (P3)**: Builds on US1's open-time highlighting pipeline and adds conservative fallback behavior

### Story Completion Order

```text
Setup -> Foundational -> US1 -> { US2, US3 } -> Polish
```

### Within Each User Story

- Tests first, and they should fail before implementation begins
- Shared profile/rule code before engine integration
- Engine integration before editor/render integration
- Story checkpoint validation before moving to the next priority when working sequentially

---

## Parallel Execution Examples

### User Story 1

```bash
Task: T012 [US1] Add unit tests for language detection and profile metadata
Task: T013 [US1] Add integration tests for open-time ANSI highlighting

Task: T014 [US1] Implement Rust and config/TOML lex rules
Task: T015 [US1] Implement D and Markdown lex rules
```

### User Story 2

```bash
Task: T019 [US2] Add unit tests for line-state stabilization
Task: T020 [US2] Add integration tests for 50,000-line behavior
Task: T021 [US2] Add edit and soft-wrap regressions

Task: T022 [US2] Emit dirty-line and edit-range updates
Task: T026 [US2] Optimize large-file lex hot paths
```

### User Story 3

```bash
Task: T027 [US3] Add fallback-rendering tests
Task: T028 [US3] Add conservative Markdown edge-case tests

Task: T029 [US3] Implement plain-text fallback handling
Task: T030 [US3] Implement helper-driven Markdown guards
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1
4. **STOP AND VALIDATE**: Open supported Rust, config/TOML, Markdown, and D files and confirm highlighting on first load

### Incremental Delivery

1. Deliver US1 for open-time readability
2. Add US2 for edit-time and large-file correctness
3. Add US3 for conservative fallback and irregular-document safety
4. Finish with docs, dependency verification, and repository-wide validation

### Parallel Team Strategy

1. Team completes Setup and Foundational together
2. One engineer finishes US1 to establish the baseline pipeline
3. After US1, separate engineers can work on US2 and US3 in parallel

---

## Notes

- `[P]` tasks touch different files or independent surfaces and can be worked in parallel
- `[US1]`, `[US2]`, and `[US3]` map directly to the clarified user stories in `spec.md`
- The MVP scope is User Story 1 only
- The plan assumes a single-threaded lexer in phase 1; revisit threading only if profiling proves the large-file targets cannot be met
- Markdown stays on the generic highlighting engine via helper predicates rather than a separate lexer architecture
