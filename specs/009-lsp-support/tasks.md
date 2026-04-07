# Tasks: Rust Code Navigation MVP

**Input**: Design documents from `/specs/009-lsp-support/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Unit-test and end-to-end test tasks are included because the feature explicitly requested both.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently once shared foundations are complete.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel when dependencies are already complete
- **[Story]**: User story label for traceability (`[US1]`, `[US2]`, `[US3]`)
- Every task includes the exact repo-relative file path to change

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add the minimal dependency and scaffold the files and fixtures needed for LSP work.

- [X] T001 Add the zero-transitive `json` dependency in Cargo.toml
- [X] T002 [P] Create the LSP module skeleton in src/lsp/mod.rs, src/lsp/project.rs, src/lsp/protocol.rs, src/lsp/session.rs, and src/lsp/manager.rs
- [X] T003 [P] Create reusable Rust navigation fixtures in tests/fixtures/lsp/workspace_one/Cargo.toml, tests/fixtures/lsp/workspace_one/src/lib.rs, tests/fixtures/lsp/workspace_one/src/main.rs, tests/fixtures/lsp/workspace_two/Cargo.toml, tests/fixtures/lsp/workspace_two/src/lib.rs, and tests/fixtures/lsp/workspace_two/src/main.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish the shared LSP plumbing that all user stories depend on.

**⚠️ CRITICAL**: No user story work should start until this phase is complete.

- [X] T004 Add a go-to-definition action and the default `g d` binding in src/keybindings.rs and src/keybindings/defaults.rs
- [X] T005 [P] Add editor-side LSP request state, lookup tokens, and render-facing view helpers in src/editor_state/mod.rs and src/editor_state/view.rs
- [X] T006 [P] Implement workspace-root discovery and canonical project-key helpers with unit tests in src/lsp/project.rs
- [X] T007 [P] Implement JSON-RPC framing, rust-analyzer request builders, and response normalization with unit tests in src/lsp/protocol.rs
- [X] T008 Implement rust-analyzer session lifecycle, document-sync bookkeeping, and request tracking with unit tests in src/lsp/session.rs
- [X] T009 Implement the app-owned LSP manager, child-process startup, and editor-request dispatch in src/lsp/manager.rs and src/app.rs
- [X] T010 Wire LSP background polling into the existing timed app loop in src/app.rs and src/editor_state/mod.rs

**Checkpoint**: Foundation ready — user story work can begin.

---

## Phase 3: User Story 1 - Jump to a symbol definition in Rust code (Priority: P1) 🎯 MVP

**Goal**: Let a user jump from a Rust symbol usage to a single resolved definition, including targets in unopened files.

**Independent Test**: Open a supported Rust project, place the cursor on a symbol with a known definition, trigger `g d`, and verify that Ordex opens the correct file and cursor location without freezing.

### Tests for User Story 1

- [X] T011 [P] [US1] Add unit tests for cursor-position translation and single-location result parsing in src/lsp/protocol.rs
- [X] T012 [P] [US1] Add an end-to-end success-path test for same-file and unopened-file definitions in tests/lsp_goto_definition_test.rs

### Implementation for User Story 1

- [X] T013 [US1] Implement definition-request creation and per-buffer version sync in src/lsp/session.rs
- [X] T014 [US1] Implement the Normal-mode go-to-definition action flow in src/editor_state/actions.rs and src/editor_state/mod.rs
- [X] T015 [US1] Apply single-target definition results by opening buffers and moving the cursor in src/app.rs and src/editor_state/mod.rs

**Checkpoint**: User Story 1 delivers the MVP and can be validated independently.

---

## Phase 4: User Story 2 - Get clear feedback when navigation cannot complete (Priority: P2)

**Goal**: Preserve the current editing context and show clear outcomes for unsupported files, unavailable navigation, missing definitions, stale responses, and multiple targets.

**Independent Test**: Trigger `g d` from unsupported files, unresolved symbols, and temporarily unavailable Rust contexts, then confirm that Ordex stays in place and reports the right message; trigger a multi-target result and confirm that the user can choose the destination.

### Tests for User Story 2

- [X] T016 [P] [US2] Add unit tests for stale-result rejection and feedback-state transitions in src/editor_state/mod.rs
- [ ] T017 [P] [US2] Add end-to-end failure-path and multi-target chooser tests in tests/lsp_feedback_test.rs

### Implementation for User Story 2

- [X] T018 [US2] Implement user-visible feedback for unsupported-file, unsupported-project, server-starting, and not-found outcomes in src/lsp/manager.rs and src/editor_state/view.rs
- [X] T019 [US2] Implement stale-result rejection with lookup-token and buffer-version checks in src/editor_state/mod.rs and src/lsp/session.rs
- [X] T020 [US2] Implement multiple-target selection using the existing picker UI in src/dialogs/picker.rs, src/editor_state/actions.rs, and src/editor_state/view.rs

**Checkpoint**: User Story 2 keeps failed lookups understandable and safe without breaking the MVP flow.

---

## Phase 5: User Story 3 - Navigate correctly across multiple Rust projects in one session (Priority: P2)

**Goal**: Reuse one rust-analyzer session per workspace root while keeping lookups correct for files from different Rust projects opened in the same Ordex session.

**Independent Test**: Open Rust files from multiple Cargo workspaces in one Ordex session, trigger `g d` in each, and verify that each lookup resolves inside the active file's workspace instead of crossing projects.

### Tests for User Story 3

- [X] T021 [P] [US3] Add unit tests for canonical workspace matching and session reuse rules in src/lsp/project.rs and src/lsp/manager.rs
- [X] T022 [P] [US3] Add an end-to-end multi-project reuse test in tests/lsp_multi_project_test.rs

### Implementation for User Story 3

- [X] T023 [US3] Implement a workspace-keyed session registry with lazy rust-analyzer startup in src/lsp/manager.rs
- [X] T024 [US3] Route buffer open, switch, and close flows through workspace-aware LSP session management in src/editor_state/mod.rs, src/app.rs, and src/lsp/session.rs
- [X] T025 [US3] Implement unsupported-project handling for Rust files outside recognized workspaces in src/lsp/project.rs and src/lsp/manager.rs

**Checkpoint**: User Story 3 makes multi-project Rust sessions correct and resource-efficient.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final validation, documentation, and cross-story regressions.

- [X] T026 [P] Update user-facing navigation documentation in docs/src/commands.md and docs/src/faq.md
- [X] T027 [P] Add regression coverage for the `g d` binding and responsive status rendering in tests/command_input_bindings_test.rs and tests/status_bar_test.rs
- [X] T028 Run the smoke, multi-project, and failure-path scenarios documented in specs/009-lsp-support/quickstart.md
- [X] T029 Run repository validation commands from Cargo.toml: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1: Setup** — No dependencies
- **Phase 2: Foundational** — Depends on Phase 1 and blocks all user stories
- **Phase 3: US1** — Depends on Phase 2
- **Phase 4: US2** — Depends on Phase 2 and the core request/result path from US1
- **Phase 5: US3** — Depends on Phase 2 and the core request/result path from US1
- **Phase 6: Polish** — Depends on all desired user stories being complete

### User Story Dependency Graph

```text
Setup -> Foundational -> US1 -> { US2, US3 } -> Polish
```

### Within Each User Story

- Unit tests and end-to-end tests are written before story implementation tasks
- Protocol/data helpers come before editor integration
- Editor integration comes before full validation of the story

### Parallel Opportunities

- `T002` and `T003` can run in parallel after `T001`
- `T005`, `T006`, and `T007` can run in parallel once `T004` is complete
- `T011` and `T012` can run in parallel for US1
- `T016` and `T017` can run in parallel for US2
- `T021` and `T022` can run in parallel for US3
- `T026` and `T027` can run in parallel during polish

---

## Parallel Example: User Story 1

```text
T011 [US1] Add unit tests for cursor-position translation and single-location result parsing in src/lsp/protocol.rs
T012 [US1] Add an end-to-end success-path test for same-file and unopened-file definitions in tests/lsp_goto_definition_test.rs
```

## Parallel Example: User Story 2

```text
T016 [US2] Add unit tests for stale-result rejection and feedback-state transitions in src/editor_state/mod.rs
T017 [US2] Add end-to-end failure-path and multi-target chooser tests in tests/lsp_feedback_test.rs
```

## Parallel Example: User Story 3

```text
T021 [US3] Add unit tests for canonical workspace matching and session reuse rules in src/lsp/project.rs and src/lsp/manager.rs
T022 [US3] Add an end-to-end multi-project reuse test in tests/lsp_multi_project_test.rs
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1
4. Validate the smoke test in `specs/009-lsp-support/quickstart.md`
5. Stop for review before starting failure feedback or multi-project reuse

### Incremental Delivery

1. Setup + Foundational create the shared LSP plumbing
2. US1 delivers the first usable Rust go-to-definition flow
3. US2 adds safe failure handling and multi-target disambiguation
4. US3 adds multi-project correctness and session reuse
5. Polish closes documentation and regression gaps

### Suggested MVP Scope

- **MVP**: Phase 1 + Phase 2 + Phase 3 (User Story 1 only)

---

## Notes

- All tasks use the required checklist format with IDs, optional `[P]` markers, required story labels for story-specific work, and exact file paths
- Unit-test tasks target module-local Rust tests, and end-to-end tasks target top-level integration tests under `tests/`
- `contracts/lsp-navigation.openapi.yaml` maps to the async request/result tasks in US1 and the feedback/session-state tasks in US2 and US3
