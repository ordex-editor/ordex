# Features

ordex is a terminal-based text editor that combines Vim-like modal editing with modern features. Key capabilities include:

## Editor Core
- Always-on line numbers with dynamic gutter width and EOF `~` markers
- Theme-aware current-line highlighting across the editor content area
- Terminal window title updates on full redraws as `<buffer-name> (<cwd>) - ordex`
- Soft line wrapping enabled by default, with a config setting to disable it

## Editing Modes
- Modal editing: NORMAL, VISUAL, VISUAL LINE, VISUAL BLOCK, INSERT, COMMAND, SEARCH
- Navigation: character, word/WORD, page, `ge` / `gE` backward word-end motions, `gf` / `gF` file jumps, `ga` alternate-file switching, `g.` last-change jumping, `*` search-under-cursor, and line-local `f/F/t/T` character motions with `;`/`,` repeat
- Editing: insert text, terminal bracketed paste, generic `d`/`c`/`y`/`=` operator bindings such as `dw`, `cE`, `ye`, `==`, `=iw`, `dfx`, `ct,`, aliases such as `D`/`C`, line joining with `J`, single-character replace with `r`, number increment/decrement with `Ctrl+A` / `Ctrl+X`, line and block comment toggles on `<Space>c` / `<Space>C`, characterwise, linewise, and blockwise visual selections, mirrored blockwise visual `I` / `A` inserts across touched lines, Vim-style `"+` / `"*` clipboard registers, and `<Space>p` / `<Space>P` clipboard paste shortcuts

## Autocompletion
- Automatic insert-mode completion with case-insensitive buffer-word, file-path, and LSP suggestions, with live preview

## LSP & Code Intelligence
- Built-in LSP defaults with per-language support for completions, signature help, navigation, hover, rename, code actions, and diagnostics
- LSP code intelligence: `gd`, `gr`, `K`, insert-mode signature help, `<Space>a`, `:rename`, gutter diagnostics, curly underlines, and `]d` / `[d`
- Syntax highlighting for 72 languages (see [full list](docs/src/syntax-highlighting.md))

## Buffer Management
- Multiple buffer support with startup multi-file arguments and `:e`, `:new`, `:bn`, `:bp`, `:ls`, `:bd`
- File commands: `:w`, `:w!`, `:wall`, `:wa`, `:q`, `:wq`, `:wq!`, `:x`, `:reload-config`, `:diagnostics`

## Picker & UI
- Picker dialogs for fuzzy buffer switching with recent named buffers near the top, recursive file opening from the working directory, and syntax-highlighted previews on wide terminals

## Crash Recovery
- Crash recovery and concurrent-open warnings via swap files stored under the XDG cache directory

## Search & Replace
- Search and replace: `/pattern` with `n`/`N` repeat, `\n` search line breaks, `<Space>l` to hide committed search highlighting, plus live-preview global-by-default `:s` / `:%s` regex substitute commands with `\r` replacement line breaks
- Go to line: `:{number}`