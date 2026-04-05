# Data Model: Swap File Safety

**Branch**: `008-swap-file-safety` | **Date**: 2026-04-05
**Prerequisite**: `research.md` complete

---

## Entities

### SwapMeta (persisted, in swap file header)

Metadata stored in the `ordex-swap-v1` header block of each swap file.

| Field | Rust Type | Description |
|-------|-----------|-------------|
| `pid` | `u32` | Process ID of the ordex instance that created the swap file |
| `hostname` | `String` | Machine hostname at creation time (for future multi-instance detection) |
| `original_path` | `PathBuf` | Absolute path of the source file this swap protects |
| `opened_at` | `u64` | Unix timestamp (seconds) when the buffer was first opened |
| `last_refreshed_at` | `u64` | Unix timestamp (seconds) of the most recent swap write |

**Validation rules:**
- `pid` must be a positive non-zero decimal integer on parse.
- `original_path` must be an absolute path string (starts with `/`); relative paths are
  rejected with a parse error.
- `opened_at` ≤ `last_refreshed_at` (enforced on write; parse does not require this for
  resilience).
- `hostname` is stored as-is; empty strings are accepted on parse for forward compatibility.

---

### SwapExclusionPattern (config, in memory)

One user-configured glob pattern that suppresses swap-file creation for matching paths.

| Field | Rust Type | Description |
|-------|-----------|-------------|
| `pattern` | `String` | Raw glob string from config (e.g., `*.gpg`, `/dev/shm/gopass*`) |

**Matching semantics** (see `src/swap/glob.rs`):
- `*` matches any sequence of characters, including path separators (`/`).
- Match is applied to the absolute string representation of the file path.
- A file is excluded if it matches **any** pattern in the configured list.

**Config format** (new `[swap]` section in ordex config):

```toml
[swap]
exclude = ["/dev/shm/gopass*", "*.gpg"]
```

The `exclude` key accepts a TOML array of strings. An empty or absent `[swap]` section means
no files are excluded.

---

### SwapHandle (runtime, per buffer, in memory only)

Tracks the on-disk swap file associated with one open buffer. Stored on `BufferState`.

| Field | Rust Type | Description |
|-------|-----------|-------------|
| `swap_path` | `PathBuf` | Absolute path of the swap file on disk |
| `meta` | `SwapMeta` | Cached metadata (pid, hostname, timestamps) for refreshed writes |

`SwapHandle` is `None` on `BufferState` when:
- The buffer has no file path (unnamed buffer).
- The buffer's file path matches a configured exclusion pattern.
- Swap-file creation failed (non-fatal; editor continues without swap for that buffer).

---

## State Transitions

```text
 ┌─────────────────────────────────────────────────────────────────────────┐
 │  File open (path matches exclusion pattern or unnamed buffer)           │
 │  → No SwapHandle created; buffer proceeds without swap protection       │
 └─────────────────────────────────────────────────────────────────────────┘

 ┌─────────────────────────────────────────────────────────────────────────┐
 │  File open (normal text file, path not excluded)                        │
 │  1. Compute swap_path from absolute source path                         │
 │  2. Check if swap_path exists on disk                                   │
 │     a. Exists → read SwapMeta; show recovery prompt (FR-003)            │
 │        - User accepts → restore content from swap; keep SwapHandle      │
 │        - User rejects → delete swap file; create fresh SwapHandle       │
 │     b. Does not exist → create swap file; store SwapHandle on buffer    │
 └──────────────────────────────┬──────────────────────────────────────────┘
                                │ SwapHandle present
                                ▼
 ┌─────────────────────────────────────────────────────────────────────────┐
 │  Buffer modified (any edit)                                             │
 │  → Refresh swap: write temp file, sync_all, rename to swap_path         │
 │    (last_refreshed_at updated in header)                                │
 └──────────────────────────────┬──────────────────────────────────────────┘
                                │
                                ▼
 ┌─────────────────────────────────────────────────────────────────────────┐
 │  User initiates save (`:w`, Space+w, or `:wq`)                          │
 │  1. Write buffer content to <target>.ordex_tmp                          │
 │  2. file.sync_all()              ← durable-write confirmation point     │
 │  3. fs::rename(tmp, target)      ← atomic replacement                   │
 │  4. On success: delete swap_path ← FR-006                               │
 │     On failure: leave swap_path intact ← FR-007                         │
 └──────────────────────────────┬──────────────────────────────────────────┘
                                │ save successful
                                ▼
 ┌─────────────────────────────────────────────────────────────────────────┐
 │  SwapHandle removed from BufferState                                    │
 │  Buffer marked clean (saved_undo_depth updated)                         │
 └─────────────────────────────────────────────────────────────────────────┘

 ┌─────────────────────────────────────────────────────────────────────────┐
 │  Process exits unexpectedly (crash, kill, power loss)                   │
 │  → swap_path remains on disk → available for recovery next open         │
 └─────────────────────────────────────────────────────────────────────────┘
```

