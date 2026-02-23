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

## Modified Indicator

The status bar shows `[+]` when the buffer has unsaved changes.

