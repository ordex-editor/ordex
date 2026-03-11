# Research: Syntax Highlighting

**Date**: 2026-03-11  
**Feature**: 001-syntax-highlighting

## Decision 1

- **Decision**: Add no new runtime dependencies for syntax highlighting.
- **Rationale**: Ordex already sits at its constitution-level runtime dependency budget, so the highlighting design must stay within the existing `termion`, `ropey`, `libc`, `numtoa`, and `str_indices` crate graph. A hand-written in-repo subsystem also keeps behavior easy to tune for editor-specific correctness and performance goals.
- **Alternatives considered**:
  - Adopting a parser or lexer crate with additional runtime dependencies.
  - Introducing a full grammar framework for highlighting.

## Decision 2

- **Decision**: Use a hand-written incremental line-state lexer with a full-document lex pass on open/load and a forward-to-stability relex after edits.
- **Rationale**: This best matches the requirement for correct full-document highlighting on files up to 50,000 lines while still keeping edit latency bounded. It handles multi-line strings, block comments, nested D comments, and Markdown fenced-code state without reparsing unrelated stable regions on every edit.
- **Alternatives considered**:
  - Full-file relex after every edit.
  - Parser-based or grammar-generated approaches.
  - Viewport-only lazy highlighting with delayed off-screen catch-up.

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

- **Decision**: Treat multiple comment styles as first-class profile data, and require a preferred default comment style whenever a language supports more than one style.
- **Rationale**: D requires both regular and nested block comments in phase 1, and future comment-toggle/comment-continuation behavior needs deterministic defaults. Encoding this metadata now keeps highlighting and future editing features aligned without tying those future features to lexer internals.
- **Alternatives considered**:
  - Recognize multiple comment styles for highlighting but defer default selection.
  - Keep comment metadata inside lexer code rather than the shared profile model.

## Decision 6

- **Decision**: Implement Markdown with a conservative hybrid rule set: line-anchored block recognition plus a small inline scanner, and leave complex constructs plain rather than aggressively guessed.
- **Rationale**: Markdown is explicitly in scope only for conservative core highlighting, and its irregular constructs make a generic code-style lexer unsafe if it tries to be too clever. Recognizing only unambiguous headings, fences, inline code, quotes, list markers, simple emphasis, simple inline links/images, and thematic breaks satisfies documentation readability goals while also respecting the "weird lexical syntax" concern by preferring plain text over incorrect coloring.
- **Alternatives considered**:
  - Treat Markdown exactly like a code language with the same rule depth as Rust or D.
  - Exclude Markdown entirely from phase 1.
  - Adopt a full Markdown parser.

## Decision 7

- **Decision**: Keep future indent metadata separate from highlighting profiles, and defer recording matched bracket locations in lexer output.
- **Rationale**: Indentation behavior and bracket jumping are future features with different change frequencies and runtime needs than highlighting. Static bracket-pair definitions may later live alongside language metadata, but phase-1 highlighting should emit spans and line state only, not maintain live match tables that the current editor does not yet consume.
- **Alternatives considered**:
  - Embedding indent settings directly into `LanguageProfile` now.
  - Recording bracket match locations during every lex pass in phase 1.