---

## Config Schema Addition

The existing `[swap]` section (new) is added to the ordex config file format:

```toml
[swap]
# Glob patterns matched against the full absolute file path.
# Files whose paths match any pattern are excluded from swap protection.
# * matches any sequence of characters including path separators.
exclude = ["/dev/shm/gopass*", "*.gpg"]
```

`ConfigSettings` gains one new field:

```rust
/// Glob patterns (matched against full absolute path) that suppress swap-file creation.
pub(crate) swap_exclude_patterns: Vec<String>,
```

Validation: each entry must be a non-empty string; an empty string pattern is ignored with a
startup warning. No other validation (patterns are matched at runtime).

---

## Module Layout and Rust Type Sketch

```rust
// src/swap/mod.rs  ────────────────────────────────────────────────────────

/// Swap-file handle attached to one open buffer while unsaved changes exist.
pub(crate) struct SwapHandle {
    /// Absolute path of the swap file on disk.
    pub(crate) swap_path: PathBuf,
    /// Cached metadata written into the swap file header.
    meta: SwapMeta,
}

impl SwapHandle {
    /// Create a new swap file for `source_path` and return a handle.
    pub(crate) fn create(source_path: &Path) -> io::Result<Self> { … }

    /// Rewrite the swap file with the current buffer content.
    pub(crate) fn refresh(&mut self, buffer: &TextBuffer) -> io::Result<()> { … }

    /// Delete the swap file from disk; consumes the handle.
    pub(crate) fn delete(self) -> io::Result<()> { … }

    /// Return the absolute path of the swap file (for recovery prompts).
    pub(crate) fn swap_path(&self) -> &Path { … }
}

// src/swap/format.rs  ─────────────────────────────────────────────────────

/// Metadata stored in the ordex-swap-v1 header.
pub(crate) struct SwapMeta {
    pub(crate) pid: u32,
    pub(crate) hostname: String,
    pub(crate) original_path: PathBuf,
    pub(crate) opened_at: u64,
    pub(crate) last_refreshed_at: u64,
}

impl SwapMeta {
    /// Write the header block to a writer; does not write the content body.
    pub(crate) fn write_header<W: Write>(&self, writer: &mut W) -> io::Result<()> { … }

    /// Read and parse the header block from a reader; leaves the reader positioned
    /// at the first byte of the content body (after the blank delimiter line).
    pub(crate) fn read_header<R: BufRead>(reader: &mut R) -> io::Result<Self> { … }
}

// src/swap/location.rs  ───────────────────────────────────────────────────

/// Return the swap file path for `source_path` inside `swap_dir`.
pub(crate) fn swap_path_for(source_path: &Path, swap_dir: &Path) -> PathBuf { … }

/// Encode an absolute path into a flat filename component (%-encoding).
pub(crate) fn encode_path(path: &Path) -> String { … }

/// Resolve the default swap-storage directory from XDG environment variables.
pub(crate) fn default_swap_dir() -> io::Result<PathBuf> { … }

// src/swap/glob.rs  ───────────────────────────────────────────────────────

/// Return true if `path` matches `pattern`, where `*` matches any characters
/// including `/`.
pub(crate) fn matches(pattern: &str, path: &str) -> bool { … }

/// Return true if `path` matches any pattern in `patterns`.
pub(crate) fn matches_any(patterns: &[String], path: &str) -> bool { … }
```

---

## Edge Cases from the Spec

| Edge Case | Handling |
|-----------|----------|
| Exclusion pattern added after swap exists | Existing swap stays until session ends or durable save; new swap creation is suppressed once pattern is applied at buffer-open time. Patterns are only evaluated when a buffer is opened, not retroactively. |
| Files that are not normal text files | Out of scope for this feature (FR-012); no swap created. |
| Swap exists but main file was also modified by another process | Recovery prompt shows that swap data exists; user decides. Ordex does not silently discard either version. |
| Save succeeds functionally but `sync_all` not confirmed | Swap remains until `sync_all` returns `Ok(())`. |
| Multiple saves in one editing session | Swap is refreshed on each edit; deleted only after the first durable confirmed save that leaves the buffer clean. |
| Pattern matching for paths with multiple dots or no extension | `*` matches any characters; `*.gpg` correctly matches `/a/b.c.gpg` and does not match `/a/b.gpg.bak`. |
