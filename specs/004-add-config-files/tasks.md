# Tasks: Resilient Configuration Files

**Input**: Design documents from `/specs/004-add-config-files/`  
**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/, quickstart.md

**Tests**: Include test tasks because the specification defines independent test criteria and acceptance scenarios per user story.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Task can run in parallel (different files, no dependency on incomplete tasks)
- **[Story]**: User story label (`[US1]`, `[US2]`, `[US3]`) for story-phase tasks only
- All tasks include explicit file paths

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Initialize config subsystem skeleton and shared test fixtures.

- [X] T001 Create config module entry points in `src/config.rs` and module declarations in `src/main.rs`
- [X] T002 [P] Create config fixture directory and baseline files in `tests/fixtures/config/`
- [X] T003 [P] Add shared config test helpers in `tests/config_test_support.rs`
- [X] T004 Create config subsystem file stubs in `src/config/parser.rs`, `src/config/include_loader.rs`, `src/config/validator.rs`, `src/config/loader.rs`, `src/config/warnings.rs`, and `src/config/keymap_merge.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build core parser/load primitives required by all user stories.

**⚠️ CRITICAL**: User story implementation starts only after this phase is complete.

- [X] T005 Implement parser AST and diagnostic structures in `src/config/parser.rs`
- [X] T006 Implement tokenizer and line parser with `#` comment stripping outside strings in `src/config/parser.rs`
- [X] T007 [P] Implement include source discovery/read helpers with recoverable errors in `src/config/include_loader.rs`
- [X] T008 [P] Implement warning event model and stderr rendering helpers in `src/config/warnings.rs`
- [X] T009 Implement config loader public API contracts (`load` and `validate`) in `src/config/loader.rs` and re-export from `src/config.rs`
- [X] T010 Wire loader invocation with safe defaults into startup flow in `src/main.rs` and `src/editor_state.rs`

**Checkpoint**: Foundation complete; user stories can proceed.

---

## Phase 3: User Story 1 - Configure behavior with files (Priority: P1) 🎯 MVP

**Goal**: Load valid configuration files and apply known settings on startup.

**Independent Test**: Create a minimal valid config file, start the app, and verify configured behavior is applied without manual post-start edits.

### Tests for User Story 1

- [X] T011 [P] [US1] Add parser unit tests for valid sections, values, and `#` comments in `src/config/parser.rs`
- [X] T012 [P] [US1] Add integration test for applying valid config settings at startup in `tests/config_loading_test.rs`

### Implementation for User Story 1

- [X] T013 [P] [US1] Implement known setting schema and defaults in `src/config/validator.rs`
- [X] T014 [US1] Implement known value normalization and type conversion in `src/config/validator.rs`
- [X] T015 [US1] Implement section merge logic to apply valid settings and default missing values in `src/config/loader.rs`
- [X] T016 [US1] Apply loaded runtime settings to editor state initialization in `src/editor_state.rs`
- [X] T017 [US1] Emit startup load summary for applied/defaulted settings in `src/config/loader.rs` and `src/config/warnings.rs`

**Checkpoint**: US1 is fully functional and independently testable.

---

## Phase 4: User Story 2 - Keep key mappings usable on partial failure (Priority: P2)

**Goal**: Preserve valid key mappings even when unrelated config sections are invalid.

**Independent Test**: Provide config where non-key-mapping content is invalid but key mappings are valid, then verify key mappings remain available.

### Tests for User Story 2

- [X] T018 [P] [US2] Add integration test for keymap survival under unrelated section failure in `tests/config_keymap_resilience_test.rs`
- [X] T019 [P] [US2] Add unit tests for duplicate keymap conflict policy (last-definition-wins) in `src/config/keymap_merge.rs`

### Implementation for User Story 2

- [X] T020 [P] [US2] Implement key mapping section parsing/validation independent of other sections in `src/config/validator.rs`
- [X] T021 [US2] Implement deterministic key mapping merge with conflict tracking in `src/config/keymap_merge.rs`
- [X] T022 [US2] Update loader to retain valid key mappings when other sections fail validation in `src/config/loader.rs`
- [X] T023 [US2] Integrate resolved key mappings into runtime bindings in `src/keybindings.rs` and `src/editor_state.rs`
- [X] T024 [US2] Emit duplicate-keymap and skipped-section warnings in `src/config/warnings.rs` and `src/config/loader.rs`

