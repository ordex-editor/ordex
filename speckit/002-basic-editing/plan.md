# Implementation Plan: Basic Editing Features

**Branch**: `002-basic-editing` | **Date**: 2025-02-04 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/002-basic-editing/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Extend the MVP viewer into a functional text editor by adding vim-style navigation (hjkl, word motions, page scrolling), insert mode for editing text, file saving capability, basic search, go-to-line command, and status bar displaying current mode. The feature requires selecting an efficient text data structure (rope, piece table, or gap buffer) that supports fast insertion/deletion while maintaining the 5-dependency constitutional limit.

## Technical Context

**Language/Version**: Rust (stable), edition 2024
**Primary Dependencies**: termion 4.0.6 (terminal handling), ropey 2.0.0-beta.1 (text rope)
**Storage**: File I/O (read/write text files to disk)
**Testing**: cargo test (unit tests inline, integration tests in tests/)
**Target Platform**: Linux (POSIX terminals with ANSI support)
**Project Type**: Single CLI application (text editor)
**Performance Goals**: 
  - 60fps (16ms) response to all keyboard input
  - Support files > 1 GB without performance degradation
  - Searches complete within 2 seconds for 100k line files
  - Save operations for 500 MB files complete within 5 seconds
**Constraints**: 
  - Max 5 total runtime dependencies (constitution)
  - No operations may freeze the editor
  - Must support future vim-like features and LSP integration
  - Text data structure must be abstracted for future swapping
**Scale/Scope**: 
  - Single file editor (no buffer management yet)
  - Files up to 1+ GB
  - Lines up to 100k+
  - ~2000-3000 LOC estimated for this phase

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Initial Check (Pre-Research)

| Constraint | Status | Notes |
|------------|--------|-------|
| ≤5 transitive runtime deps | ⚠️ PROVISIONAL | Current: 3 (termion, libc, numtoa). Budget remaining: 2. |
| No proc-macros | ⚠️ PROVISIONAL | Pending text data structure crate selection |
| No heavy build scripts | ⚠️ PROVISIONAL | Pending text data structure crate selection |
| Minimal dependency trees | ⚠️ PROVISIONAL | Must verify chosen text data structure has minimal transitive deps |

**Initial Status**: ⚠️ CONDITIONAL PASS - Proceeded to Phase 0 research.

### Post-Research Check

**Decision**: Selected `ropey` v2.0.0-beta.1 for text data structure (see research.md)

| Constraint | Status | Notes |
|------------|--------|-------|
| ≤5 transitive runtime deps | ✅ PASS | Adding ropey: termion + libc + numtoa + ropey + str_indices = **5 total** (at limit) |
| No proc-macros | ✅ PASS | Ropey has no proc-macro dependencies |
| No heavy build scripts | ✅ PASS | Ropey is pure Rust + str_indices (pure Rust) |
| Minimal dependency trees | ✅ PASS | Ropey adds only 1 transitive dep (str_indices) |
| Feature branch workflow | ✅ PASS | Branch: `002-basic-editing` |
| Rust project at root | ✅ PASS | Cargo.toml at repo root |
| Agent files in subdirectory | ✅ PASS | `speckit/002-basic-editing/` |
| Code style (comments for "why") | ✅ PASS | Will follow established pattern from Phase 001 |
| cargo fmt before commit | ✅ PASS | Will follow established pattern |
| cargo clippy before commit | ✅ PASS | Will follow established pattern |
| Unit tests in modules | ✅ PASS | Will follow established pattern from Phase 001 |

**GATE STATUS**: ✅ **PASS** - Dependency limit reached (5/5). No additional runtime dependencies can be added.

## Project Structure

### Documentation (this feature)

```text
speckit/002-basic-editing/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
src/
├── main.rs              # Entry point (from Phase 001)
├── tui.rs               # Terminal handling (from Phase 001) - extend with cursor positioning
├── command.rs           # Command mode (from Phase 001) - extend with :w, :{number}
├── viewer.rs            # Content rendering (from Phase 001) - extend for insert mode
├── text_buffer.rs       # NEW: Text data structure abstraction (rope/piece table/gap buffer)
├── cursor.rs            # NEW: Cursor management and navigation logic
├── mode.rs              # NEW: Editor mode enum and transitions (Normal/Insert/Command/Search)
├── navigation.rs        # NEW: Navigation commands (hjkl, w/b, Ctrl+F/B)
├── search.rs            # NEW: Search functionality
├── status_bar.rs        # NEW: Status bar rendering
└── keybindings.rs       # NEW: In-memory key-to-action mapping

tests/
├── integration/
│   ├── cli_test.rs      # From Phase 001
│   ├── navigation_test.rs   # NEW: Test navigation commands
│   ├── editing_test.rs      # NEW: Test insert mode operations
│   ├── save_test.rs         # NEW: Test file saving
│   └── search_test.rs       # NEW: Test search functionality
└── unit/
    └── (inline #[cfg(test)] modules in each source file)
```

**Structure Decision**: Extends Phase 001's single-project structure. Continues pattern of isolating terminal library code in `tui.rs`. New modules follow single-responsibility principle: `text_buffer.rs` abstracts the data structure (enabling future swapping), `cursor.rs` handles position tracking, `mode.rs` manages state transitions, and specialized modules for navigation, search, and status bar keep concerns separated.

## Complexity Tracking

> ✅ No constitution violations - research completed successfully.

**Research Outcome**: Selected ropey v2.0.0-beta.1, which adds exactly 1 transitive dependency (str_indices), bringing total to 5/5 (at constitutional limit).

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none) | — | — |

**Note**: No additional runtime dependencies can be added without violating the constitution. Future features must work within the current dependency set.
