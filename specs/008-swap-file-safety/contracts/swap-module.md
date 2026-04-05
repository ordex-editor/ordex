# Internal Module Contract: `swap`

**Branch**: `008-swap-file-safety` | **Date**: 2026-04-05
**Module**: `src/swap/` — swap-file creation, refresh, deletion, and recovery detection

This document specifies the public-to-crate interface of the `swap` module: the types,
functions, and invariants that the rest of ordex depends on. Implementers must satisfy every
invariant listed here. Callers must not rely on any behaviour not specified here.

---

## Modules

| Module | Responsibility |
|--------|---------------|
| `swap::mod` | `SwapHandle` type; orchestration of create / refresh / delete |
| `swap::format` | `SwapMeta` type; `ordex-swap-v1` header serialization and parsing |
| `swap::location` | Swap directory resolution; path-to-filename encoding |
| `swap::glob` | `*`-wildcard pattern matching against file paths |

All items are `pub(crate)` unless stated otherwise.

---

## `swap::SwapHandle`

### Type

```rust
pub(crate) struct SwapHandle {
    pub(crate) swap_path: PathBuf,
    meta: SwapMeta,
}
```

### Invariants

- `swap_path` is an absolute path. It always ends with `.swp`.
- A `SwapHandle` value guarantees that the corresponding swap file **was successfully written**
  at least once (at creation time). The file may have been deleted externally; `delete` must
  tolerate a missing file with a best-effort attempt.
- At most one `SwapHandle` exists per `swap_path` within one ordex process.

### Functions

#### `SwapHandle::create(source_path: &Path) -> io::Result<Self>`

**Pre-conditions:**
- `source_path` is an absolute path to a file that has been successfully loaded into a buffer.
- No exclusion pattern has matched `source_path` (the caller is responsible for checking).

**Post-conditions:**
- A valid `ordex-swap-v1` file exists at the returned `swap_path` containing the buffer
  content at time of call.
- The swap file is written atomically: written to a temp path, `sync_all`-ed, then renamed.
- Returns `Err` if the swap directory cannot be created, the temp write fails, `sync_all`
  fails, or the rename fails. On error, any temp file created is removed (best-effort).

**Side effects:**
- Creates `$XDG_CACHE_HOME/ordex/swap/` if it does not exist.
- Writes one `.swp` file to that directory.

---

#### `SwapHandle::refresh(&mut self, buffer: &TextBuffer) -> io::Result<()>`

**Pre-conditions:**
- `self.swap_path` was a valid swap path returned by `create`.
- `buffer` is the current in-memory state of the buffer for which the swap was created.

**Post-conditions:**
- The swap file at `self.swap_path` contains the current buffer content with an updated
  `last_refreshed_at` timestamp.
- Write is atomic (temp + `sync_all` + rename); partial writes are not visible.
- `self.meta.last_refreshed_at` is updated to the timestamp written.

**Side effects:**
- Overwrites the swap file.

---

#### `SwapHandle::delete(self) -> io::Result<()>`

**Pre-conditions:** None (may be called even if the file no longer exists).

**Post-conditions:**
- The file at `self.swap_path` no longer exists, or never existed.
- Returns `Ok(())` if the file was successfully deleted or did not exist (`ENOENT` is treated
  as success).
- Returns `Err` only for unexpected I/O errors (e.g., permission denied).

**Side effects:**
- Removes one file from disk.

---

#### `SwapHandle::swap_path(&self) -> &Path`

Returns `self.swap_path`. No side effects.

---

## `swap::format::SwapMeta`

### Type

```rust
pub(crate) struct SwapMeta {
    pub(crate) pid: u32,
    pub(crate) hostname: String,
    pub(crate) original_path: PathBuf,
    pub(crate) opened_at: u64,
    pub(crate) last_refreshed_at: u64,
}
```

### Wire Format (ordex-swap-v1)

```
ordex-swap-v1\n
pid=<decimal u32>\n
hostname=<UTF-8 string, no newlines>\n
original_path=<absolute UTF-8 path, no newlines>\n
opened_at=<decimal u64>\n
last_refreshed_at=<decimal u64>\n
\n
<raw UTF-8 buffer content to EOF>
```

- Lines use `\n` only (no `\r\n`).
- The blank line after `last_refreshed_at` is the header/content delimiter.
- Key order in the header is fixed as shown above; parsers must accept keys in any order for
  forward compatibility.
