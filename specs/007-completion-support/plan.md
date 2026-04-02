# Implementation Plan: Completion Support

**Branch**: `007-completion-support` | **Date**: 2026-04-02 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/007-completion-support/spec.md`

## Summary

Add MVP buffer-text completion that appears automatically while typing in Insert mode, suggests only current-buffer words of at least 3 characters, matches case-insensitively, previews the selected candidate directly in the buffer, and cancels by navigating to a state with no selected item. **Architectural impact is moderate** because the feature touches core insert-mode input, editor state, and rendering, but the recommended design is to **create a new `src/completion/` module and extend existing code only at integration seams** instead of refactoring the picker stack or embedding completion logic directly into the existing file-picker/dialog code.

## Technical Context

**Language/Version**: Rust stable, edition 2024  
**Primary Dependencies**: `termion 4.0.6`, `ropey 2.0.0-beta.1` with `metric_chars`, `libc 0.2.180`; no new dependencies planned  
**Storage**: In-memory Rope-backed text buffers plus filesystem-backed documents; no external database or service  
**Testing**: `cargo test --quiet`, focused integration tests in `tests/`, module unit tests in `src/`, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`  
**Target Platform**: POSIX-compatible terminal editor on Linux/Unix-like systems  
**Project Type**: Single Rust TUI application at the repository root  
**Performance Goals**: Buffer-text completion must stay within the normal interactive edit path without visible typing stalls; future async sources must fit the existing background polling model and drop stale work safely  
**Constraints**: Minimal-dependency constitution, Insert mode must remain active while suggestions are visible, selection changes must preview directly in the buffer, cancellation must work by navigating to no selected item, candidates must be at least 3 characters, matching is case-insensitive with original-case insertion, and the MVP is limited to the active buffer  
**Scale/Scope**: One active completion session per editor, potentially large buffers, and future support for file-path, LSP, and plugin sources without changing the completion user flow

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Pre-Research Gate

- **I. Minimal Dependencies** — **PASS**: the plan reuses the standard library plus existing dependencies; no new crates are required for the MVP design.
- **II. Code Style** — **PASS**: the design keeps completion logic in a focused new module and limits changes in existing files to integration seams.
- **III. Testing** — **PASS**: the plan includes unit coverage for candidate extraction/session rules plus integration coverage for insert-mode behavior and stale-session dismissal.
- **IV. Project Structure** — **PASS**: generated planning artifacts stay under `specs/007-completion-support/`, and code changes remain under the root Rust project.
- **V. Documentation Maintenance** — **PASS with follow-through**: implementation must update the user-facing docs site in the same change.
- **VI. Git Workflow** — **PASS**: work is already on feature branch `007-completion-support`.

### Post-Design Re-check

- **I. Minimal Dependencies** — **PASS**: Phase 1 artifacts still assume no new runtime or dev dependencies.
- **II. Code Style** — **PASS**: the design keeps completion state/source logic outside the modal picker implementation to avoid mixed responsibilities.
- **III. Testing** — **PASS**: the design supports isolated unit tests and end-to-end editor tests without bespoke infrastructure.
- **IV. Project Structure** — **PASS**: the chosen structure adds a focused `src/completion/` namespace and leaves existing module boundaries intact.
- **V. Documentation Maintenance** — **PASS with follow-through**: quickstart and implementation notes align with the requirement to update the docs site later.
- **VI. Git Workflow** — **PASS**: no branch or workflow changes are needed.

## Project Structure

### Documentation (this feature)

```text
specs/007-completion-support/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── completion.openapi.yaml
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── app.rs
├── completion/             # new completion-specific source/session logic
│   ├── mod.rs
│   └── buffer_source.rs
├── dialogs/
│   ├── buffer_switch.rs
│   ├── file_picker.rs
│   └── picker.rs
├── editor_state/
│   ├── actions.rs
│   ├── buffers.rs
│   ├── commands.rs
│   ├── history.rs
│   ├── matching.rs
│   ├── mod.rs
│   └── view.rs
├── keybindings/
│   ├── defaults.rs
│   ├── parse.rs
│   └── registry.rs
├── mode.rs
├── render.rs
└── text_buffer.rs

tests/
├── editing_test.rs
├── file_picker_test.rs
├── sequence_discovery_popup_test.rs
├── status_bar_test.rs
└── completion_test.rs      # new end-to-end completion coverage

crates/
└── test_utils/             # existing shared test helpers
```

**Structure Decision**: **Create a new module**. Completion should live in `src/completion/` because it is an inline Insert-mode capability with future source extensibility, not a modal query dialog. Existing code should be extended only where it owns integration concerns: `editor_state` for lifecycle/state transitions and preview restoration, `render.rs` for popup drawing, `keybindings` for selection/navigation semantics including the no-selection cancel state, and `app.rs` only if future async completion sources need to join the current background polling path. A refactor-first approach is rejected because it would entangle the proven file-picker/dialog system with different completion semantics before the MVP proves the need.

## Complexity Tracking

No constitution violations identified.
