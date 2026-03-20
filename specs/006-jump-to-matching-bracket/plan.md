# Implementation Plan: Jump To Matching Bracket

**Branch**: `006-jump-to-matching-bracket` | **Date**: 2026-03-20 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/006-jump-to-matching-bracket/spec.md`

## Summary

Add Vim-style `%` matching to Ordex using a syntax-aware on-demand scan that reuses the existing sparse-checkpoint lexer state, supports `()[]{}` and `<>`, matches syntax-profile block comments, falls back to plaintext matching inside ignored regions, and exposes a visible-only passive highlight for the active pair.

The implementation deliberately avoids turning the lexer into a global pair table. V1 uses a read-only syntax replay API plus a generation-scoped endpoint cache, leaving SIMD and Xi-style chunk summaries as follow-up work if profiling shows the on-demand scan is too expensive on large files.

## Technical Context

**Language/Version**: Rust stable (edition 2024)  
**Primary Dependencies**: Existing runtime dependencies only; no new crates planned  
**Storage**: Existing rope-backed `TextBuffer`, existing syntax-checkpoint state, plus ephemeral match state and endpoint cache in editor/navigation state  
**Testing**: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, plus integration tests for navigation and rendering  
**Target Platform**: POSIX terminals with ANSI support (Linux/macOS)  
**Project Type**: Single native CLI/TUI application  
**Performance Goals**:
- `%` should feel immediate on already-visible or moderately distant matches
- repeated `%` on the same endpoints should be cheap after the first resolution
- passive highlighting must not introduce off-screen scan work
**Constraints**:
- No tree-sitter
- No new runtime dependencies
- No dense whole-file pair map in lexer state
- Count semantics for `%` must remain Vim-compatible
- Visible syntax span cache must not be mutated purely for `%` matching
- Any later SIMD work must satisfy constitution rules for isolated unsafe code and profiling justification
**Scale/Scope**:
- One editor buffer at a time
- Existing syntax profiles remain the source of block-comment metadata
- V1 delivers `%` motion, block-comment matching, cache, and passive visible-pair highlight

## Constitution Check

*GATE: Must pass before implementation begins. Re-check after design changes.*

### Initial Check

| Rule | Status | Notes |
|------|--------|-------|
| Runtime dependencies must stay minimal | PASS | Plan adds no new dependencies |
| No proc-macro or heavy build-script dependencies | PASS | Feature reuses in-repo lexer and editor state only |
| Prefer narrow visibility and behavior on owning types | PASS | New scan/cache APIs can stay private or `pub(crate)` |
| User-facing docs updated in same change | PASS | `%` behavior and highlighting rules must be documented when implemented |
| Test risky logic directly | PASS | Plan includes navigation, syntax, cache, and render tests |
| Feature branch workflow | PASS | Planned branch is `006-jump-to-matching-bracket` |

**GATE STATUS**: PASS

## Project Structure

### Documentation (this feature)

```text
specs/006-jump-to-matching-bracket/
├── plan.md
├── research.md
└── spec.md
```

### Source Code (repository root)

```text
src/
├── editor_state.rs          # UPDATE: add `%` action handling, endpoint cache, passive match state
├── keybindings.rs           # UPDATE: add `%` binding and action text
├── main.rs                  # UPDATE: render passive visible-pair highlight
├── navigation.rs            # UPDATE: delimiter targeting and match search logic
├── syntax/engine.rs         # UPDATE: read-only delimiter replay API from checkpoints
├── syntax/profile.rs        # UPDATE: expose block-comment metadata needed for matching
├── themes/mod.rs            # UPDATE: add passive-match style overlay
└── tui.rs                   # UPDATE: keep overlay composition predictable for syntax, match, and selection

