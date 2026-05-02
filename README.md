# Ordex

> **Warning:** Do **NOT** use this project as it is not ready to use.

> **Note:** Significant portions of this project were designed and implemented with the help of advanced AI systems, blending automated code generation with human review and refinement.

> **Alpha warning:** This project is currently in alpha. Expect bugs, and use caution because document loss is possible.

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
- Soft line wrapping enabled by default, with a config setting to disable it
- Syntax highlighting for 72 languages and Linux-oriented config/build formats; see `docs/src/syntax-highlighting.md` for the full table
- Modal editing: NORMAL, VISUAL, VISUAL LINE, INSERT, COMMAND, SEARCH
- Navigation: character, word, page, and line-local `f/F/t/T` character motions with `;`/`,` repeat
- Editing: insert text, generic `d`/`c`/`y` operator bindings such as `dw`, `cE`, `ye`, `dfx`, `ct,`, linewise `dd`/`cc`/`yy`, and visual `d`/`c` selections
- Automatic insert-mode completion with case-insensitive buffer-word, file-path, and LSP suggestions, with live preview
- Automatic LSP signature help in Insert mode for supported calls, including active-parameter highlighting
- Multiple buffer support with startup multi-file arguments and `:e`, `:bn`, `:bp`, `:ls`, `:bd`
- Picker dialogs for fuzzy buffer switching and recursive file opening from the working directory
- File commands: `:w`, `:w!`, `:q`, `:wq`, `:wq!`, `:reload-config`, `:diagnostics`
- Built-in LSP defaults with per-language support for completions, signature help, navigation, hover, rename, code actions, and diagnostics
- LSP code intelligence: `gd`, `gr`, `K`, insert-mode signature help, `<Space>a`, `:rename`, gutter diagnostics, curly underlines, and `]d` / `[d`
- Crash recovery via swap files stored under the XDG cache directory
- Search: `/pattern` with `n`/`N` repeat (case-sensitive literal match)
- Go to line: `:{number}`

Swap files are enabled by default for edited buffers. Use `[swap].exclude` in the
config file to skip swap creation for sensitive paths such as encrypted notes or
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
