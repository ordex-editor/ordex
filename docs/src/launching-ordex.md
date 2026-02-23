# Launching Ordex

## Basic Usage

```bash
ordex [file]
```

Examples:

```bash
ordex
ordex notes.txt
ordex README.md
```

When launched without a filename, Ordex starts with an empty unnamed buffer.

If the target file does not exist, Ordex will open a new buffer associated with that path.

## Interface Layout

Ordex renders:

- Main text area
- Status bar (mode, file name, cursor position, modified marker)
- Command/message line

The status bar shows `[+]` when the current buffer has unsaved changes.