tests/
├── navigation_test.rs       # UPDATE: `%` integration behavior
├── editing_test.rs          # UPDATE: cache invalidation after edits
└── ...existing render/syntax tests in src/main.rs and src/syntax/engine.rs
```

**Structure Decision**: Keep `%` split across the current responsibilities: navigation for scan logic, syntax engine for read-only delimiter classification, editor state for cache and current-match state, and rendering for passive highlighting.

## Implementation Phases

### Phase 1 - Motion semantics and key handling

- Add a dedicated `%` action and bind it in normal mode.
- Preserve existing count handling so a prefixed count still routes `%` to percentage-of-file motion.
- Add line-local candidate targeting:
  - use the delimiter under the cursor when present
  - treat the cursor as being "on" a block-comment delimiter when it is inside any delimiter character
  - otherwise search right on the current logical line for the next supported delimiter

### Phase 2 - Syntax-aware matching engine

- Add a read-only syntax replay API that can classify delimiters from the nearest checkpoint without mutating the visible span window.
- Expose enough data from syntax replay to:
  - decide whether the current position is code, string, or comment
  - identify block-comment open/close tokens for the active profile
  - walk nested block-comment depth where supported
- Implement directional matching:
  - bracket depth walk for `()[]{}` and `<>`
  - block-comment delimiter walk using profile metadata
- Add ignored-region fallback:
  - if matching starts inside a string or comment and not on a block-comment delimiter, switch to plaintext matching confined to that ignored region

### Phase 3 - Cache and passive highlighting

- Add a generation-scoped endpoint cache keyed by resolved endpoint positions.
- Cache only complete matches, in both directions.
- Invalidate cache on edit or syntax generation change.
- Add editor state for the current visible passive match pair.
- Add a dedicated pale-match theme overlay.
- Update render composition so:
  - source delimiter is bold
  - mate gets pale background
  - selected mate in Visual mode becomes bold only
  - passive highlighting appears only when both endpoints are already visible

### Phase 4 - Documentation and validation

- Update user-facing docs for `%`, including:
  - supported delimiters
  - count behavior
  - block-comment matching
  - ignored-region fallback
  - visible-only passive highlighting
- Run formatting, clippy, and tests.
- Verify no `%` path mutates the visible syntax span cache accidentally.

## Test Plan

### Unit and module tests

- `src/navigation.rs`
  - symmetric matching for each bracket pair, including `<>`
  - nested matching across mixed bracket types
  - next-delimiter-on-line targeting
  - no-op when no candidate exists
- `src/syntax/engine.rs`
  - delimiter classification in code vs string/comment
  - block-comment opener/closer recognition
  - nested block-comment replay correctness
  - ignored-region boundary detection for plaintext fallback

### Integration tests

- `tests/navigation_test.rs`
  - `%` in representative multi-line code fixtures
  - `%` from inside strings/comments using fallback
  - count-prefixed `%` preserves percentage behavior
- `tests/editing_test.rs`
  - endpoint cache invalidates after edits
  - repeated `%` works after invalidation

### Render tests

- `src/main.rs` tests
  - visible pair highlights both endpoints when both are on screen
  - source delimiter is bold
  - mate gets pale-match background
  - Visual mode suppresses passive background on selected mate
  - off-screen mate produces no passive highlight

### Acceptance checks

- `%` matches nested pairs in real fixtures using `()[]{}` and `<>`
- `%` ignores misleading delimiters in strings/comments during code-mode matching
- `%` matches block comments in languages with ordinary and nested block comments
- repeated `%` across the same pair is responsive after first resolution

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Dedicated syntax replay API | Needed to reuse checkpoints without disturbing visible cache ownership | Reusing visible-window preparation would couple motion to render cache and create accidental invalidation |
| Dedicated passive match theme overlay | Needed to distinguish passive matching from true Visual selection | Reusing selection background exactly would blur two different UI states |

## Follow-up Work (Not In Scope For V1)

- SIMD delimiter discovery after profiling
- Xi-style chunk-summary index for long-distance skipping
- Parser-aware disambiguation of `<` / `>` in languages where angle brackets are ambiguous
- Off-screen-aware passive highlighting
