# Research: Swap File Safety

**Branch**: `008-swap-file-safety` | **Date**: 2026-04-05
**Status**: Complete — all NEEDS CLARIFICATION items resolved

---

## 1. Swap File Format

### Decision

Use a **custom line-oriented text format** named `ordex-swap-v1`. Each swap file consists of a
fixed header block (magic line + key=value pairs) terminated by a blank line, followed by the
raw UTF-8 content of the buffer. Files are stored in `$XDG_CACHE_HOME/ordex/swap/`.

**Header structure:**

```
ordex-swap-v1
pid=<decimal process id>
hostname=<hostname string>
original_path=<absolute path of the source file>
opened_at=<unix timestamp seconds>
last_refreshed_at=<unix timestamp seconds>

<raw UTF-8 buffer content, to end of file>
```

The blank line between the last header key and the content acts as the delimiter. Parsers read
lines until they encounter the blank line, then treat the remainder as raw content.

### Rationale

- **No extra dependencies.** All parsing is line-by-line over a `BufReader`; the header is
  simpler than the existing TOML-like session format and requires no additional parser.
- **Unambiguous magic line.** `ordex-swap-v1` at byte 0 lets any future ordex version detect
  the format version before reading further fields.
- **Includes PID and hostname.** These two fields are needed for future multi-instance
  duplicate-open detection (see section 5) without requiring a format version bump.
- **Single file per buffer.** Avoids the atomicity complexity of a two-file approach; the
  write strategy (temp file + `sync_all` + `rename`) already provides atomic replacement.
- **Human-readable.** The metadata is inspectable with any text editor, which aids debugging
  and user trust.

### Alternatives Considered

| Alternative | Reason Rejected |
|-------------|-----------------|
| Vim's binary block format (1024-byte blocks) | Over-engineered; no user-visible benefit at ordex's scale; significant implementation effort |
| TOML-like format with escaped content body | Escaping arbitrary file bytes in a TOML string value is fragile and hard to round-trip cleanly |
| Two-file approach (`.swp` metadata + `.swp.content` raw) | Doubles file-system operations; hard to make atomic; complicates cleanup |
| Hash-named files with SHA-256 | Requires a hash function not in std; path encoding achieves the same uniqueness without a dependency |

---

## 2. Swap File Location and Naming

### Decision

- **Directory**: `$XDG_CACHE_HOME/ordex/swap/`, falling back to `$HOME/.cache/ordex/swap/`
  when `XDG_CACHE_HOME` is unset or empty. This mirrors the existing session storage logic in
  `src/session.rs` (`default_sessions_dir`).
- **Filename encoding**: take the absolute path, replace `%` with `%%` then `/` with `%2F`,
  append `.swp`. Example: `/home/alice/notes.txt` → `%2Fhome%2Falice%2Fnotes.txt.swp`.
- **Decoding**: replace `%2F` with `/` and `%%` with `%` (order matters).

### Rationale

- Reuses the XDG cache convention already established in ordex; no new location logic needed
  beyond a new sub-directory name.
- Path encoding is reversible and transparent: the swap filename directly encodes the source
  path, making it possible to enumerate all swap files and map each back to its source without
  reading the file header.
- The `.swp` suffix is immediately recognizable to experienced terminal users and vim users.

### Alternatives Considered

| Alternative | Reason Rejected |
|-------------|-----------------|
| Same-directory `.file.swp` (vim default) | Pollutes the user's project directories with hidden files; undesirable for a configurable editor |
| Content-addressed (SHA-256 of path) | Requires implementing or depending on a hash function; path encoding is simpler |
| Random UUID filenames | Non-reversible; requires reading every swap file to find the one for a given path |

---

## 3. Durable Save Confirmation

### Decision

Replace the current `File::create` → `write_buffer_to` → `complete_deferred_write` pattern in
`app::execute_deferred_write` with a four-step durable-write sequence:

1. Write buffer content to `<target_path>.ordex_tmp` (sibling temp file, same directory as
   target to ensure same filesystem for atomic rename).
2. Call `file.sync_all()` on the temp file (maps to `fsync(2)`; flushes data and metadata to
   persistent storage).
3. Call `fs::rename(temp_path, target_path)` for atomic replacement.
4. After the rename succeeds: delete the swap file for this buffer.

If any step fails, the swap file is **not** deleted. The temp file is removed on failure to
avoid litter.

### Rationale

- `sync_all()` is available in `std::fs::File` with no extra dependencies; it guarantees
  durable persistence before the swap is removed, satisfying FR-005 and FR-007.
- Atomic rename prevents a half-written target from being visible to the OS; if the process is
  killed between the write and the rename, the target is unchanged and the swap file remains.
- Deleting the swap only after a confirmed rename satisfies FR-006 exactly: the swap persists
  until durability is confirmed.
- The `.ordex_tmp` suffix is unlikely to conflict with any user file and clearly identifies
  ordex's in-progress writes.

### Alternatives Considered

| Alternative | Reason Rejected |
|-------------|-----------------|
| `sync_data()` instead of `sync_all()` | `sync_data()` skips metadata (size, mtime); safe on most filesystems but `sync_all()` is the safer default for a safety-critical save path |
| Direct write to target (current approach) | A crash mid-write leaves a corrupted target; atomic rename eliminates this risk |
| `libc::fsync` directly | `File::sync_all()` wraps `fsync` in safe Rust; no need to drop into unsafe |

