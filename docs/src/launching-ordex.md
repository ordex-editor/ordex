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
ordex README.md Cargo.toml
```

When launched without a filename, Ordex starts with an empty unnamed buffer.

If a target file does not exist, Ordex will open a new buffer associated with that path.

Ordex also accepts multiple file paths at startup. Each path opens in its own
buffer, with the first file active initially.

## Interface Layout

Ordex renders:

- Buffer tab strip (always visible, showing open buffers in order)
- Line-number gutter (absolute line numbers, plus `~` rows past EOF)
- Main text area
- Status bar (mode, file name, cursor position, modified marker)
- Command/message line

The status bar shows `[+]` when the current buffer has unsaved changes. The tab
strip keeps the active buffer highlighted and prefers buffer basenames in narrow
terminals.
