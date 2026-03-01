# Modes and Keybindings

Ordex is modal. The active mode appears in the status bar.

## Normal Mode

Navigation and mode switching happen here.

The status bar `line:col` position reports logical buffer coordinates.

- `h`, `j`, `k`, `l`: move left/down/up/right
- `w`: move to next word
- `b`: move to previous word
- `f{char}`: find next `{char}` on current line
- `F{char}`: find previous `{char}` on current line
- `t{char}`: move right until before next `{char}` on current line
- `T{char}`: move left until after previous `{char}` on current line
- `;`: repeat last `f/F/t/T` in same direction
- `,`: repeat last `f/F/t/T` in opposite direction
- `gg`: move to the first line (keeps column when possible)
- `g$`: move to end of current line
- `g0`: move to start of current line
- `Ctrl+F`: page forward
- `Ctrl+B`: page backward
- `diw`: delete inner word
- `ciw`: change inner word (delete and enter insert mode)
- `da(`: delete the smallest surrounding balanced `(...)` region
- `i`: enter insert mode
- `:`: enter command mode
- `/`: enter search mode
- `n`: jump to next search occurrence
- `N`: jump to previous search occurrence
- Multi-key `g` navigation shows a pending `g` indicator on the right side of the bottom message line
- Pending `f/F/t/T` shows a matching one-key indicator while waiting for the target character

## Insert Mode

Text entry mode.

- Printable characters: insert text at cursor
- `Backspace`: delete character before cursor
- `Enter`: insert new line
- `Esc`: return to normal mode

## Command Mode

Executes editor commands typed after `:`.
See [Commands](./commands.md) for a command reference.

- `:w`: save file
- `:q`: quit editor
- `:q!`: quit without saving
- `:wq`: save and quit
- `:{number}`: jump to a line
- `Esc`: cancel command input

## Search Mode

Find text in the buffer.

- `/pattern` then `Enter`: find next occurrence
- `n`: repeat search forward
- `N`: repeat search backward
- Search is case-sensitive and literal
- Search wraps to the beginning of the file
- `Esc`: cancel search input
