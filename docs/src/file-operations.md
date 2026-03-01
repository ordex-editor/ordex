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

## Modified Indicator

The status bar shows `[+]` when the buffer has unsaved changes.
