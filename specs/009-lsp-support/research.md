# Phase 0 Research: Rust Code Navigation MVP

## Decision 1: Keep definition lookups off the input path with an app-owned LSP manager

**Decision**: Use a project-scoped `LspManager` owned by `src/app.rs`, backed by worker threads and `std::sync::mpsc` channels. `EditorState` should emit a new deferred request for go-to-definition and hold only lightweight UI state such as the active lookup token, last known request status, and any chooser state for multiple targets.

**Rationale**: Ordex already has a proven background-work model in the file picker: the app loop switches from blocking input to timed polling only while background work exists, and `EditorState::poll_background_tasks()` applies visible updates without freezing the UI. Reusing that pattern avoids introducing an async runtime while still keeping rust-analyzer startup, JSON-RPC I/O, and response waits away from the input loop. The app layer already owns process-level side effects through `EditorRequest`, so it is the right place to own child-process lifecycle as well.

**Alternatives considered**:

- Put child-process management directly inside `EditorState`. This would reuse some file-picker patterns but would mix filesystem/process concerns into editor-local state that is currently intentionally separated by `EditorRequest`.
- Add an async runtime such as Tokio. This would solve concurrency but violates the repository's dependency and simplicity goals for a narrow MVP.
- Block the input loop until the language server responds. This is simplest to implement but directly violates the feature requirement to avoid UI freezes.

## Decision 2: Reuse one rust-analyzer process per canonical Rust workspace root

**Decision**: Determine the active file's project context by canonicalizing the file path, walking upward to the nearest `rust-project.json` or `Cargo.toml`, and, for `Cargo.toml`, resolving the actual Cargo `workspace_root`. Reuse exactly one rust-analyzer session per resolved workspace root and create a separate session only when a file belongs to a different root.

**Rationale**: This keeps multiple buffers from the same project on one shared analysis context while isolating unrelated projects so lookups stay correct. It also fits Ordex's current multi-buffer behavior, where several files can remain open while the active buffer changes frequently. Reusing by workspace root avoids wasteful per-buffer servers and makes project switching a routing problem instead of a server churn problem.

**Alternatives considered**:

- One server per buffer. This avoids root detection complexity but multiplies process count unnecessarily and loses shared project state.
- One global server for every open Rust file. This could work in a future multi-root design, but it adds lifecycle complexity and ambiguous project scoping that the MVP does not need.
- Support loose standalone Rust files. This would require fallback behavior with unclear correctness guarantees; the MVP is safer if it clearly reports unsupported files outside a recognized Rust workspace.

## Decision 3: Add `json` and keep LSP framing dependency-free

**Decision**: Add the `json` crate as the single new dependency and implement `Content-Length` framing, process I/O, and narrow JSON-RPC message modeling with the Rust standard library.

**Rationale**: The repository constitution strongly limits dependencies, and the feature only needs a small LSP surface: `initialize`, `initialized`, `didOpen`, full-text `didChange`, `textDocument/definition`, and `shutdown`/`exit`. Manual framing is straightforward with `std::io`, but manual JSON parsing would add unnecessary correctness risk. The `json` crate keeps the dependency footprint minimal while still making dynamic LSP payload construction and parsing manageable for the MVP.

**Alternatives considered**:

- Add no dependency at all. This is possible but would force hand-written JSON parsing that is harder to validate and maintain.
- Add `serde_json`, `lsp-types`, or a full JSON-RPC/LSP client stack. Those options are more ergonomic but add too much dependency weight and complexity for the repository rules and MVP scope.

## Decision 4: Version documents and reject stale results instead of canceling requests aggressively

**Decision**: Before a definition request is sent, publish the current buffer snapshot to the matching rust-analyzer session using `didOpen` and full-text `didChange` messages keyed by a per-buffer version counter. When a response arrives, apply it only if its lookup token, buffer id, and buffer version still match the active request state; otherwise discard it as stale.

**Rationale**: This keeps the user-visible result aligned with the current editor state without requiring invasive cancellation machinery. It also handles the common race where the user keeps typing, changes buffers, or triggers a second lookup before the first one completes. Ordex already chooses bounded polling plus state comparison in background UI work, so stale-result rejection matches the existing architectural style.

**Alternatives considered**:

- Attempt to cancel every superseded lookup in flight. That adds coordination complexity and is not necessary for MVP correctness.
- Ignore unsaved edits and query the on-disk file only. That would produce confusing definition results when the open buffer differs from disk.
- Always apply the latest response, even if the editor moved on. That risks jumping the user to the wrong place after they changed context.
