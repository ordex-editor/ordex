# Tasks: Swap File Safety

**Input**: Design documents from `/specs/008-swap-file-safety/`
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/swap-module.md`, `quickstart.md`

**Tests**: Unit-test and end-to-end test tasks are included because they were explicitly requested.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no blocking dependency on an incomplete task)
- **[Story]**: Which user story this task belongs to (`[US1]`, `[US2]`, `[US3]`)
- Every task includes exact file paths

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the file/module scaffolding and shared test harness needed by all stories.

- [ ] T001 Add swap module entry points in `src/main.rs` and create `src/swap/mod.rs`, `src/swap/format.rs`, `src/swap/location.rs`, and `src/swap/glob.rs`
- [ ] T002 Create shared swap test support helpers in `tests/swap_test_support.rs` and wire them into `tests/config_test_support.rs`
- [ ] T003 [P] Create dedicated end-to-end swap test files in `tests/swap_recovery_test.rs` and `tests/swap_exclusion_test.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Implement the shared swap infrastructure that all user stories rely on.

**⚠️ CRITICAL**: No user story work should begin until this phase is complete.

- [ ] T004 [P] Implement `ordex-swap-v1` header serialization and parsing with unit tests in `src/swap/format.rs`
- [ ] T005 [P] Implement XDG swap directory resolution and path encoding with unit tests in `src/swap/location.rs`
- [ ] T006 [P] Implement full-path `*` glob matching with unit tests in `src/swap/glob.rs`
- [ ] T007 Implement atomic swap create/refresh/delete flows with unit tests in `src/swap/mod.rs`
- [ ] T008 Implement `BufferState` swap handle storage and shared swap lifecycle hooks in `src/editor_state/buffers.rs` and `src/editor_state/mod.rs`

**Checkpoint**: Swap infrastructure is ready; user story work can begin.

---

## Phase 3: User Story 1 - Recover unsaved text edits safely (Priority: P1) 🎯 MVP

**Goal**: Keep recovery data for normal text files and let users restore or discard stale swap content after an interrupted session.

**Independent Test**: Open a normal text file, make unsaved edits, interrupt the session, reopen the same file in Ordex, and verify the user can detect and restore or discard the interrupted work.

### Tests for User Story 1

- [ ] T009 [P] [US1] Add unit tests for recovery-state transitions and restore/discard decisions in `src/editor_state/buffers.rs` and `src/editor_state/mod.rs`
- [ ] T010 [P] [US1] Add end-to-end crash-recovery coverage in `tests/swap_recovery_test.rs`

### Implementation for User Story 1

- [ ] T011 [US1] Create and refresh swap files for normal text buffers on open and edit in `src/editor_state/mod.rs` and `src/editor_state/buffers.rs`
- [ ] T012 [US1] Implement stale-swap detection plus restore/discard prompt handling in `src/editor_state/mod.rs` and `src/app.rs`

**Checkpoint**: User Story 1 is complete when interrupted edits can be recovered independently of the other stories.

---

## Phase 4: User Story 2 - Keep swap files until the save is durable (Priority: P1)

**Goal**: Preserve the recovery copy until the real file save is durably completed and keep the swap file when save durability is not confirmed.

**Independent Test**: Edit a normal text file, save it, and verify the swap file survives until durable completion; simulate save failure or interruption and verify the swap file remains available for recovery.

### Tests for User Story 2

- [ ] T013 [P] [US2] Add unit tests for deferred-write durability sequencing and swap cleanup in `src/app.rs`
- [ ] T014 [P] [US2] Add end-to-end successful-save and failed-save coverage in `tests/save_test.rs`

### Implementation for User Story 2

- [ ] T015 [US2] Replace direct save writes with temp-file `sync_all` and atomic rename logic in `src/app.rs`
- [ ] T016 [US2] Remove swap files only after durable save confirmation and preserve them on save failure in `src/app.rs` and `src/editor_state/buffers.rs`

**Checkpoint**: User Story 2 is complete when durable saves clean up swaps and failed saves always leave recovery data behind.

---

## Phase 5: User Story 3 - Exclude selected path patterns from swap files (Priority: P2)

**Goal**: Let users suppress swap-file creation for configured absolute-path glob matches such as `/dev/shm/gopass*` and `*.gpg`.

**Independent Test**: Configure one or more exclusion globs, edit matching and non-matching files, and verify swap files are skipped only for paths that match the configured patterns.

### Tests for User Story 3

