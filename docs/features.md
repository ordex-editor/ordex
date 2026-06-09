# Features

ordex is a terminal-based text editor that combines Vim-like modal editing with modern features. Key capabilities include:

## Editor Core
- Always-on line numbers with dynamic gutter width and EOF `~` markers
- Theme-aware current-line highlighting across the editor content area
- Terminal window title updates on full redraws as `<buffer-name> (<cwd>) - ordex`
- Soft line wrapping enabled by default, with a config setting to disable it

## Editing Modes
- Modal editing: NORMAL, VISUAL, VISUAL LINE, VISUAL BLOCK, INSERT, COMMAND, SEARCH
- Navigation:
  - Character, word/WORD, and page movements
  - `ge` / `gE` (backward word-end motions)
  - `gf` / `gF` (file jumps)
  - `ga` (alternate-file switching)
  - `g.` (last-change jumping)
  - `*` (search-under-cursor)
  - Line-local `f/F/t/T` (character motions) with `;`/`,` repeat
- Editing:
  - Insert text
  - Terminal bracketed paste
  - Generic `d`/`c`/`y`/`=` operator bindings (e.g., `dw`, `cE`, `ye`, `==`, `=iw`)
  - Aliases (e.g., `D`, `C`)
  - Line joining (`J`)
  - Single-character replace (`r`)
  - Number increment/decrement (`Ctrl+A` / `Ctrl+X`)
  - Line and block comment toggles (`<Space>c` / `<Space>C`)
  - Characterwise, linewise, and blockwise visual selections
  - Mirrored blockwise visual `I` / `A` inserts across touched lines
  - Vim-style `"+` / `"*` clipboard registers
  - `<Space>p` / `<Space>P` clipboard paste shortcuts

## Autocompletion
- Automatic insert-mode completion with case-insensitive buffer-word, file-path, and LSP suggestions, with live preview

## LSP & Code Intelligence
- Built-in LSP defaults with per-language support for completions, signature help, navigation, hover, rename, code actions, and diagnostics
- LSP code intelligence:
  - `gd`, `gr`, `K` (goto definitions)
  - Insert-mode signature help with active-parameter highlighting
  - `<Space>a` (code actions)
  - `:rename` (symbol renaming)
  - Gutter diagnostics with curly underlines
  - `]d` / `[d` (next/previous diagnostic)
- Syntax highlighting for 72 languages (see [full list](docs/src/syntax-highlighting.md))

## Buffer Management
- Multiple buffer support with:
  - Startup multi-file arguments
  - `:e` (edit)
  - `:new` (new buffer)
  - `:bn` / `:bp` (next/previous buffer)
  - `:ls` (list buffers)
  - `:bd` (close buffer)
- File commands:
  - `:write` (save buffer)
  - `:write!` (force save)
  - `:wall` / `:wa` (write all buffers)
  - `:quit` (close without saving)
  - `:writequit` / `:wq` (save and close)
  - `:writequit!` / `:wq!` (force save and close)
  - `:exit` / `:x` (close buffer)
  - `:reload-config` (reload config)
  - `:diagnostics` (show diagnostics)

## UI & Navigation
- Picker dialogs for:
  - Fuzzy buffer switching (recent named buffers near top)
  - Recursive file opening from working directory
  - Syntax-highlighted previews on wide terminals

## Crash Recovery
- Crash recovery via swap files stored under XDG cache directory
- Concurrent-open warnings with read-only/recover/discard/continue options

## Search & Replace
- Search: `/pattern` with `n`/`N` repeat and `
` for line breaks
- Search highlighting (`<Space>l` to hide)`
- Live-preview global `:s` / `:%s` substitute commands with `` replacement line breaks
- Go to line: `:{number}`