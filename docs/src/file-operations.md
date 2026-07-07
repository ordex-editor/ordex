# File Operations

## Open Existing Files

```bash
ordex path/to/file.txt
```

Ordex reads the file and displays it in the editor.

To open multiple files at startup, pass each path as a separate argument:

```bash
ordex first.txt second.txt third.txt
```

Use `:bn` and `:bp` to move between open buffers after startup. A persistent
top-row tab strip also lists open buffers in order and highlights the active
buffer.

Normal mode also supports `gf` to open the filename-like token under the
cursor, `gF` to honor trailing `:line[:column]` suffixes, and `ga` to jump to
the most recently visited named buffer that is still open.

Press `<Space>f` in Normal mode to open a recursive file picker rooted at the
current working directory. The picker scans asynchronously and streams rows as
results arrive.

Inclusion and exclusion rules for file-picker rows:

- Hidden paths are included.
- In Git work trees, Ordex walks the worktree directly with `.gitignore` and
  `.ignore` evaluation so `.ignore` negations can re-include Git-ignored paths.
- `.ignore` rules are evaluated after `.gitignore` rules and can re-include
  paths excluded by `.gitignore`.
- Parent-directory exclusions (for example `target/`) continue to apply inside
  re-included subtrees unless a later rule explicitly un-ignores them.
- For Git scans, ignore files are read from the current Git worktree root down
  to the scanned path, so ignore files outside the worktree do not affect
  results.

Type any fuzzy subsequence to filter by basename or relative path, press
`Enter` to open the selected file, or press `Esc` to cancel.
The picker query row shows `filtered/total` fuzzy-match counts on the right and
keeps the spinner prefix while scanning or deferred filtering is still in
flight.

On wide terminals, the file picker also shows a right-side content preview for
the selected file with syntax highlighting. If the selected file is already open
with unsaved edits, the preview uses the live in-memory buffer instead of the
on-disk version. Narrow terminals fall back to the picker-only layout.

Fuzzy picker filtering splits the query on whitespace. Positive terms use
case-insensitive subsequence matching, so `cfg rs` can match
`src/config_reader.rs` even though the letters are not adjacent. Prefix a term
with `!` to exclude entries whose basename or relative path contains that exact
substring, such as `rs !test` to keep Rust files while hiding paths with
`test`. A bare `!` is ignored.

## Search File Contents

Run `:grep {pattern}` or `:gr {pattern}` from Normal mode to search file
contents under the current working directory. The pattern is treated as a regex.
`<Space>/` opens command mode prefilled with `:grep ` as a shortcut, and
`<Space>*` immediately searches for whole-word matches of the identifier under
the cursor, or of the next same-line identifier when the cursor is on
whitespace or punctuation.

Ordex opens the search-results picker immediately, continues streaming new
matches in the background, and lets you fuzzy-filter those results with the
picker query while the search is still running.
The query row shows `filtered/total` fuzzy-match counts on the right and keeps
the spinner prefix while search or deferred filtering work is still active.

Each picker row represents one matching line in the form
`path:line:column: preview`. Press `Enter` to open the selected file and jump to
the first match on that line with the destination centered in the viewport, or
press `Esc` to cancel the picker.

Ordex prefers `rg` when it is available on `PATH` and falls back to `grep`
otherwise. By default the search skips hidden and ignored files.

## Start a New File

```bash
ordex new-file.txt
```

If the file does not exist, Ordex opens an empty buffer associated with that path.

Save with `:w` to create the file on disk.

## Save and Overwrite Behavior

| Command | Description | Notes |
| --- | --- | --- |
| `:w` | Write the current file. | Stays in the editor. |
| `:wall` / `:wa` | Write every modified named buffer. | Returns to the originally active buffer after the save-all sequence finishes. |
| `:wq` | Write the current file and quit. | Quits only after a successful save. |
| `:x` | Write the current file only when modified, then quit. | Acts like `:wq` for dirty buffers and `:q` for clean ones. |
| `:w <path>` / `:write <path>` | Write to a new path and make it the current file. | Uses the supplied destination path. |
| `:w!`, `:wq!`, `:w! <path>` | Bypass overwrite confirmation. | Forces the write flow for the current path or the supplied destination. |

If the destination already exists **and is different from the current buffer
file path**, Ordex asks for confirmation:
`Overwrite "<path>"? [y/N]`

Press `y` or `Y` to confirm. Any other key cancels the write.

If the current file changed on disk since Ordex last loaded or saved it, `:w`
asks before overwriting those external edits:
`"<path>" changed on disk. Overwrite anyway? [y/N]`

This save-time confirmation still appears even if you previously ignored the
reload alert for that buffer.

