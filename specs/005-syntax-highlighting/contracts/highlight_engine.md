# Module Interface Contract: Highlight Engine

**Module**: `src/syntax/engine.rs`  
**Purpose**: Own document highlight state, perform full-load lexing, and incrementally relex after edits  
**Date**: 2026-03-11

## Public API

```rust
pub(crate) struct HighlightSpan {
    pub(crate) line_index: usize,
    pub(crate) start_col: usize,
    pub(crate) end_col: usize,
    pub(crate) class: SyntaxClass,
    pub(crate) modifier: Option<SyntaxModifier>,
}

pub(crate) struct BufferEdit {
    pub(crate) start_line: usize,
    pub(crate) old_end_line: usize,
    pub(crate) new_end_line: usize,
}

pub(crate) struct SyntaxEngine {
    // private fields
}

impl SyntaxEngine {
    pub(crate) fn new() -> Self;
    pub(crate) fn open_document(&mut self, path: Option<&Path>, buffer: &TextBuffer);
    pub(crate) fn apply_edit(&mut self, buffer: &TextBuffer, edit: BufferEdit);
    pub(crate) fn active_profile(&self) -> Option<LanguageId>;
    pub(crate) fn spans_for_line(&self, line_index: usize) -> &[HighlightSpan];
    pub(crate) fn generation(&self) -> u64;
    pub(crate) fn is_fully_lexed(&self) -> bool;
}
```

## Responsibilities

- Detect the active built-in profile when a document is opened
- Run a full-document lex pass for supported documents on load
- Maintain per-line state so multiline constructs and nested D comments stay correct
- Relex only the affected region plus forward dependencies after edits
- Expose ordered line-local spans for rendering
- Fall back safely to plain text when no profile matches

## Required Behaviors

- Load-time highlighting must reach full-document correctness for supported 50,000-line files
- Line-state is carry-over context between lines, not independent line-by-line lexing
- Edit-time relexing must continue forward until line exit state stabilizes
- Phase 1 edit-time lexing stays synchronous on the main thread; the engine must not expose partially stale background results
- Unsupported constructs must remain readable instead of being aggressively guessed
- Generation changes must let rendering notice visual changes even when cursor/viewport stay still

## Testing Requirements

- full-load lexing for Rust, config/TOML, Markdown, and D
- nested D comment correctness
- multiline string/comment recovery after delimiter edits
- 50,000-line open/edit/scroll behavior
- deterministic plain-text fallback for unsupported files
