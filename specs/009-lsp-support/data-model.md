# Data Model: Rust Code Navigation MVP

## 1. LspWorkspaceKey

Represents the canonical Rust project context used to decide whether a rust-analyzer session can be reused.

| Field | Type | Description |
|-------|------|-------------|
| `root_path` | Canonical path | Stable workspace root used as the session key |
| `root_kind` | Enum | Either `cargo-workspace` or `rust-project-json` |
| `manifest_path` | Optional path | The `Cargo.toml` or `rust-project.json` that established the root |

**Validation rules**

- `root_path` must be canonical and absolute.
- Two workspaces are the same only when their canonical `root_path` values match.
- Files without a recognized root do not produce a workspace key in the MVP.

## 2. LspSession

Represents one long-lived rust-analyzer process shared by all buffers inside one `LspWorkspaceKey`.

| Field | Type | Description |
|-------|------|-------------|
| `workspace_key` | `LspWorkspaceKey` | Project scope for the session |
| `status` | Enum | `starting`, `ready`, `shutting-down`, or `failed` |
| `open_documents` | Map | Tracks synced buffers by buffer id |
| `pending_requests` | Map | Tracks in-flight definition lookups by request id |
| `last_error` | Optional string | Most recent user-visible failure summary |

**Validation rules**

- There may be at most one live `LspSession` per `LspWorkspaceKey`.
- A session must reach `ready` before definition requests are sent.
- A failed session must surface actionable feedback before it is retried or replaced.

**State transitions**

`starting -> ready -> shutting-down`  
`starting -> failed`  
`ready -> failed`  

## 3. LspDocumentState

Represents one Rust buffer as seen by the shared LSP session.

| Field | Type | Description |
|-------|------|-------------|
| `buffer_id` | Integer | Stable Ordex buffer identifier |
| `file_path` | Canonical path | Buffer file path used by rust-analyzer |
| `workspace_key` | `LspWorkspaceKey` | Owning project context |
| `version` | Integer | Monotonic document version sent with `didChange` |
| `sync_state` | Enum | `closed`, `open`, or `dirty-pending-sync` |

**Validation rules**

- Only Rust buffers in supported workspaces create `LspDocumentState`.
- `version` increases every time a new buffer snapshot is published before a lookup.
- `file_path` must remain stable for the lifetime of one open buffer unless the buffer is reopened under a different path.

**State transitions**

`closed -> open -> dirty-pending-sync -> open`  
`open -> closed`

## 4. DefinitionLookup

Represents one user-triggered go-to-definition request.

| Field | Type | Description |
|-------|------|-------------|
| `lookup_token` | Integer or UUID-like token | UI-facing token used to reject stale results |
| `request_id` | Integer or string | Session-local LSP request identifier |
| `buffer_id` | Integer | Source buffer that triggered the lookup |
| `buffer_version` | Integer | Document version active when the request was sent |
| `cursor_line` | Integer | Zero-based source line |
| `cursor_character` | Integer | Zero-based source character |
| `state` | Enum | `queued`, `in-flight`, `resolved-single`, `resolved-multiple`, `not-found`, `failed`, or `stale` |

**Validation rules**

- Only one active lookup token may be considered current for a given buffer at a time.
- A result is applicable only if `lookup_token`, `buffer_id`, and `buffer_version` still match the active request state.
- A lookup targeting an unsupported buffer must not enter `in-flight`.

## 5. DefinitionTarget

Represents one navigable destination returned for a definition lookup.

| Field | Type | Description |
|-------|------|-------------|
| `file_path` | Path | Destination file |
| `line` | Integer | Zero-based destination line |
| `character` | Integer | Zero-based destination character |
| `display_label` | String | Human-readable label shown when multiple targets exist |

**Validation rules**

- Targets must be normalized into file/line/character form before the editor consumes them.
- Multiple targets must preserve deterministic order for chooser rendering.

## 6. NavigationFeedback

Represents the user-visible outcome for one lookup attempt.

| Field | Type | Description |
|-------|------|-------------|
| `kind` | Enum | `success`, `unsupported-file`, `server-starting`, `not-found`, `multiple-targets`, or `error` |
| `message` | String | Status message rendered by Ordex |
| `lookup_token` | Integer or UUID-like token | Correlates the feedback with the active request |

**Validation rules**

- Failure feedback must preserve the original editing context.
- `multiple-targets` feedback must pair with a chooser state rather than an immediate jump.

## Relationships

- One `LspWorkspaceKey` owns zero or one `LspSession`.
- One `LspSession` owns many `LspDocumentState` records.
- One `LspDocumentState` can produce many `DefinitionLookup` records over time.
- One `DefinitionLookup` resolves to zero, one, or many `DefinitionTarget` records and exactly one `NavigationFeedback` outcome.