- Unknown keys in the header are silently ignored (forward compatibility).

### Functions

#### `SwapMeta::write_header<W: Write>(&self, writer: &mut W) -> io::Result<()>`

Writes the six header lines and the trailing blank delimiter line to `writer`. Does not write
any content body.

**Invariants:**
- Output is valid UTF-8.
- `opened_at` ≤ `last_refreshed_at` is enforced by callers; `write_header` does not validate.

---

#### `SwapMeta::read_header<R: BufRead>(reader: &mut R) -> io::Result<Self>`

Reads and parses the header block. Leaves `reader` positioned at the first byte of the content
body (the byte after the blank delimiter line).

**Error conditions (all return `io::Error` with `InvalidData` kind):**
- First line is not exactly `ordex-swap-v1`.
- A required key (`pid`, `hostname`, `original_path`, `opened_at`, `last_refreshed_at`) is
  missing after exhausting the header block.
- `pid` or timestamp values are not valid decimal integers.
- `original_path` is not an absolute path.
- EOF encountered before the blank delimiter line.

---

## `swap::location`

### Functions

#### `default_swap_dir() -> io::Result<PathBuf>`

Returns `$XDG_CACHE_HOME/ordex/swap` if `XDG_CACHE_HOME` is set and non-empty, otherwise
`$HOME/.cache/ordex/swap`. Returns `Err` if neither environment variable yields a usable path.

Mirrors the logic of `session::default_sessions_dir` but targets the `swap` subdirectory.

---

#### `swap_path_for(source_path: &Path, swap_dir: &Path) -> PathBuf`

Returns `swap_dir.join(encode_path(source_path))` with `.swp` appended.

**Pre-condition:** `source_path` is absolute.

---

#### `encode_path(path: &Path) -> String`

Converts an absolute path to a flat filename component:
1. Replace every `%` in the UTF-8 path string with `%%`.
2. Replace every `/` with `%2F`.

**Invariant:** The encoding is injective (two distinct absolute paths yield two distinct
encoded strings).

---

## `swap::glob`

### Functions

#### `matches(pattern: &str, path: &str) -> bool`

Returns `true` if `path` matches `pattern` under the following semantics:
- `*` matches any sequence of characters, including `/` and the empty string.
- All other characters in `pattern` are matched literally (case-sensitive).
- The match is anchored: `pattern` must match the entire `path` string.

**Examples:**

| `pattern` | `path` | Result |
|-----------|--------|--------|
| `*.gpg` | `/tmp/secret.gpg` | `true` |
| `*.gpg` | `/tmp/secret.gpg.bak` | `false` |
| `/dev/shm/gopass*` | `/dev/shm/gopass_edit123` | `true` |
| `/dev/shm/gopass*` | `/dev/shm/gopass_edit123/file` | `true` |
| `/dev/shm/gopass*` | `/dev/shm/other` | `false` |
| `/tmp/notes.txt` | `/tmp/notes.txt` | `true` |
| `/tmp/notes.txt` | `/tmp/notes.txt.bak` | `false` |

---

#### `matches_any(patterns: &[String], path: &str) -> bool`

Returns `true` if `matches(p, path)` is `true` for any `p` in `patterns`.
Returns `false` for an empty `patterns` slice.

---

## Integration Points (callers)

| Caller | Contract used | Purpose |
|--------|---------------|---------|
| `app::execute_deferred_write` | `SwapHandle::delete` | Remove swap after durable confirmed save |
| `editor_state::buffers::BufferState` | `SwapHandle` field | Store handle while buffer has unsaved changes |
| File-open path (in `editor_state`) | `SwapHandle::create` | Create swap when opening a normal text buffer |
| Edit path (in `editor_state`) | `SwapHandle::refresh` | Refresh swap on each modification |
| Config validator | `swap_exclude_patterns: Vec<String>` | Expose exclusion patterns to match at open time |
| Recovery prompt (in `editor_state`) | `swap::format::SwapMeta::read_header` | Read stale swap for recovery UI |

---

## Error Handling

- All swap I/O errors are **non-fatal**: if swap creation or refresh fails, ordex logs a status
  message and continues without swap protection for that buffer session.
- Save failures that prevent `sync_all` or `rename` **do not delete** the swap file; the
  existing save-error reporting path in `editor_state` handles user messaging.
- `SwapHandle::delete` failure is logged but not propagated to the user (the important save
  already succeeded; a stale swap on next open triggers the normal recovery prompt).
