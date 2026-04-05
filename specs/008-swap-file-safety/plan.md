# Implementation Plan: Swap File Safety

**Branch**: `008-swap-file-safety` | **Date**: 2026-04-05 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/008-swap-file-safety/spec.md`

## Summary

Ordex will create and maintain swap files for normal text files that contain unsaved edits,
keeping each swap file on disk until the corresponding real-file save is durably confirmed via
`sync_all()` and an atomic rename. A new `[swap] exclude` config key accepts user-defined glob
patterns matched against the full file path to opt specific files out of swap protection. The
swap file format is a custom line-oriented text format (`ordex-swap-v1`) that stores process
metadata and raw buffer content, intentionally including a PID and hostname field to support
future multi-instance duplicate-open warnings without a format change.

## Technical Context

**Language/Version**: Rust stable, edition 2024
**Primary Dependencies**: `libc` 0.2, `ropey` 2.0.0-beta.1, `termion` 4.0 — no new deps permitted
  (runtime crate budget is fully consumed: libc, ropey, str_indices, termion, numtoa = 5/5)
**Storage**: Local filesystem; swap files at `$XDG_CACHE_HOME/ordex/swap/`
  (same XDG cache root used by existing session files)
**Testing**: `cargo test` — unit tests in `tests` sub-modules per convention
**Target Platform**: Linux (existing target; `libc` already provides `getpid`/`gethostname`)
**Project Type**: Single Rust binary at repository root
**Performance Goals**: Swap refresh must not block the event loop; write is synchronous but
  bounded to one `write_all` + `sync_all` + `rename` per save, which is acceptable for text
  files at editor scale
**Constraints**: Zero new runtime dependencies; swap-file I/O must use only `std::fs` and `libc`
**Scale/Scope**: One swap file per open normal-text buffer; targets single-user local editing

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Rule | Status | Notes |
|---|------|--------|-------|
| I | Runtime deps ≤ 5 transitive crates | ✅ PASS | No new deps added; all swap logic uses std only |
| I | No deps with proc macros / heavy build scripts | ✅ PASS | No new deps |
| II | Every function has a doc comment | ✅ PASS | Enforced in implementation per project instructions |
| II | Complex / >10-line functions have inline comments | ✅ PASS | Enforced in implementation |
| II | Narrowest visibility (private → pub(crate) → pub) | ✅ PASS | swap module internals stay `pub(crate)` |
| II | Methods on types over free functions | ✅ PASS | Swap I/O grouped under `SwapFile` type |
| II | No trailing whitespace; `cargo fmt` + `cargo clippy` clean | ✅ PASS | Enforced before commit |
| III | Unit tests in `tests` sub-module | ✅ PASS | Each new sub-module gets its own `tests` block |
| IV | Single project at repo root | ✅ PASS | New `src/swap/` module inside existing crate |
| V | Docs updated in same change | ✅ PASS | `docs/src/configuration.md` and `docs/src/file-operations.md` updated |
| VI | Work in feature branch | ✅ PASS | Branch `008-swap-file-safety` |

**Pre-design gate: ALL PASS. No violations to justify.**

## Project Structure

### Documentation (this feature)

```text
specs/008-swap-file-safety/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── swap-module.md
└── tasks.md             # Phase 2 output (created by /speckit.tasks)
```

### Source Code (repository root)

```text
src/
├── swap/
│   ├── mod.rs           # SwapFile type: create, refresh, delete, open-for-recovery
│   ├── format.rs        # ordex-swap-v1 read/write (header parse + raw content)
│   ├── location.rs      # XDG path resolution + path-encoding for swap filenames
│   └── glob.rs          # Simple *-wildcard pattern matcher (no path-sep restriction)
├── config/
│   └── validator.rs     # Add `swap_exclude_patterns: Vec<String>` to ConfigSettings
├── editor_state/
│   └── buffers.rs       # Add `swap: Option<SwapHandle>` field to BufferState
├── app.rs               # Upgrade execute_deferred_write: atomic rename + sync_all +
│                        # swap deletion after durable confirmation
└── swap.rs              # Module declaration (pub(crate) mod swap)

tests/                   # Existing integration test directory
└── (new swap integration tests alongside existing files)

docs/src/
├── configuration.md     # Document [swap] exclude setting
└── file-operations.md   # Document recovery workflow and swap lifecycle
```

**Structure Decision**: Single-project layout. The `src/swap/` sub-directory groups all
swap-file logic behind a clean module boundary, following the pattern already established by
`src/config/`, `src/editor_state/`, and `src/syntax/`.
