# Tasks: Completion Support

**Input**: Design documents from `/specs/007-completion-support/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/, quickstart.md

**Tests**: Unit and end-to-end test tasks are included for each user story phase.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- Single Rust TUI project rooted at `src/` and `tests/`
- User-facing documentation lives under `docs/src/`
- Feature planning artifacts live under `specs/007-completion-support/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the completion module entry points and wire them into the crate layout.

- [ ] T001 Create the completion module entry and file scaffolding in `src/main.rs`, `src/completion/mod.rs`, and `src/completion/buffer_source.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core completion infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T002 Define shared completion request, candidate, and session types with no-selection preview restoration semantics in `src/completion/mod.rs`
- [ ] T003 [P] Add `EditorState`-owned completion fields plus reset and invalidation hooks in `src/editor_state/mod.rs`
- [ ] T004 [P] Add completion-specific insert-mode actions and default Up/Down navigation bindings in `src/keybindings.rs`, `src/keybindings/defaults.rs`, and `src/editor_state/actions.rs`
- [ ] T005 [P] Add completion popup snapshot plumbing and overlay presentation hooks in `src/editor_state/view.rs` and `src/render.rs`

**Checkpoint**: Foundation ready - user story implementation can now begin

---

## Phase 3: User Story 1 - Complete words already in the buffer (Priority: P1) 🎯 MVP

**Goal**: Deliver automatic buffer-text completion with live preview and restoration to the original prefix when no item is selected.

**Independent Test**: Open a document with repeated words, type a matching prefix in Insert mode, confirm suggestions appear automatically, confirm Up/Down changes previewed text immediately, and confirm moving to no selection restores the original prefix.

### Tests for User Story 1

- [ ] T006 [P] [US1] Add unit tests for buffer-word scanning, case-insensitive matching, and duplicate collapsing in `src/completion/buffer_source.rs`
- [ ] T007 [P] [US1] Add end-to-end completion-flow tests for live preview and prefix restoration in `tests/completion_test.rs`

### Implementation for User Story 1

- [ ] T008 [P] [US1] Implement buffer-word scanning, case-insensitive matching, and duplicate collapsing in `src/completion/buffer_source.rs`
- [ ] T009 [P] [US1] Render completion popup rows, selected preview state, and no-selection state in `src/editor_state/view.rs` and `src/render.rs`
- [ ] T010 [US1] Implement prefix detection and automatic completion-session refresh after Insert-mode edits in `src/completion/mod.rs` and `src/editor_state/actions.rs`
- [ ] T011 [US1] Implement live preview replacement and original-prefix restoration in `src/completion/mod.rs` and `src/editor_state/mod.rs`

**Checkpoint**: User Story 1 should now provide usable buffer-text completion on its own

---

## Phase 4: User Story 2 - Keep typing fluid while using completion (Priority: P1)

**Goal**: Keep completion responsive in larger buffers and ensure stale previews do not survive editing or navigation changes.

**Independent Test**: In a large document, trigger completion repeatedly while typing and navigating, and confirm the editor stays responsive while invalid or stale previews disappear or restore correctly.

### Tests for User Story 2

- [ ] T012 [P] [US2] Add unit tests for refresh bounding and stale-session invalidation helpers in `src/completion/mod.rs`
- [ ] T013 [P] [US2] Add end-to-end responsiveness and stale-preview tests in `tests/completion_test.rs`

### Implementation for User Story 2

- [ ] T014 [P] [US2] Bound completion refresh work and skip unnecessary rescans for unchanged prefixes in `src/completion/mod.rs` and `src/completion/buffer_source.rs`
- [ ] T015 [US2] Invalidate or restore stale completion sessions on cursor moves, buffer switches, and invalidating edits in `src/editor_state/mod.rs` and `src/editor_state/actions.rs`
- [ ] T016 [P] [US2] Minimize redraw scope for completion-popup updates in `src/editor_state/view.rs` and `src/render.rs`

**Checkpoint**: User Story 2 should keep the completion flow responsive without breaking User Story 1

---

## Phase 5: User Story 3 - Preserve room for future completion sources (Priority: P2)

**Goal**: Keep the MVP architecture source-agnostic so future file-path, LSP, and plugin providers can reuse the same completion lifecycle.

**Independent Test**: Review the implemented completion module and confirm source boundaries, request generation, and popup lifecycle can support additional providers without redefining the user interaction model.

### Tests for User Story 3

- [ ] T017 [P] [US3] Add unit tests for source registration and generation-tracking behavior in `src/completion/mod.rs`
- [ ] T018 [P] [US3] Add end-to-end extensibility seam coverage in `tests/completion_test.rs`

### Implementation for User Story 3

- [ ] T019 [P] [US3] Refine the `CompletionSource` abstraction and buffer-source registration path in `src/completion/mod.rs` and `src/completion/buffer_source.rs`
- [ ] T020 [P] [US3] Carry source metadata and source-agnostic popup fields through `src/editor_state/view.rs` and `src/render.rs`
- [ ] T021 [US3] Thread request-generation and future async-source seams through `src/completion/mod.rs` and `src/editor_state/mod.rs`

**Checkpoint**: User Story 3 should make the completion architecture future-ready without changing the MVP UX

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, validation, and final cleanup across all stories

- [ ] T022 [P] Update completion usage documentation in `docs/src/modes-and-keybindings.md` and `docs/src/SUMMARY.md`
- [ ] T023 [P] Update completion feature overview in `README.md` and `docs/src/index.md`
- [ ] T024 Run completion validation from `specs/007-completion-support/quickstart.md` and repository checks referenced in `README.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Story 1 (Phase 3)**: Depends on Foundational completion
- **User Story 2 (Phase 4)**: Depends on User Story 1 completion because it optimizes and hardens the same completion flow
- **User Story 3 (Phase 5)**: Depends on User Story 1 completion because it formalizes the MVP source boundaries around the delivered flow
- **Polish (Phase 6)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Starts after Foundational and delivers the MVP completion flow
- **User Story 2 (P1)**: Builds on User Story 1 to preserve responsiveness and freshness
- **User Story 3 (P2)**: Builds on User Story 1 to keep the architecture extensible for future sources