Successful writes go through a sibling temp file, `fsync`, and atomic rename.
Ordex also ensures the saved file ends with a trailing newline.
Ordex keeps the corresponding swap file until the owning instance exits so other
Ordex instances can still warn that the file is already open.

If the durable save does not complete, Ordex keeps the swap file so recovery
data is still available on the next open.

When another Ordex instance already owns the swap file, Ordex warns before
opening the buffer and offers read-only, edit-anyway, recover, discard, and
cancel choices. The read-only choice still allows edits in memory, but Ordex
asks again before writing back to that same file.

For unnamed buffers, an additional `[i] ignore` option appears when a swap file is
found. This option leaves the orphaned swap file on disk and opens a fresh empty
buffer, preserving the ability to recover the swap later if needed. This is
useful when you opened multiple instances of ordex with unnamed buffers and want
to keep the recovery option available for future use.

## External File Changes

Ordex tracks changes to named file-backed buffers after they are opened or
saved.

- Clean active buffers auto-reload by default and report:
  `"<path>" reloaded after external change`
- Clean hidden buffers also auto-reload by default, but Ordex waits to show that
  message until you activate the buffer again
- Dirty buffers show a prompt instead of reloading automatically:
  `"<name>" changed on disk. Reload from disk and discard changes? [r]eload/[i]gnore`
- Clean buffers show the same prompt when
  `[editor].auto_reload_external_changes = false`, using `Reload from disk`
  without the discard warning

Choosing `i` keeps the in-memory buffer and suppresses repeat alerts until the
file changes again on disk.

## Buffer Commands

| Command | Description |
| --- | --- |
| `:e <path>` / `:edit <path>` | Open another buffer and switch to it. |
| `:e` / `:edit` | Reload the active file-backed buffer from disk. |
| `:e!` / `:edit!` | Reload from disk and discard unsaved edits without the save/discard/cancel prompt. |
| `:new` | Open a new unnamed buffer and switch to it. |
| `:bn` / `:buffer-next` | Switch to the next open buffer. |
| `:bp` / `:buffer-prev` | Switch to the previous open buffer. |
| `:ls` / `:buffers` | List open buffers on the message line. |
| `:bd` / `:buffer-delete` | Close the active buffer. |

If `:bd` targets a dirty buffer, Ordex asks whether to save it before closing.
If `:e` targets a dirty named buffer, Ordex asks whether to save before reload.
If `:e` or `:e!` targets an unnamed buffer, Ordex reports `No file name`.
If `:wall` encounters a dirty unnamed buffer, it stops before saving any buffer and reports `No file name`.

The tab strip remains visible even with one open buffer.

## Project Sessions

| Command | Description | Notes |
| --- | --- | --- |
| `:save-session <name>` / `:ss <name>` | Store the current working directory, open buffers, and each buffer cursor position in `~/.cache/ordex/sessions/<name>.toml`. | Also makes that session the active quit-time autosave target. |
| `:open-session <name>` / `:os <name>` | Restore the saved working directory first, then reopen the saved buffers in their original order and activate the saved current buffer. | Afterward, quitting Ordex autosaves back into the same session file. |
| `:delete-session <name>` / `:ds <name>` | Remove `~/.cache/ordex/sessions/<name>.toml`. | Deletes the named saved session file. |

Session files use the same TOML-like text format as Ordex config files and are
intended to be reopened by name rather than edited by hand.

If the current editor state contains unsaved buffers, Ordex asks for each dirty
buffer before replacing the session:
`Save changes to "<name>" before opening session "<session>"? [y]es/[n]o/[c]ancel`

Missing files from a saved session reopen as named empty buffers at their saved
paths.

Recoverable session-load warnings do not block reopening. Ordex reports the
warning count on the message line.

## Quit with Unsaved Changes

| Command | Description | Notes |
| --- | --- | --- |
| `:q` | Quit immediately when there are no unsaved changes. | If buffers are dirty, Ordex asks about each one in turn. |
| `:q!` | Quit immediately without saving. | Still follows the discard path cleanly. |

If there are unsaved changes, Ordex asks:
`Save changes to "<name>"? [y]es/[n]o/[c]ancel`

Press `y` or `Y` to save and quit.
Press `n` or `N` to discard the current dirty buffer and continue quitting.
Press `c` or `C`, or any other key, to cancel and stay in the editor.

Graceful quits delete any remaining swap files for the session, including
deliberate discard flows such as `:q!`.

For unnamed buffers, choosing `y` shows `No file name` and keeps the editor
open.

## Modified Indicator

The status bar shows `[+]` when the active buffer has unsaved changes and a
colored `🔒` marker when the active file is read-only.

The tab strip prefers buffer basenames when terminal width is tight and may drop
tab modified markers before shortening labels.
