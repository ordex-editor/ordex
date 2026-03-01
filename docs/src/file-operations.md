# File Operations

## Open Existing Files

```bash
ordex path/to/file.txt
```

Ordex reads the file and displays it in the editor.

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

## Quit with Unsaved Changes

- `:q` quits immediately only when there are no unsaved changes.
- If there are unsaved changes, Ordex asks:
  `Save changes to "<name>"? [y]es/[n]o/[c]ancel`
- Press `y`/`Y` to save and quit.
- Press `n`/`N` to quit without saving.
- Press `c`/`C` (or any other key) to cancel and stay in the editor.
- `:q!` always quits immediately without saving.
- For unnamed buffers, choosing `y` shows `No file name` and keeps the editor open.

## Modified Indicator

The status bar shows `[+]` when the buffer has unsaved changes.
