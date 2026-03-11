# Implementation Plan: Syntax Highlighting

**Branch**: `001-syntax-highlighting` | **Date**: 2026-03-11 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/001-syntax-highlighting/spec.md`

## Summary

Add syntax highlighting to Ordex for Rust, config/TOML, conservative-core Markdown, and D without introducing new runtime dependencies or tree-sitter.  
The chosen design is a single-threaded hand-written incremental line-state lexer with a full-document top-to-bottom pass on open and a synchronous forward-to-stability relex after edits, backed by built-in language profiles that keep comment metadata, documentation-comment variants, syntax classes, and future nested-language hooks reusable across rendering, theme work, and future comment commands.

## Technical Context

**Language/Version**: Rust stable (edition 2024)  
**Primary Dependencies**: Existing runtime dependencies only (`termion` 4.0.6, `ropey` 2.0.0-beta.1, `libc` 0.2.180); no new runtime crates planned  
**Storage**: Local files loaded into the existing rope-backed text buffer; highlight cache stored in editor state only (not persisted)  
**Testing**: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test -- --test-threads=1`, plus inline unit tests and PTY integration tests in `tests/`  
**Target Platform**: POSIX terminals with ANSI support (Linux/macOS), current development target Linux  
**Project Type**: Single native CLI/TUI application  
**Performance Goals**:
- Full-document highlighting for supported files up to 50,000 lines is correct within 3 seconds on initial open
- Full-document highlighting remains correct after scrolling without delayed off-screen catch-up
- 95% of single-line insert/delete edits update the affected highlighting within 0.2 seconds
- Wrapped and unwrapped rendering paths remain responsive while highlighting is active
**Constraints**:
- No tree-sitter
- No new heavy, proc-macro, or build-script-heavy runtime dependencies
- No background lexing thread in phase 1 unless profiling later proves the single-threaded design misses the stated large-file targets
- Phase 1 supports Rust, config/TOML, conservative-core Markdown, and D only
- Language detection uses filename and extension matching only in phase 1
- Language profiles must support multiple comment styles, documentation-comment variants where the language defines them, and a preferred default ordinary comment style when multiple ordinary styles exist
- Design must preserve future theme support, nested syntax highlighting, comment continuation/toggle reuse, and a later bracket-jump feature
**Scale/Scope**:
- Single active editor buffer at a time
- Four built-in language profiles in phase 1
- Files up to 50,000 lines with both wrapped and horizontally scrolled rendering paths
- A new `src/syntax/` subsystem integrated into existing editor, rendering, and test modules

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Initial Check (Pre-Research)

| Rule | Status | Notes |
|------|--------|-------|
| Runtime dependencies must stay minimal | ✅ PASS | Verified current non-root runtime crate set is already `termion`, `ropey`, `str_indices`, `libc`, `numtoa`; there is no room for new runtime crates |
| No proc-macro or heavy build-script dependencies | ✅ PASS | Planned syntax engine is implemented in-repo with existing crates only |
| Feature branch workflow | ✅ PASS | Active branch is `001-syntax-highlighting` |
| User-facing docs updated in same change | ✅ PASS | Plan includes `docs/src/syntax-highlighting.md` plus `SUMMARY.md` and `index.md` updates |
| Test risky logic directly | ✅ PASS | Plan includes lexer/profile unit tests and PTY integration tests for ANSI rendering and large-file behavior |

**Initial Status**: ✅ PASS - Proceed to Phase 0 research.

### Post-Design Check

| Rule | Status | Notes |
|------|--------|-------|
| Runtime dependencies must stay minimal | ✅ PASS | Final design adds zero runtime crates and stays within the existing five-crate budget |
| No proc-macro or heavy build-script dependencies | ✅ PASS | Lexer, language profiles, and Markdown rules remain hand-written in-repo |
| Prefer methods on types / narrow visibility | ✅ PASS | `SyntaxEngine`, `LanguageProfile`, and `DocumentHighlightState` own their behavior; helper functions can remain private or `pub(crate)` |
| User-facing docs updated in same change | ✅ PASS | Quickstart and structure plan call out docs site updates explicitly |
| Test risky logic directly | ✅ PASS | Contracts and quickstart include unit, render, edit, wrap, and large-file validation surfaces |
| Feature branch workflow | ✅ PASS | Branch remains `001-syntax-highlighting` |

**GATE STATUS**: ✅ PASS - No constitution violations or unresolved gate failures remain.

## Project Structure

### Documentation (this feature)

```text
specs/001-syntax-highlighting/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── editor_syntax.md
│   ├── highlight_engine.md
│   ├── language_profile.md
│   └── markdown_scope.md
└── tasks.md             # Created later by /speckit.tasks
```

### Source Code (repository root)

```text
src/
├── main.rs
├── editor_state.rs
├── text_buffer.rs
├── tui.rs
├── navigation.rs
├── config.rs
├── config/
│   └── ...existing modules
├── syntax.rs                    # NEW: syntax subsystem entry points and shared exports
├── syntax/
│   ├── engine.rs                # NEW: incremental line-state highlighting engine and cache invalidation
│   ├── profile.rs               # NEW: SyntaxClass, SyntaxModifier, LanguageProfile, CommentStyle
│   ├── helpers.rs               # NEW: shared boundary/context helpers for profile rules
│   └── profiles/
│       ├── mod.rs               # NEW: registry and filename/extension detection
│       ├── rust.rs              # NEW: Rust profile and rules
│       ├── toml.rs              # NEW: config/TOML profile and rules
│       ├── markdown.rs          # NEW: Markdown profile using the generic engine with conservative helper-driven rules
│       └── d.rs                 # NEW: D profile and rules, including nested comments
└── ...existing modules

tests/
├── syntax_highlighting_test.rs  # NEW: ANSI-highlight rendering for supported languages
├── syntax_large_file_test.rs    # NEW: 50k-line open/edit/scroll correctness and responsiveness
├── soft_wrap_test.rs            # UPDATE: wrapped-row highlight boundaries
├── editing_test.rs              # UPDATE: edit-driven relex and cache invalidation
└── ...existing integration tests

docs/src/
├── syntax-highlighting.md       # NEW: supported languages, current limits, fallback behavior
├── SUMMARY.md                   # UPDATE: add syntax-highlighting page
├── index.md                     # UPDATE: mention syntax highlighting in user guide overview
└── ...existing docs
```

**Structure Decision**: Extend the current single-project Rust layout with a dedicated `src/syntax/` subsystem. Keep document text in `TextBuffer`, derived highlight state in `EditorState`, one built-in language profile per file under `src/syntax/profiles/`, and final ANSI style emission in the existing render path (`main.rs` + `tui.rs`) so syntax analysis remains separate from text storage and terminal control. `Line-state` here means each line stores the continuation state inherited from the previous line; the initial load still lexes the whole file from top to bottom so multiline comments and strings remain correct.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none) | — | — |
