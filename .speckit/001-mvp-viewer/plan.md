# Implementation Plan: MVP Viewer

**Branch**: `001-mvp-viewer` | **Date**: 2026-01-31 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `.speckit/001-mvp-viewer/spec.md`

## Summary

Implement a minimal TUI text editor that can open and display a file passed via CLI argument, with a vim-style `:q` command to quit. Uses termion for terminal handling in raw mode.

## Technical Context

**Language/Version**: Rust (stable)  
**Primary Dependencies**: termion 4.0.6 (2 transitive deps: libc, numtoa)  
**Storage**: N/A (read-only file display)  
**Testing**: cargo test (unit + integration)  
**Target Platform**: Linux (POSIX terminals)  
**Project Type**: single CLI application  
**Performance Goals**: General responsiveness (no specific metrics for MVP)  
**Constraints**: ≤5 transitive runtime dependencies per constitution  
**Scale/Scope**: Single-file viewer, ~500-1000 LOC estimated

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Constraint | Status | Notes |
|------------|--------|-------|
| ≤5 transitive deps | ✅ PASS | termion brings 2 (libc, numtoa), total: 3 |
| No proc-macros | ✅ PASS | termion has no proc-macro deps |
| No heavy build scripts | ✅ PASS | termion is pure Rust + libc |
| Feature branch workflow | ✅ | Branch: `001-mvp-viewer` |
| Rust project at root | ✅ | `Cargo.toml` at repo root |
| Agent files in subdirectory | ✅ | `.speckit/001-mvp-viewer/` |

## Project Structure

### Documentation (this feature)

```text
.speckit/001-mvp-viewer/
├── spec.md              # Feature specification
├── plan.md              # This file
└── tasks.md             # Phase 2 output (/speckit.tasks command)
```

### Source Code (repository root)

```text
src/
├── main.rs              # Entry point, CLI arg parsing, orchestration
├── tui.rs               # All termion/terminal code isolated here
├── viewer.rs            # File content rendering logic
└── command.rs           # Command mode handling (:q parsing)

tests/
└── integration/
    └── cli_test.rs      # End-to-end CLI tests (binary invocation)
```

**Structure Decision**: Single project with flat `src/` modules. Unit tests are inline (`#[cfg(test)]` modules within each source file). Integration tests live in `tests/` for binary-level testing. All terminal library code (termion) is isolated in `tui.rs` — if we ever need to switch libraries, only this one file needs to change.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                        main.rs                          │
│  • Parse CLI args (file path)                          │
│  • Load file content                                    │
│  • Initialize terminal via tui module                  │
│  • Run event loop                                       │
│  • Restore terminal on exit                            │
└─────────────────────────────────────────────────────────┘
                            │
          ┌─────────────────┼─────────────────┐
          ▼                 ▼                 ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
│    tui.rs       │ │   viewer.rs     │ │   command.rs    │
│ (termion code)  │ │                 │ │                 │
│ • enter_raw()   │ │ • render()      │ │ • CommandMode   │
│ • exit_raw()    │ │ • get_visible() │ │ • parse_cmd()   │
│ • clear_screen()│ │                 │ │ • execute()     │
│ • read_key()    │ │                 │ │                 │
│ • write_at()    │ │                 │ │                 │
└─────────────────┘ └─────────────────┘ └─────────────────┘
```

## Key Design Decisions

1. **Isolated TUI module**: All termion-specific code lives in `tui.rs`. If the terminal library needs to change in the future, only this single file requires modification.

2. **Terminal restoration**: Use RAII pattern (Drop trait) to ensure terminal is restored even on panic

3. **Error handling**: Return `Result` from main, use `eprintln!` for user-facing errors

4. **Event loop**: Single-threaded blocking read on stdin for keyboard input

5. **Line storage**: Simple `Vec<String>` for file content (sufficient for MVP)

## Complexity Tracking

> No constitution violations identified.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none) | — | — |
