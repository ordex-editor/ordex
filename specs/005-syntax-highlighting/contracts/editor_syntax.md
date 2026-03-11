# Module Interface Contract: Editor Syntax Integration

**Modules**: `src/editor_state.rs`, `src/main.rs`, `src/tui.rs`  
**Purpose**: Integrate derived highlight state into editing, redraw decisions, and ANSI rendering  
**Date**: 2026-03-11

## Public API

```rust
impl EditorState {
    pub(crate) fn load_file(path: &Path, terminal_height: usize) -> Result<Self, io::Error>;
    pub(crate) fn syntax_generation(&self) -> u64;
    pub(crate) fn active_language_id(&self) -> Option<LanguageId>;
    pub(crate) fn syntax_spans_for_line(&self, line_index: usize) -> &[HighlightSpan];
}
```

`RenderSnapshot` in `src/main.rs` must include a syntax-generation field so syntax-only visual changes can trigger redraws.

`render_row_content()` must combine:

1. syntax-highlight spans,
2. visual-selection styling,
3. cursor emphasis,
4. wrap/horizontal-scroll clipping,

without corrupting ANSI output.

## Responsibilities

- Initialize syntax state when files are loaded
- Trigger relexing after buffer edits and reloads
- Expose highlight spans to rendering without leaking lexer internals
- Ensure wrapped and unwrapped rows both display correct styles
- Keep rendering responsive and deterministic

## Required Behaviors

- Syntax changes alone must be able to force a redraw
- Selection and cursor styles must remain visible on top of syntax colors
- Rendering logic must not assume one screen row equals one logical line
- Plain-text fallback must render identically to ordinary unhighlighted text

## Testing Requirements

- ANSI output snapshots for supported files
- wrapped-row highlighting boundaries
- edit-driven redraws when syntax changes but cursor stays put
- safe fallback rendering for unsupported files