---

## 4. Exclusion Glob Matching

### Decision

Implement a **simple `*`-wildcard matcher** where `*` matches any sequence of characters,
**including path separators**. Matching is applied to the absolute string representation of the
file path. A path is excluded if it matches **any** pattern in `swap.exclude`.

Algorithm (iterative, O(n·m)):
1. Split the pattern on `*` to obtain literal segments.
2. Verify the path starts with the first segment and ends with the last segment.
3. Scan the middle segments left-to-right, advancing a cursor through the path string.

This is expressible in under 30 lines of std Rust with no unsafe code.

**Examples from the spec:**

| Pattern | Path | Matches? | Explanation |
|---------|------|----------|-------------|
| `*.gpg` | `/tmp/secret.gpg` | ✅ | `*` matches `/tmp/secret` |
| `*.gpg` | `/tmp/notes.txt` | ❌ | Path does not end with `.gpg` |
| `/dev/shm/gopass*` | `/dev/shm/gopass_edit123` | ✅ | `*` matches `_edit123` |
| `/dev/shm/gopass*` | `/dev/shm/gopass_edit123/file` | ✅ | `*` matches `_edit123/file` |
| `/dev/shm/gopass*` | `/tmp/notes.txt` | ❌ | Path does not start with `/dev/shm/gopass` |

This behaviour directly satisfies FR-009 and FR-010.

### Rationale

- **No dependency needed.** A `*`-wildcard matcher is short enough to implement and test
  within the project's existing budget.
- **`*` crosses path separators** intentionally, matching the spec's requirement that
  `/dev/shm/gopass*` covers descendant paths (FR-010). Standard shell globbing (`*` stays
  within one segment) would require `**` and would contradict the spec's examples.
- The pattern set is expected to be small (single digits), so algorithmic efficiency is
  irrelevant; correctness and simplicity are the priorities.

### Alternatives Considered

| Alternative | Reason Rejected |
|-------------|-----------------|
| `glob` crate | Would consume one of the zero remaining dependency slots; not justified for a matcher needing < 30 lines |
| Standard shell glob semantics (`*` stops at `/`) | Contradicts FR-010; `/dev/shm/gopass*` would not match `/dev/shm/gopass_edit123/file` |
| Regex | Over-engineered; the spec examples don't require regex; adds implementation surface |
| `.gitignore`-style patterns (`**`) | Richer than needed; `*` crossing separators is sufficient |

---

## 5. Future Multi-Instance Duplicate-Open Warning

### Analysis

Vim's swap mechanism doubles as a duplicate-open detector: when Vim opens a file, it looks for
an existing `.swp` file, reads the PID and hostname from it, and warns the user if that PID is
still alive on the same host (via `kill(pid, 0)` returning 0 without `ESRCH`).

**Is this the right mechanism for ordex?** Yes, for the following reasons:

1. **Swap files are already per-file and per-instance.** Each swap file's `original_path` field
   uniquely identifies the source file, and the `pid` field identifies the owning process. No
   additional coordination file is needed.
2. **`kill(pid, 0)` is available via `libc`**, which is already a runtime dependency. Checking
   process liveness requires zero new dependencies.
3. **Cross-host editing** (e.g., the same file mounted over NFS) can be detected weakly:
   if `hostname` differs from the current machine's hostname, ordex can warn that a swap from
   a different host exists, without claiming the other instance is necessarily still running.
4. **The proposed `ordex-swap-v1` format already includes all fields required** (`pid`,
   `hostname`, `original_path`). Implementing the multi-instance warning in a future version
   requires only new logic in the file-open path, not a format change.

### What is **out of scope** for the current feature

- Checking whether the PID in an existing swap is alive (multi-instance liveness check).
- Warning the user that another ordex instance has the file open.
- Locking the file to prevent concurrent edits.

These are deferred to a future feature. The current feature only uses swap files for
**crash-recovery detection** (was the previous session interrupted?), not for liveness
detection.

### Recommendation

Adopt swap files as the future mechanism for multi-instance warnings. The format is designed to
support this without amendment.

---

## 6. "Normal Text File" Scope

### Decision

For the current feature, a file is considered a **normal text file** if ordex successfully
loads it into a `TextBuffer` via `TextBuffer::from_reader`. No additional file-type gating is
added in this feature; the spec already restricts swap-file scope to files that ordex treats as
ordinary editable text (FR-012).

Binary detection and per-format gating are deferred to future work. In practice, ordex today
only opens files it can read as text; structured formats (if any) that cannot be represented as
a `TextBuffer` are already outside the editor's current scope.

---

## Summary of Decisions

| Topic | Decision |
|-------|----------|
| Swap file format | `ordex-swap-v1` line-oriented text: key=value header + blank line + raw UTF-8 content |
| Swap file location | `$XDG_CACHE_HOME/ordex/swap/` (XDG cache, mirrors session storage) |
| Swap file naming | Path-encoded filename: `%2F`-encoded absolute path + `.swp` suffix |
| Durable save | Write to `.ordex_tmp` → `sync_all()` → atomic `rename` → delete swap |
| Glob matching | `*`-wildcard crossing path separators; implemented in std Rust, no dep |
| Multi-instance future | Swap files are the right future mechanism; format pre-includes pid+hostname |
| Normal-text scope | Files loaded into `TextBuffer`; no additional gating in this feature |
