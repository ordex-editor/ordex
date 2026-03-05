# Implementation Plan: Resilient Configuration Files

**Branch**: `004-add-config-files` | **Date**: 2026-03-05 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/004-add-config-files/spec.md`

## Summary

Add resilient, declarative configuration-file support with partial recovery and key-mapping preservation under failure, including `#` line comment support in config files.  
Given the hard no-dependency parser constraint, implement an in-repo TOML-like parser (section + key/value subset) with per-section fault isolation, warning aggregation, and defaults merge instead of adopting external parser crates.

## Technical Context

**Language/Version**: Rust stable (edition 2024)  
**Primary Dependencies**: Existing runtime deps only (`termion`, `ropey`, `libc`); no new parser crate  
**Storage**: Local filesystem config file(s) and optional included files  
**Testing**: `cargo test` (module unit tests + integration tests in `tests/`)  
**Target Platform**: POSIX terminals (Linux/macOS)  
**Project Type**: Single native CLI application  
**Performance Goals**:
- Parse and resolve typical config (<500 lines) in under 20ms on local disk
- Startup succeeds in >=95% cases where only non-key-mapping sections are invalid (SC-002)
- Valid key mappings remain available in 100% such cases (SC-003)
**Constraints**:
- No new runtime dependencies for parser implementation
- Unknown keys/sections must be non-fatal with warnings
- Missing include files must be recoverable (skip + default + warn)
- Warnings are surfaced on startup stderr/console
- `#` comments in config lines are supported and ignored by the parser
**Scale/Scope**:
- Single-user local config
- One main config and optional split/include files
- Tens to hundreds of settings and key mappings

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Initial Check (Pre-Research)

| Rule | Status | Notes |
|------|--------|-------|
| Runtime dependencies must stay minimal | ✅ PASS | Plan adds zero runtime crates |
| No proc-macro/heavy build-script deps | ✅ PASS | Parser is implemented in-repo with std only |
| Test coverage for risky logic required | ✅ PASS | Plan includes parser, recovery, and conflict tests |
| Docs must be updated in same change | ✅ PASS | Plan includes docs updates for config behavior |
| Feature-branch workflow | ✅ PASS | Branch is `004-add-config-files` |

### Post-Design Check

| Rule | Status | Notes |
|------|--------|-------|
| Runtime dependencies must stay minimal | ✅ PASS | Design maintains current dependency graph |
| No proc-macro/heavy build-script deps | ✅ PASS | No external parsing framework introduced |
| Test coverage for risky logic required | ✅ PASS | Data model/quickstart/contracts include failure-path testing surfaces |
| Docs must be updated in same change | ✅ PASS | Quickstart explicitly includes docs updates |
| Feature-branch workflow | ✅ PASS | Branch unchanged |

**GATE STATUS**: ✅ PASS (pre- and post-design)

## Project Structure

### Documentation (this feature)

```text
specs/004-add-config-files/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   └── config-loader.openapi.yaml
└── tasks.md             # Created later by /speckit.tasks
```

### Source Code (repository root)

```text
src/
├── main.rs
├── editor_state.rs
├── keybindings.rs
├── config.rs                    # NEW: public loading entry points
├── config/
│   ├── parser.rs                # NEW: home-made TOML-like parser
│   ├── include_loader.rs        # NEW: include discovery and read behavior
│   ├── validator.rs             # NEW: known-key validation + defaults mapping
│   ├── loader.rs                # NEW: per-section apply + recovery orchestration
│   ├── warnings.rs              # NEW: warning events and stderr formatting
│   └── keymap_merge.rs          # NEW: last-definition-wins conflict handling
└── ...existing modules

tests/
├── config_parser_test.rs              # NEW: syntax and tokenization behavior
├── config_loading_test.rs             # NEW: mixed known/unknown + defaults
├── config_keymap_resilience_test.rs   # NEW: keymap retained under unrelated failures
├── config_include_missing_test.rs     # NEW: missing include recovery path
└── ...existing integration tests

docs/src/
├── configuration.md                   # NEW
├── troubleshooting.md                 # UPDATE
└── SUMMARY.md                         # UPDATE
```

**Structure Decision**: Extend current single-project Rust layout with a dedicated `src/config/` subsystem so parsing, validation, recovery, and warning surfaces remain isolated and independently testable.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none) | — | — |
