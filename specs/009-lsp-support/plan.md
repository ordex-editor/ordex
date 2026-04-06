# Implementation Plan: Rust Code Navigation MVP

**Branch**: `009-lsp-support` | **Date**: 2026-04-06 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/009-lsp-support/spec.md`

## Summary

Add Rust-only go-to-definition to Ordex by introducing a project-scoped rust-analyzer integration that runs outside the input path, reuses one language-server process per Rust workspace root, and feeds results back through the existing app-loop polling model so navigation stays responsive while multi-project sessions remain correct.

## Technical Context

**Language/Version**: Rust stable (edition 2024)
**Primary Dependencies**: Existing `termion`, `ropey`, `libc`; planned zero-transitive `json` crate for LSP payload parsing/encoding
**Storage**: In-memory editor and LSP session state; filesystem-backed project discovery; child-process stdio transport to rust-analyzer
**Testing**: `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, targeted unit tests in affected modules, and manual acceptance checks from `quickstart.md`
**Target Platform**: Linux/POSIX terminal environments already supported by Ordex
**Project Type**: Single Rust TUI application at repository root
**Performance Goals**: Keep normal cursor movement and mode changes responsive during 100% of definition lookups; meet spec targets of successful navigation within 5 seconds in at least 90% of supported cases
**Constraints**: No async runtime, no UI stalls on lookup or server startup, at most one new dependency with no transitive crates, Rust-only MVP, one rust-analyzer session per workspace root, docs updated in the same implementation change
**Scale/Scope**: One editor session may hold Rust files from at least three distinct projects; MVP supports go-to-definition only for Rust files inside recognized Cargo or `rust-project.json` workspaces

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Pre-Research Gate Review

- **Minimal Dependencies**: Pass. The design keeps transport logic in `std` and limits new runtime dependencies to a single zero-transitive crate (`json`), avoiding heavy LSP or async stacks.
- **Code Style**: Pass. Planned changes fit the existing module-oriented Rust style and do not require new unsafe code.
- **Testing**: Pass. The plan adds unit coverage in touched Rust modules and keeps behavior-specific regression tests close to the implementation.
- **Project Structure**: Pass. Runtime code stays under `src/`, and planning artifacts stay under `specs/009-lsp-support/`.
- **Documentation Maintenance**: Pass. User-facing docs updates are included in the implementation scope.
- **Git Workflow**: Pass. Work is already on feature branch `009-lsp-support`.

### Post-Design Gate Review

- **Minimal Dependencies**: Pass. Phase 0 research confirmed `json` is the smallest practical option and avoids new transitive crates.
- **Code Style**: Pass. The design keeps process ownership in `app.rs`, editor UI state in `editor_state`, and protocol/process details in a dedicated `src/lsp/` module.
- **Testing**: Pass. The design includes deterministic unit tests for workspace resolution, response parsing, stale-result rejection, and navigation selection behavior.
- **Project Structure**: Pass. New runtime modules are limited to `src/lsp/`, plus small touchpoints in existing editor, keybinding, render, and docs files.
- **Documentation Maintenance**: Pass. `docs/src/commands.md` and `docs/src/faq.md` are the planned user-facing documentation touchpoints.
- **Git Workflow**: Pass. No branch or repository-structure changes are required beyond the current feature branch.

## Project Structure

### Documentation (this feature)

```text
specs/009-lsp-support/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── lsp-navigation.openapi.yaml
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── app.rs
├── render.rs
├── keybindings/
│   └── defaults.rs
├── editor_state/
│   ├── actions.rs
│   ├── commands.rs
│   ├── mod.rs
│   └── view.rs
└── lsp/
    ├── mod.rs
    ├── manager.rs
    ├── project.rs
    ├── protocol.rs
    └── session.rs

docs/src/
├── commands.md
└── faq.md
```

**Structure Decision**: Keep the feature inside the existing single-project Rust layout. `app.rs` owns long-lived child-process management, `editor_state/` owns request and UI state, `keybindings/` adds the user trigger, and a new `src/lsp/` module contains workspace discovery, protocol framing, and rust-analyzer session management.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| None | N/A | N/A |