### Within Each User Story

- Unit and end-to-end tests before or alongside the matching story implementation
- Shared data types and editor-state hooks before story-specific integration
- Candidate generation before live preview application
- Popup rendering before full end-to-end flow integration
- Story-specific behavior complete before moving to the next dependent story

### Parallel Opportunities

- **Phase 2**: T003, T004, and T005 can proceed in parallel after T002
- **User Story 1**: T006, T007, T008, and T009 can be split into test and implementation work after Phase 2
- **User Story 2**: T012, T013, T014, and T016 can proceed in parallel after User Story 1
- **User Story 3**: T017, T018, T019, and T020 can proceed in parallel after User Story 1
- **Polish**: T022 and T023 can proceed in parallel before T024

---

## Parallel Example: User Story 1

```bash
# Launch the independent candidate-generation and popup-rendering work together:
Task: "Add unit tests for buffer-word scanning, case-insensitive matching, and duplicate collapsing in src/completion/buffer_source.rs"
Task: "Add end-to-end completion-flow tests for live preview and prefix restoration in tests/completion_test.rs"
Task: "Render completion popup rows, selected preview state, and no-selection state in src/editor_state/view.rs and src/render.rs"
```

## Parallel Example: User Story 2

```bash
# Launch responsiveness work that touches different layers together:
Task: "Add unit tests for refresh bounding and stale-session invalidation helpers in src/completion/mod.rs"
Task: "Add end-to-end responsiveness and stale-preview tests in tests/completion_test.rs"
Task: "Minimize redraw scope for completion-popup updates in src/editor_state/view.rs and src/render.rs"
```

## Parallel Example: User Story 3

```bash
# Launch future-facing architecture work on separate files together:
Task: "Add unit tests for source registration and generation-tracking behavior in src/completion/mod.rs"
Task: "Add end-to-end extensibility seam coverage in tests/completion_test.rs"
Task: "Carry source metadata and source-agnostic popup fields through src/editor_state/view.rs and src/render.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1
4. Stop and validate the live-preview completion flow from `specs/007-completion-support/quickstart.md`

### Incremental Delivery

1. Complete Setup + Foundational → completion infrastructure ready
2. Add User Story 1 → validate MVP buffer-text completion
3. Add User Story 2 → validate responsiveness and stale-session handling
4. Add User Story 3 → validate future-source extensibility seams
5. Finish Polish → update docs and run final checks

### Parallel Team Strategy

1. One developer completes Setup + Foundational
2. After Phase 2:
   - Developer A: T006/T008/T010/T011 (completion core + tests)
   - Developer B: T007/T009 (end-to-end coverage + popup rendering)
3. After User Story 1:
   - Developer A: User Story 2 responsiveness tasks
   - Developer B: User Story 3 extensibility tasks

---

## Notes

- [P] tasks touch different files and avoid incomplete-task dependencies
- User story phases are organized for incremental delivery, even where later stories depend on the MVP flow
- The suggested MVP scope is **Phase 1 + Phase 2 + User Story 1**
- User-facing docs updates are required by the project constitution