**Checkpoint**: US1 and US2 both work independently.

---

## Phase 5: User Story 3 - Tolerate unknown settings (Priority: P3)

**Goal**: Ignore unknown settings while continuing startup and applying known settings.

**Independent Test**: Add unknown keys to an otherwise valid config and verify known settings are still loaded and unknown settings are warned.

### Tests for User Story 3

- [X] T025 [P] [US3] Add integration test for mixed known/unknown key tolerance in `tests/config_loading_test.rs`
- [X] T026 [P] [US3] Add integration test for missing include recovery in `tests/config_include_missing_test.rs`

### Implementation for User Story 3

- [X] T027 [P] [US3] Implement unknown section/key detection and non-fatal issue tracking in `src/config/validator.rs`
- [X] T028 [US3] Implement missing include handling (skip + default + warn) in `src/config/include_loader.rs` and `src/config/loader.rs`
- [X] T029 [US3] Implement validation result output matching contract issue schema in `src/config/loader.rs`
- [X] T030 [US3] Ensure startup continues for recoverable parse/validation failures in `src/config/loader.rs` and `src/main.rs`
- [X] T031 [US3] Emit unknown-key and missing-include warnings to startup stderr in `src/config/warnings.rs`

**Checkpoint**: All user stories are independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Finalize docs, coverage, and validation across stories.

- [X] T032 [P] Add user configuration guide (syntax, `#` comments, includes, warnings) in `docs/src/configuration.md`
- [X] T033 [P] Update docs navigation and troubleshooting for config failures in `docs/src/SUMMARY.md` and `docs/src/troubleshooting.md`
- [X] T034 Add success-criteria regression assertions for SC-001/SC-002/SC-003 in `tests/config_loading_test.rs` and `tests/config_keymap_resilience_test.rs`
- [X] T035 Validate and align feature quickstart scenarios with final behavior in `specs/004-add-config-files/quickstart.md`
- [X] T036 Run repository validation commands and capture any required fixes in `src/config/*.rs` and `tests/*.rs`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies
- **Phase 2 (Foundational)**: Depends on Phase 1; blocks all user stories
- **Phase 3 (US1)**: Depends on Phase 2; MVP start
- **Phase 4 (US2)**: Depends on Phase 2; can run after US1 or in parallel once foundation is ready
- **Phase 5 (US3)**: Depends on Phase 2; can run after US1 or in parallel once foundation is ready
- **Phase 6 (Polish)**: Depends on completion of desired user stories

### User Story Dependencies

- **US1 (P1)**: Independent after foundational phase
- **US2 (P2)**: Independent after foundational phase; reuses loader/parser primitives from Phase 2
- **US3 (P3)**: Independent after foundational phase; reuses loader/parser primitives from Phase 2

### Within-Story Order

- Write story tests first and ensure they fail
- Implement parser/validator/model logic before loader integration
- Complete story checkpoints before moving priority if working sequentially

---

## Parallel Execution Examples

### User Story 1

```bash
Task: T011 [US1] parser unit tests in src/config/parser.rs
Task: T012 [US1] integration load test in tests/config_loading_test.rs
```

### User Story 2

```bash
Task: T018 [US2] resilience integration test in tests/config_keymap_resilience_test.rs
Task: T019 [US2] keymap conflict unit test in src/config/keymap_merge.rs
```

### User Story 3

```bash
Task: T025 [US3] mixed known/unknown integration test in tests/config_loading_test.rs
Task: T026 [US3] missing include integration test in tests/config_include_missing_test.rs
```

---

## Implementation Strategy

### MVP First (US1)

1. Complete Setup + Foundational phases
2. Deliver US1 end-to-end (valid config loading)
3. Validate US1 independently before expanding scope

### Incremental Delivery

1. Add US2 for keymapping resilience under partial failure
2. Add US3 for unknown-key and missing-include tolerance
3. Finish with polish/documentation and final validation

### Team Parallelization

1. Team completes Phase 1-2 together
2. Then one engineer can take US1 while others start US2/US3 tests in parallel
3. Merge story branches after each independent checkpoint passes
