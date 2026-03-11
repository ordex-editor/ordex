# Research: Syntax Highlighting

**Date**: 2026-03-11  
**Feature**: 005-syntax-highlighting

## Decision 1

- **Decision**: Add no new runtime dependencies for syntax highlighting.
- **Rationale**: Ordex already sits at its constitution-level runtime dependency budget, so the highlighting design must stay within the existing `termion`, `ropey`, `libc`, `numtoa`, and `str_indices` crate graph. A hand-written in-repo subsystem also keeps behavior easy to tune for editor-specific correctness and performance goals.
- **Alternatives considered**:
  - Adopting a parser or lexer crate with additional runtime dependencies.
  - Introducing a full grammar framework for highlighting.

## Decision 2

- **Decision**: Keep lexing on the main thread in phase 1 and use a hand-written incremental line-state lexer with a full-document top-to-bottom lex pass on open/load and a synchronous forward-to-stability relex after edits.
- **Rationale**: This best matches the clarified promise of correct full-document highlighting after open and scroll for files up to 50,000 lines. A background worker would add stale-highlight latency, generation-merging complexity, and redraw coordination costs without reducing the total CPU work; the initial implementation should first rely on a fast single-threaded lexer and only revisit threading if profiling proves the large-file target cannot be met.
- **Alternatives considered**:
  - Full-file relex after every edit.
  - Background worker lexing for the whole document.
  - Viewport-first lexing plus background catch-up.
  - Parser-based or grammar-generated approaches.

## Decision 3

- **Decision**: Keep syntax state in `EditorState`, not `TextBuffer`, and expose a highlight generation counter to rendering decisions.
- **Rationale**: `TextBuffer` should remain the rope-backed text abstraction, while syntax highlighting is derived visual state tied to file path, active language, and render invalidation. `EditorState` already owns the viewport-facing state and integrates naturally with `RenderSnapshot` and `render_row_content()`, so it is the right place for highlight cache ownership and redraw signaling.
- **Alternatives considered**:
  - Embedding token metadata directly into `TextBuffer`.
  - Recomputing highlight output during rendering with no persistent cache.

## Decision 4

- **Decision**: Represent each built-in language with a semantic `LanguageProfile` containing filename/extension detection, reusable syntax classes/modifiers, comment styles, and future nested-language hooks.
- **Rationale**: A small profile schema keeps language behavior data-driven without overcommitting to a grammar framework. It also preserves shared metadata for future theme work and future comment-continuation/toggle behavior, while keeping phase-1 detection limited to filename and extension matching as clarified in the spec.
- **Alternatives considered**:
  - Hard-coded per-language lexers with no shared profile data.
  - A single monolithic config object that combines highlighting, indent, theme, and bracket behavior.
  - Tree-sitter/TextMate-style grammar definitions.

## Decision 5

- **Decision**: Treat multiple comment styles and documentation-comment variants as first-class profile data, and require a preferred default ordinary comment style whenever a language supports more than one ordinary style.
- **Rationale**: D requires both regular and nested block comments in phase 1, and Rust/D documentation comments are worth styling differently for readability and theme flexibility. Encoding this metadata now keeps highlighting and future comment-toggle/comment-continuation behavior aligned without tying those future features to lexer internals.
- **Alternatives considered**:
  - Recognize multiple comment styles for highlighting but defer default selection.
  - Treat documentation comments as ordinary comments with no distinct modifier.
  - Keep comment metadata inside lexer code rather than the shared profile model.

## Decision 6

- **Decision**: Keep Markdown on the same generic highlighting engine as the other languages, but give it its own profile module with conservative block and inline rules plus shared helper predicates for boundary-sensitive cases.
- **Rationale**: This preserves one engine and one profile-per-file structure while still acknowledging that Markdown needs a few context helpers beyond keyword/comment rules. Helper-style predicates such as delimiter-boundary checks are a clean extension point and keep the path open for future profiles like AsciiDoc without introducing a separate Markdown-only lexer architecture.
- **Alternatives considered**:
  - Treat Markdown exactly like a code language with only keyword/comment-style rules.
  - Build a fully separate Markdown lexer pipeline.
  - Exclude Markdown entirely from phase 1.
  - Adopt a full Markdown parser.

## Decision 7

- **Decision**: Keep future indent metadata separate from highlighting profiles, and defer recording matched bracket locations in lexer output.
- **Rationale**: Indentation behavior and bracket jumping are future features with different change frequencies and runtime needs than highlighting. Static bracket-pair definitions may later live alongside language metadata, but phase-1 highlighting should emit spans and line state only, not maintain live match tables that the current editor does not yet consume.
- **Alternatives considered**:
  - Embedding indent settings directly into `LanguageProfile` now.
  - Recording bracket match locations during every lex pass in phase 1.
