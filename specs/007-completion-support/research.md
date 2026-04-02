# Research: Completion Support

## Architecture boundary and code ownership

- **Decision**: Create a new `src/completion/` module, and keep `EditorState`, `render.rs`, and keybindings limited to lifecycle and UI integration.
- **Rationale**: Completion is an inline Insert-mode behavior that must keep writing directly into the active buffer while suggestions stay visible. The existing picker stack is optimized for modal query input and, in the file-picker case, asynchronous filesystem scanning. Keeping completion in its own module reduces coupling, prevents further growth in the already-large `EditorState`, and keeps future LSP/plugin sources from inheriting file-picker-specific concerns.
- **Alternatives considered**:
  - Extend `dialogs::PickerState` directly — rejected because pickers own a separate query buffer and modal state, while completion must stay in Insert mode and track live buffer edits.
  - Refactor all popup infrastructure first — rejected because it raises change risk and delays the MVP without solving a current correctness problem.

## MVP source strategy

- **Decision**: Ship only a synchronous `BufferTextCompletionSource` behind a small `CompletionSource` abstraction.
- **Rationale**: The current buffer text is already in memory via `TextBuffer`, so buffer-word extraction is the simplest MVP source and directly matches the clarified spec. A small source abstraction keeps file-path, LSP, and plugin providers possible later without building multi-source infrastructure before it is needed.
- **Alternatives considered**:
  - File-path completion first — rejected because it adds path parsing, working-directory, and unsaved-buffer semantics before the MVP proves the flow.
  - Multiple sources in the MVP — rejected because it expands ranking, cancellation, and testing scope too early.

## Insert-mode UX and popup behavior

- **Decision**: Keep the editor in Insert mode, show a dedicated completion popup driven by `CompletionSession`, apply the selected candidate directly to the buffer as a live preview, and let Up/Down navigate through candidates including a no-selection state that restores the original prefix.
- **Rationale**: This preserves ordinary typing semantics while aligning the UI with the clarified behavior that selection itself changes the editor text. It also avoids a new modal mode whose only purpose would be to mimic Insert mode while suggestions are visible, while still keeping cancellation explicit through the deselected state.
- **Alternatives considered**:
  - Introduce `Mode::Completion` — rejected because it mixes text-entry and popup-selection responsibilities and makes simple typing behavior harder to reason about.
  - Keep explicit accept/cancel keys such as Enter/Escape — rejected because the clarified behavior makes selection itself the preview/apply action and uses a no-selection state for cancellation.
  - Use a picker query buffer — rejected because completion suggestions should follow the live buffer prefix, not a second input model.

## Non-freezing strategy

- **Decision**: Keep MVP buffer completion synchronous, bounded, and recomputed from the active buffer on relevant Insert-mode edits; reserve background work for future external or expensive sources.
- **Rationale**: Buffer text is already local and does not require filesystem or network I/O. Introducing threads, channels, and cancellation for the MVP would add complexity before profiling shows the need. Ordex already has a safe background-work pattern for expensive tasks (`FilePickerState` + `app.rs` polling), so the design can defer async completion work until file-path, LSP, or plugin providers justify it.
- **Alternatives considered**:
  - Spawn a background worker for every completion refresh — rejected because the MVP source is in-memory and should be cheaper than thread orchestration.
  - Debounce all completion updates — rejected because the feature should appear after the first typed character and feel immediate.
  - Build a persistent global word index up front — rejected as premature optimization and extra invalidation complexity.

## Future async-source contract

- **Decision**: Design completion sessions around generation-aware requests so future async providers can be canceled or ignored when the buffer, cursor, or prefix changes.
- **Rationale**: VS Code’s completion provider contract passes both trigger context and a cancellation token, which shows that stale results are expected in real editor completion flows. Ordex’s existing file-picker pattern already uses background polling and bounded per-poll work, so a future async completion adapter can reuse the same high-level idea without affecting the MVP.
- **Alternatives considered**:
  - Accept whichever async result arrives last — rejected because stale candidates would violate the spec’s requirement to discard outdated suggestions.
  - Design only for synchronous sources — rejected because the feature is explicitly intended to grow into LSP and plugin completions.

## Freshness and invalidation rules

- **Decision**: Treat completion as ephemeral UI state: refresh after relevant insert-mode edits, restore the original typed prefix when navigation reaches no selection, and dismiss the session on cursor movement outside the active prefix, buffer switches, or any edit that invalidates the replacement range.
- **Rationale**: This satisfies both stale-suggestion handling and the newly clarified cancellation behavior without forcing an incremental index or complex edit reconciliation in the MVP.
- **Alternatives considered**:
  - Keep suggestions visible across arbitrary edits and try to patch them in place — rejected because it complicates correctness and replacement-range tracking.
  - Maintain a continuously updated buffer-wide index from day one — rejected because it adds maintenance cost before performance data requires it.

## Supporting references

- **Ordex local architecture**:
  - `src/editor_state/mod.rs` stores overlay state (`buffer_switch`, `file_picker`, `matching`) and owns background polling hooks.
  - `src/dialogs/file_picker.rs` shows the project’s current non-blocking pattern: worker thread, `mpsc` batches, bounded `poll()`, and deferred query updates.
  - `src/app.rs` enables timeout-based polling only when asynchronous work is active.
  - `src/dialogs/picker.rs`, `src/dialogs/buffer_switch.rs`, and `src/render.rs` show reusable popup rendering and list-selection patterns, but they are modal/query-driven rather than inline Insert-mode flows.
- **VS Code**:
  - `CompletionItemProvider` is triggered either explicitly or while typing and receives both trigger context and a cancellation token (`vscode.d.ts`).
- **Helix**:
  - Editor settings include `auto-completion`, `completion-timeout`, `completion-trigger-len`, `path-completion`, and `preview-completion-insert`, showing that automatic completion commonly uses configurable thresholds and may separate path completion from general completion.
- **Neovim**:
  - `completeopt` and insert-completion docs show popup-menu completion with explicit accept/cancel behavior and automatic buffer-text completion from multiple sources without forcing immediate insertion.