- [ ] T017 [P] [US3] Add unit tests for `[swap] exclude` parsing and matching in `src/config/validator.rs` and `src/swap/glob.rs`
- [ ] T018 [P] [US3] Add end-to-end exclusion-pattern coverage for matched and unmatched paths in `tests/swap_exclusion_test.rs`

### Implementation for User Story 3

- [ ] T019 [US3] Parse and validate `[swap] exclude` glob patterns in `src/config/loader.rs`, `src/config/validator.rs`, and `src/config/warnings.rs`
- [ ] T020 [US3] Skip swap creation when file paths match exclusion patterns in `src/editor_state/mod.rs` and `src/editor_state/buffers.rs`

**Checkpoint**: User Story 3 is complete when configured path patterns suppress swap creation without affecting normal text files that do not match.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Finish documentation, reconcile design artifacts, and run the full repository validation flow.

- [ ] T021 [P] Update swap configuration and recovery documentation in `docs/src/configuration.md` and `docs/src/file-operations.md`
- [ ] T022 [P] Reconcile final behavior notes in `specs/008-swap-file-safety/quickstart.md` and `specs/008-swap-file-safety/contracts/swap-module.md`
- [ ] T023 Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --quiet` from `Cargo.toml`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1: Setup** — no dependencies
- **Phase 2: Foundational** — depends on Phase 1 and blocks all user stories
- **Phase 3: US1** — depends on Phase 2
- **Phase 4: US2** — depends on Phase 2
- **Phase 5: US3** — depends on Phase 2
- **Phase 6: Polish** — depends on all selected user stories being complete

### User Story Dependencies

- **US1 (P1)**: starts after Foundational and delivers the MVP recovery flow
- **US2 (P1)**: starts after Foundational and hardens save behavior without depending on US3
- **US3 (P2)**: starts after Foundational and adds configuration-based exclusion without depending on US2

### Within Each User Story

- Unit tests and end-to-end tests should be written before implementation tasks in that story
- Swap/data helpers before editor-state integration
- Core editor behavior before docs and final validation

### Suggested Story Completion Order

1. **US1** for MVP recovery value
2. **US2** for durable-save safety
3. **US3** for configurable exclusions

---

## Parallel Opportunities

- **Setup**: `T003` can run in parallel once `T001` defines the swap feature scope
- **Foundational**: `T004`, `T005`, and `T006` can run in parallel because they touch separate swap submodules
- **US1**: `T009` and `T010` can run in parallel
- **US2**: `T013` and `T014` can run in parallel
- **US3**: `T017` and `T018` can run in parallel
- **Polish**: `T021` and `T022` can run in parallel

---

## Parallel Example: User Story 1

```bash
Task: "T009 [US1] Add unit tests for recovery-state transitions in src/editor_state/buffers.rs and src/editor_state/mod.rs"
Task: "T010 [US1] Add end-to-end crash-recovery coverage in tests/swap_recovery_test.rs"
```

## Parallel Example: User Story 2

```bash
Task: "T013 [US2] Add unit tests for deferred-write durability sequencing in src/app.rs"
Task: "T014 [US2] Add end-to-end successful-save and failed-save coverage in tests/save_test.rs"
```

## Parallel Example: User Story 3

```bash
Task: "T017 [US3] Add unit tests for [swap] exclude parsing and matching in src/config/validator.rs and src/swap/glob.rs"
Task: "T018 [US3] Add end-to-end exclusion-pattern coverage in tests/swap_exclusion_test.rs"
```

---

## Implementation Strategy

### MVP First

1. Complete **Phase 1: Setup**
2. Complete **Phase 2: Foundational**
3. Complete **Phase 3: US1**
4. Validate crash recovery independently before moving on

### Incremental Delivery

1. Add shared swap infrastructure
2. Deliver **US1** recovery
3. Add **US2** durable-save protection
4. Add **US3** configurable exclusions
5. Finish docs and full validation

### Parallel Team Strategy

1. One engineer completes Setup + Foundational
2. Then work can split across stories:
   - Engineer A: **US1**
   - Engineer B: **US2**
   - Engineer C: **US3**

---

## Notes

- All tasks follow the required checklist format: checkbox, task ID, optional `[P]`, required story label for story tasks, and exact file paths
- Unit-test tasks are included in source files with `tests` sub-modules to match project conventions
- End-to-end tasks are included in `tests/` integration test files
- The current feature does **not** implement duplicate-open warnings; that remains future work
