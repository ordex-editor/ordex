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
ordex --help
ordex -h
ordex --version
ordex -- --dash-prefixed-file
```

When launched without a filename, Ordex starts with an empty unnamed buffer.

If a target file does not exist, Ordex will open a new buffer associated with that path.

Ordex also accepts multiple file paths at startup. Each path opens in its own
buffer, with the first file active initially.

`--help` and `-h` print startup usage and exit without opening the editor.
`--version` prints the Ordex version and exits without opening the editor.
Unknown flags exit with an error instead of opening a buffer named after the
flag. Use `--` before a dash-prefixed filename when that argument should be
treated as a path.

## Interface Layout

Ordex renders:

- Buffer tab strip (always visible, showing open buffers in order)
- Line-number gutter (absolute line numbers, plus `~` rows past EOF)
- Main text area
- Status bar (mode, file name, cursor position, modified marker, read-only marker)
- Command/message line
- LSP progress overlay (when background language-server work is active)

The status bar shows `[+]` when the current buffer has unsaved changes and a
colored `🔒` marker when the file is read-only. The tab strip keeps the active
buffer highlighted and prefers buffer basenames in narrow terminals.
