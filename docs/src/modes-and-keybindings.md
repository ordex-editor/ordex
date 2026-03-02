# Modes and Keybindings

Ordex is modal. The active mode appears in the status bar.

## Normal Mode

Navigation and mode switching happen here.

The status bar `line:col` position reports logical buffer coordinates.

- `h`, `j`, `k`, `l`: move left/down/up/right
- `w`: move to next word
- `b`: move to previous word
- `}`: move to the next blank separator line
- `{`: move to the previous blank separator line
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
- `Ctrl+D`: half-page forward
- `Ctrl+U`: half-page backward
- `diw`: delete inner word
- `ciw`: change inner word (delete and enter insert mode)
- `da(`: delete the smallest surrounding balanced `(...)` region
- `i`: enter insert mode
- `a`: append after cursor (move right and enter insert mode)
- `x`: delete character under cursor
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
- `Ctrl+A`, `Home`: move input cursor to start
- `Ctrl+E`, `End`: move input cursor to end
- `Ctrl+B`, `Left`: move input cursor left
- `Ctrl+F`, `Right`: move input cursor right
- `Alt+B`: move input cursor one Vim-style word left
- `Alt+F`: move input cursor one Vim-style word right
- `Ctrl+W`: delete previous Vim-style word
- `Ctrl+U`: delete from cursor to start of input
- `Ctrl+K`: delete from cursor to end of input
- `Ctrl+H` or `Backspace`: delete character before cursor
- `Ctrl+D` or `Delete`: delete character under cursor
- `Esc`: cancel command input

## Search Mode

Find text in the buffer.

- `/pattern` then `Enter`: find next occurrence
- `n`: repeat search forward
- `N`: repeat search backward
- Search is case-sensitive and literal
- Search wraps to the beginning of the file
- Search input supports the same inline editing key bindings as command mode
- `Esc`: cancel search input
