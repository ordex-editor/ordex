# Quickstart

## Build

```bash
cargo build --release
```

The binary will be available at:

```text
target/release/ordex
```

## Open a File

```bash
ordex README.md
```

## Core Flow

1. Move in normal mode with `h`, `j`, `k`, `l`
2. Jump to the top of file with `gg`
3. Press `i` to enter insert mode
4. Type text
5. Press `Esc` to return to normal mode
6. Type `:w` then press `Enter` to save
7. Type `:q` then press `Enter` to quit

## One-Command Save and Quit

Use:

```text
:wq
```
