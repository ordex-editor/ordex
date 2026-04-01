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

Use `:bn` and `:bp` to move between open buffers after startup.

Press `<Space>f` in Normal mode to open a recursive file picker rooted at the
current working directory. The picker scans the disk asynchronously, streams
matches while it walks the tree, includes hidden paths, and respects
`.gitignore` when Ordex is running inside a Git work tree.

Type any fuzzy subsequence to filter by basename or relative path, press
`Enter` to open the selected file, or press `Esc` to cancel.

Fuzzy picker filtering splits the query on whitespace. Positive terms use
case-insensitive subsequence matching, so `cfg rs` can match
`src/config_reader.rs` even though the letters are not adjacent. Prefix a term
with `!` to exclude entries whose basename or relative path contains that exact
substring, such as `rs !test` to keep Rust files while hiding paths with
`test`. A bare `!` is ignored.

## Start a New File

```bash
ordex new-file.txt
```

If the file does not exist, Ordex opens an empty buffer associated with that path.

Save with `:w` to create the file on disk.

## Save and Overwrite Behavior

- `:w` writes the current file.
- `:wq` writes and quits on successful save.
- `:w <path>` or `:write <path>` writes to a new path and makes it the current file.
- If the destination already exists **and is different from the current buffer file path**,
  Ordex asks for confirmation:
  `Overwrite "<path>"? [y/N]`
- Press `y` or `Y` to confirm. Any other key cancels the write.
- `:w!`, `:wq!`, and `:w! <path>` bypass overwrite confirmation.

## Buffer Commands

- `:e <path>` or `:edit <path>` opens another buffer and switches to it.
- `:bn` / `:buffer-next` switches to the next open buffer.
- `:bp` / `:buffer-prev` switches to the previous open buffer.
- `:ls` / `:buffers` lists open buffers on the message line.
- `:bd` / `:buffer-delete` closes the active buffer.
- If `:bd` targets a dirty buffer, Ordex asks whether to save it before closing.

## Quit with Unsaved Changes

- `:q` quits immediately only when there are no unsaved changes.
- If there are unsaved changes, Ordex asks about each dirty buffer in turn:
  `Save changes to "<name>"? [y]es/[n]o/[c]ancel`
- Press `y`/`Y` to save and quit.
- Press `n`/`N` to discard the current dirty buffer and continue quitting.
- Press `c`/`C` (or any other key) to cancel and stay in the editor.
- `:q!` always quits immediately without saving.
- For unnamed buffers, choosing `y` shows `No file name` and keeps the editor open.

## Modified Indicator

The status bar shows `[+]` when the buffer has unsaved changes.
