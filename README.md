# Ordex

> **Warning:** Do **NOT** use this project as it is not ready to use.

> **Note:** Significant portions of this project were designed and implemented with the help of advanced AI systems, blending automated code generation with human review and refinement.

> **Alpha warning:** This project is currently in alpha. Expect bugs, and use caution because document loss is possible.

A TUI text editor written in Rust with vim-style keybindings.

## Documentation

- User guide source: `docs/`
- [Published docs site](https://ordex-editor.github.io/ordex/)

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
ordex [file...]
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

- Always-on line numbers with dynamic gutter width and EOF `~` markers
- Theme-aware current-line highlighting across the editor content area
- Soft line wrapping enabled by default, with a config setting to disable it
- Syntax highlighting for 72 languages and Linux-oriented config/build formats; see `docs/src/syntax-highlighting.md` for the full table
- Modal editing: NORMAL, VISUAL, VISUAL LINE, VISUAL BLOCK, INSERT, COMMAND, SEARCH
- Navigation: character, word/WORD, page, `ge` / `gE` backward word-end motions, `gf` / `gF` file jumps, `ga` alternate-file switching, `g.` last-change jumping, `*` search-under-cursor, and line-local `f/F/t/T` character motions with `;`/`,` repeat
- Editing: insert text, generic `d`/`c`/`y`/`=` operator bindings such as `dw`, `cE`, `ye`, `==`, `=iw`, `dfx`, `ct,`, aliases such as `D`/`C`, line joining with `J`, single-character replace with `r`, number increment/decrement with `Ctrl+A` / `Ctrl+X`, and characterwise, linewise, and blockwise visual selections
- Macros: session-local Vim-style recording and replay with lowercase registers via `q{register}`, `@{register}`, and `@@`
- Automatic insert-mode completion with case-insensitive buffer-word, file-path, and LSP suggestions, with live preview
- Automatic LSP signature help in Insert mode for supported calls, including active-parameter highlighting
- Multiple buffer support with startup multi-file arguments and `:e`, `:new`, `:bn`, `:bp`, `:ls`, `:bd`
- Picker dialogs for fuzzy buffer switching with recent named buffers near the top and recursive file opening from the working directory
- File commands: `:w`, `:w!`, `:wall`, `:wa`, `:q`, `:wq`, `:wq!`, `:x`, `:reload-config`, `:diagnostics`
- Built-in LSP defaults with per-language support for completions, signature help, navigation, hover, rename, code actions, and diagnostics
- LSP code intelligence: `gd`, `gr`, `K`, insert-mode signature help, `<Space>a`, `:rename`, gutter diagnostics, curly underlines, and `]d` / `[d`
- Crash recovery and concurrent-open warnings via swap files stored under the XDG cache directory
- Search and replace: `/pattern` with `n`/`N` repeat, `<Space>l` to hide committed search highlighting, plus global-by-default `:s` / `:%s` regex substitute commands
- Go to line: `:{number}`

Swap files are enabled by default for named open buffers. Ordex keeps them until
the owning instance exits, warns when another instance already owns one, and
offers read-only, recover, discard, and continue-editing choices from that
prompt. Use `[swap].exclude` in the config file to skip swap creation for
sensitive paths such as encrypted notes or
password-store working files.

## Requirements

- Rust (stable)
- POSIX-compatible terminal with ANSI support
- Language-server binaries available on `PATH` for the languages you want to use

## Development

Run checks locally:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
