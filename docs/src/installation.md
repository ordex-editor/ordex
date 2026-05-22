# Installation and Build

## Requirements

- Rust stable toolchain
- POSIX-compatible terminal with ANSI support
- `wl-copy` / `wl-paste` on Wayland or `xclip` on X11 if you want system clipboard support

## Build from Source

```bash
cargo build --release
```

Run the binary from:

```text
target/release/ordex
```

## Verify Build

```bash
cargo test
```
