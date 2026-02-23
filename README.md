# Ordex

> **Note:** Significant portions of this project were designed and implemented with the help of advanced AI systems, blending automated code generation with human review and refinement.

A TUI text editor written in Rust with vim-style keybindings.

## Documentation

- User guide source: `docs/`
- Published docs site: `https://antoyo.github.io/ordex/`

For local docs development:

```bash
mdbook build docs
mdbook serve docs
```

## Quickstart

Build:

```bash
cargo build --release
```

Run:

```bash
ordex [file]
```

Example:

```bash
ordex README.md
```

Ordex can also be launched without a filename:

```bash
ordex
```

## Features (Overview)

- Modal editing: NORMAL, INSERT, COMMAND, SEARCH
- Navigation: character, word, and page movement
- Editing: insert text, delete, create new lines
- File commands: `:w`, `:q`, `:wq`
- Search: `/pattern` with `n`/`N` repeat (case-sensitive literal match)
- Go to line: `:{number}`

## Requirements

- Rust (stable)
- POSIX-compatible terminal with ANSI support

## Development

Run checks locally:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test -- --test-threads=1
```
