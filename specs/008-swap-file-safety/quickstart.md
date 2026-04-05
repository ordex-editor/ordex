# Quickstart: Swap File Safety

**Feature**: `008-swap-file-safety` | **Branch**: `008-swap-file-safety`

This document describes how ordex's swap-file safety feature works and how to configure it.
See `docs/src/configuration.md` for the full configuration reference.

---

## What Are Swap Files?

Ordex creates a **swap file** for each normal text file you open. The swap file stores a copy
of your in-progress edits so that if ordex exits unexpectedly — due to a crash, a lost SSH
connection, or a power failure — your unsaved work is not lost.

Swap files are kept in ordex's cache directory (usually `~/.cache/ordex/swap/`) and are
removed automatically once your file is saved durably to disk.

---

## How It Works

### When you open a file

Ordex checks whether a swap file already exists for that path.

- **No swap file found**: ordex creates a swap file and begins protecting your edits.
- **Swap file found**: ordex shows a recovery prompt before opening the file normally.

### While you edit

The swap file is refreshed each time you make a change. Your most recent unsaved edits are
always available for recovery.

### When you save

Ordex writes your file to disk, calls `fsync` to confirm durability, then removes the swap
file. The swap file is only deleted **after** the save is confirmed. If saving fails for any
reason, the swap file is kept.

### If ordex exits unexpectedly

The swap file remains on disk. The next time you open the same file, ordex detects the swap
and offers to restore your interrupted work.

---

## Recovery Prompt

When ordex finds a swap file for a file you are opening, it will display a message such as:

```
Recovery data exists for this file from a previous session.
  [r] Restore unsaved work    [d] Discard recovery data and open from disk
```

- Press **r** to restore: ordex loads the content from the swap file.
- Press **d** to discard: ordex deletes the swap file and opens the file from disk as normal.

If you are unsure, always inspect the recovered content first; you can discard it afterwards
with `:q!` or continue editing and saving.

---

## Excluding Files From Swap Protection

Some files — such as password manager temporaries or encrypted files — should not have swap
files. Use the `[swap]` section in your ordex configuration file to specify exclusion patterns:

```toml
[swap]
exclude = ["/dev/shm/gopass*", "*.gpg"]
```

**Pattern rules:**

- Each pattern is matched against the **full absolute path** of the file.
- `*` matches any sequence of characters, **including path separators** (`/`).
- A file is excluded if it matches **any** pattern in the list.

**Examples:**

| Pattern | Paths excluded |
|---------|---------------|
| `*.gpg` | `/home/alice/secrets.gpg`, `/tmp/keyring.gpg` |
| `/dev/shm/gopass*` | `/dev/shm/gopass_edit123`, `/dev/shm/gopass_edit123/file.txt` |
| `/tmp/scratch*` | `/tmp/scratch`, `/tmp/scratch_work/notes.md` |

Exclusion patterns are evaluated when a file is opened. If you add a pattern while a file is
already open with an active swap, the existing swap remains available until the editing session
ends or a durable save completes.

---

## Swap File Location

Swap files are stored in `$XDG_CACHE_HOME/ordex/swap/` (usually `~/.cache/ordex/swap/`).
You can inspect or remove swap files manually if needed, though ordex handles this
automatically under normal operation.

Each swap file's name encodes the absolute path of the original file. For example, the swap
file for `/home/alice/notes.txt` is named `%2Fhome%2Falice%2Fnotes.txt.swp`.

---

## Troubleshooting

**I see a recovery prompt for a file I already saved in another editor.**
The swap file was left behind because ordex did not perform the durable save. Choose **d** to
discard the recovery data and open the file as it is on disk.

**A stale swap file reappears after I delete it.**
If ordex is still running and has that file open, it will refresh the swap on each edit. Close
the buffer in ordex (`:bd`) or save the file (`:w`) to remove the swap normally.

**I want to disable swap files entirely.**
You cannot disable the feature globally in this release, but you can use a broad exclusion
pattern — for example, `/*` — to suppress swap creation for all files. Note that this removes
crash-recovery protection for all your open files.

**I used `:q!`, so why is there no recovery prompt next time?**
Intentional quit/discard flows remove the session's swap files on exit. Recovery prompts are
reserved for interrupted sessions where ordex did not shut down cleanly.

---

## See Also

- `docs/src/configuration.md` — full reference for the `[swap]` configuration section
- `docs/src/file-operations.md` — file saving workflow and durable-write guarantees
