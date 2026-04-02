# Data Model: Completion Support

## Overview

The MVP adds transient completion state to the active editor session. The model is intentionally scoped to the current buffer, but the source boundary is designed so future file-path, LSP, or plugin providers can plug into the same session flow.

## Entities

### CompletionSource

Represents one provider of completion candidates.

| Field | Type | Description | Validation |
|-------|------|-------------|------------|
| `source_id` | enum | Canonical source identifier (`buffer-text` for MVP; future `file-path`, `lsp`, `plugin`) | Must be unique within a completion session |
| `kind` | enum | `synchronous` or `asynchronous` execution model | MVP uses `synchronous` only |
| `enabled` | boolean | Whether the source may contribute candidates | MVP defaults to `true` for `buffer-text` |
| `priority` | integer | Stable source ordering for future multi-source ranking | Reserved for future sources |

**Relationships**:
- One `CompletionSession` aggregates candidates from one or more `CompletionSource` values.
- The MVP always uses exactly one source: `buffer-text`.

### CompletionRequest

Captures one attempt to resolve completion for the current editing context.

| Field | Type | Description | Validation |
|-------|------|-------------|------------|
| `buffer_id` | integer | Active buffer identifier at request time | Must match the current active buffer |
| `request_generation` | integer | Monotonic session generation used to discard stale work | Must increase when the prefix or buffer context changes |
| `trigger_kind` | enum | `automatic` or `manual` | MVP uses `automatic` for normal typing |
| `prefix_start_char_idx` | integer | Start of the replaceable prefix in the buffer | Must be on the same logical word as the cursor |
| `cursor_char_idx` | integer | Cursor position when completion is requested | Must be at or after `prefix_start_char_idx` |
| `prefix_text` | string | Current typed prefix | Must contain at least 1 character |
| `normalized_prefix` | string | Case-folded prefix used for matching | Derived from `prefix_text` |
| `min_candidate_length` | integer | Candidate-length gate | Fixed at `3` for the MVP |

**Relationships**:
- One `CompletionRequest` may produce zero or one `CompletionSession`.
- A new relevant Insert-mode edit replaces the previous active request.

### CompletionCandidate

Represents one suggested insertion.

| Field | Type | Description | Validation |
|-------|------|-------------|------------|
| `candidate_id` | string | Stable per-session identifier | Unique within the session |
| `source_id` | enum | Source that produced the candidate | Must reference an enabled `CompletionSource` |
| `display_text` | string | Text shown in the popup | Must be at least 3 characters |
| `insert_text` | string | Text inserted when this candidate is previewed | Preserves original buffer casing |
| `normalized_key` | string | Case-folded value used for deduplication | Used to collapse visually identical duplicates |
| `replace_start_char_idx` | integer | Start of replacement range | Must equal the request prefix start |
| `replace_end_char_idx` | integer | End of replacement range | Must equal the request cursor position |
| `rank` | integer | Stable sort rank | Lower rank means higher display priority |

**Validation rules**:
- Must extend the current prefix rather than replace it with unrelated text.
- Must preserve original casing in `insert_text`.
- Must not create duplicate visible suggestions after normalization.

### CompletionSession

Tracks the currently visible completion state.

| Field | Type | Description | Validation |
|-------|------|-------------|------------|
| `session_id` | string | Stable active-session identifier | Unique within the editor process |
| `buffer_id` | integer | Buffer that owns the session | Must match the active buffer while visible |
| `request_generation` | integer | Generation that produced the session | Must match the latest active request |
| `prefix_text` | string | Live prefix currently being completed | Must stay aligned with the replacement range |
| `prefix_start_char_idx` | integer | Start of the replaceable prefix | Must remain valid while the session is visible |
| `cursor_char_idx` | integer | Cursor position for replacement | Must remain valid while the session is visible |
| `selected_index` | integer or null | Currently highlighted candidate, or no selection | Must point to an existing candidate or be null |
| `candidates` | list of `CompletionCandidate` | Visible candidate set | May be empty only transiently before dismissal |
| `original_prefix_text` | string | The user-typed prefix before any preview is applied | Must be restorable while the session is visible |
| `preview_text` | string or null | The candidate currently previewed in the buffer | Null when no item is selected |
| `state` | enum | `active`, `dismissed` | Exactly one active session at a time |

**Relationships**:
- One `CompletionSession` belongs to one `CompletionRequest`.
- One `CompletionSession` owns zero or more `CompletionCandidate` values.

## State Transitions

### CompletionSession lifecycle

| From | Event | To |
|------|-------|----|
| `none` | Insert-mode edit creates a valid prefix with matching candidates | `active` |
| `none` | Insert-mode edit creates a valid prefix with no matching candidates | `none` |
| `active` | Relevant typed edit keeps the prefix valid | `active` (refreshed) |
| `active` | Selection navigation changes highlighted row | `active` |
| `active` | Selection navigation reaches no selected item | `active` (prefix restored) |
| `active` | Cursor move, buffer switch, or invalidating edit occurs | `dismissed` |
| `dismissed` | Next qualifying edit | `active` or `none` |

## Derived Rules

- Only one completion session may be visible at a time.
- The selected candidate, if any, is previewed directly in the buffer.
- A session must preserve the original typed prefix so it can be restored when selection returns to none.
- The MVP never forces a candidate to remain selected, even when exactly one candidate exists.
- Session invalidation is preferred over patching stale suggestions in place.
- Future asynchronous sources must publish results against `request_generation` so stale responses can be ignored safely.
