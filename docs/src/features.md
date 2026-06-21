# Features

ordex is a terminal-based text editor that combines Vim-like modal editing with modern features. Key capabilities include:

## Editor Core
- Line numbers (absolute or relative via `relative_line_numbers` setting)
- Theme-aware current-line highlighting across the editor content area
- Terminal window title updates on full redraws as `<buffer-name> (<cwd>) - ordex`
- Soft line wrapping enabled by default, with a config setting to disable it
- Horizontal scrolling with configurable margin (`horizontal_scroll_margin`)
- Visible whitespace markers: tabs, non-breaking spaces, and trailing spaces (configurable)
- Multi-key sequence discovery popup showing available continuations after first key
- Jump history with count support (`3Ctrl+O` jumps back 3 times)

## Editing Modes
- Modal editing: `NORMAL`, `VISUAL`, `VISUAL LINE`, `VISUAL BLOCK`, `INSERT`, `COMMAND`, `SEARCH`
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
   - Operator motions include line-position targets: `$` (line end), `0` (line start), `^` (first non-blank)
   - Text objects: `i`/`a` prefix with words (`w`/`W`), balanced bracket pairs (`()`/`[]`/`{}`/`<>`), and quote pairs (`"`, `'`, `` ` ``); Vim-style aliases `b` = `)` and `B` = `}`
   - Operator aliases (e.g., `D` for `d$`, `C` for `c$`, `Y` for `y$`)
   - Line joining (`J`)
   - Single-character replace (`r`)
   - Number increment/decrement (`Ctrl+A` / `Ctrl+X`)
   - Line and block comment toggles (`<Space>c` / `<Space>C`)
   - Characterwise, linewise, and blockwise visual selections
   - Mirrored blockwise visual `I` / `A` inserts across touched lines
   - Vim-style `"+` / `"*` clipboard registers
   - `<Space>p` / `<Space>P` clipboard paste shortcuts
   - Auto-insert: Automatic bracket closing, comment continuation, and language-aware indentation

## Autocompletion
- Automatic insert-mode completion with case-insensitive buffer-word, file-path, and LSP suggestions, with live preview
- Command completion for command names, file paths, and session names
- LSP signature help with active-parameter highlighting during function calls

## LSP & Code Intelligence
- Built-in LSP defaults with per-language support for completions, signature help, navigation, hover, rename, code actions, and diagnostics
- LSP code intelligence:
   - `gd` (goto definition)
   - `gr` (show references)
   - `K` (hover information)
   - `<Space>a` (code actions)
   - `:rename` (symbol renaming)
   - Gutter diagnostics with curly underlines
   - `]d` / `[d` (next/previous diagnostic)
- Syntax highlighting for 72 languages (see [full list](./syntax-highlighting.md))

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
   - `:wq` (save and quit)
   - `:wq!` (force save and quit)
   - `:x` (save if modified and quit)
   - `:reload-config` (reload configuration)
   - `:diagnostics` (show diagnostics)
- Navigation commands:
   - `:{number}` (go to line)

## Input Editing (Command & Search Modes)
- Advanced prompt editing with `Ctrl+W` (delete word), `Ctrl+U` (delete to start), `Ctrl+K` (delete to end)
- Word-based cursor movement in prompts with `Ctrl+Left` / `Ctrl+Right`
- Home/End key support in prompts
- History navigation: `Up`/`Down` (prefix-filtered), `Ctrl+P`/`Ctrl+N` (full history)
- Session-local history per mode (command vs search) with adjacent-deduplication

## UI & Navigation
- Picker dialogs for:
   - Fuzzy buffer switching (recent named buffers near top)
   - Recursive file opening from working directory
   - Location picker (multi-target navigation results)
   - Diagnostic picker (LSP diagnostics with navigation)
   - Code action picker (LSP code actions with preview)
   - Search/grep results picker
   - Syntax-highlighted previews on wide terminals

## File Management & Monitoring
- External file change detection with fingerprint-based comparison
- Automatic file reload on disk changes (configurable via `auto_reload_external_changes`)
- Reload behavior: clean buffers auto-reload; buffers with changes prompt user

## Crash Recovery
- Crash recovery via swap files stored under XDG cache directory
- Concurrent-open warnings with read-only/recover/discard/continue options
- Swap file exclusions via glob patterns for sensitive files (e.g., `*.gpg`, `/dev/shm/*`)

## Search & Replace
- Search commands:
   - `/pattern` with `n`/`N` repeat
   - `\n` for matching line breaks in search patterns
- Replacement commands:
    - `\r` for inserting line breaks in substitutions
    - `$1`, `$2`, etc. for capture group references in replacements
    - Live-preview global `:s` / `:%s` regex substitute commands
- Hide committed search highlighting with `<Space>l`

## Undo & Redo
- Full undo/redo support (`u` / `Ctrl+R`)
- Single undo step for complete insert sessions (Vim-like behavior)

## Macros
- Session-local macro recording (`q{a-z}`) and playback (`@{a-z}`, `@@`)
- Counted macro playback: `3@a` replays macro 3 times
- Macro support for Normal, Insert, Visual, Command, and Search modes

## Other Features
- **Insert mode text insertion**: `Ctrl+V` for literal character insertion (Tab and visible printables)
- **Exit code control**: `:cq` command to quit with exit code 1 (compile error convention), `:update` (save and quit if modified)
- **Project session management**: `:save-session`, `:open-session`, `:delete-session` for project persistence
- **Repeat last action**: `.` command repeats last change, including counted edits and insert sessions
- **Read-only file indicator**: Unicode indicator in status bar shows when file is read-only
- **Terminal window event handling**: Proper SIGTERM handling to restore terminal on process termination
- **Visual mode features**:
   - Ability to recreate last selection with `gv`
   - Swap visual anchor with `o` to adjust selection endpoints
- **Space-mode shortcuts**: `<Space>q` saves and quits, `<Space>w` saves current file
- **Configuration multi-actions**: Array and sequence-based multi-action bindings via config
