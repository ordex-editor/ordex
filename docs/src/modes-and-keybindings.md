# Modes and Keybindings

Ordex is modal. The active mode appears in the status bar.

On terminals that support DECSCUSR cursor-style control, Ordex requests a block
cursor in Normal and Visual modes, and a beam cursor in Insert, Command, and
Search modes.

## Normal Mode

Navigation and mode switching happen here.

The status bar `line:col` position reports logical buffer coordinates. The
always-visible top tab strip lists open buffers and highlights the active one.

- `h`, `j`, `k`, `l`: move left/down/up/right
- `w`: move to next word
- `b`: move to previous word
- `}`: move to the next blank separator line
- `{`: move to the previous blank separator line
- `f{char}`: find next `{char}` on current line
- `F{char}`: find previous `{char}` on current line
- `t{char}`: move right until before next `{char}` on current line
- `T{char}`: move left until after previous `{char}` on current line
- `%`: jump to the matching `()[]{}` / `<>` bracket or block-comment delimiter; if the cursor is not already on a delimiter, Ordex uses the next delimiter on the current line
- `;`: repeat last `f/F/t/T` in same direction
- `,`: repeat last `f/F/t/T` in opposite direction
- `gg`: move to the first line (keeps column when possible)
- `g$`: move to end of current line
- `g0`: move to start of current line
- `gd` (LSP): go to the symbol definition
- `gr` (LSP): go to one symbol reference
- `<Space>r` (LSP): prefill command mode with `rename {current_symbol}` for the symbol under the cursor
- `K` (LSP): show hover information for the symbol under the cursor
- `<Space>d` (LSP): open the active-buffer diagnostics picker
- `]d` (LSP): jump to the next active-buffer diagnostic
- `[d` (LSP): jump to the previous active-buffer diagnostic
- `zt`: place the current cursor row near the top of the viewport, respecting `scroll_margin`
- `zz`: place the current cursor row near the center of the viewport
- `zb`: place the current cursor row near the bottom of the viewport, respecting `scroll_margin`
- `Ctrl+F`: page forward
- `Ctrl+B`: page backward
- `Ctrl+D`: half-page forward
- `Ctrl+U`: half-page backward
- Generic operators: `d{motion}`, `c{motion}`, and `y{motion}` combine delete, change, or yank with supported motions and text objects
- `dw`, `de`, `db`: delete by word motions
- `dW`, `dE`, `dB`: delete by WORD motions
- `cw`, `cE`, `yw`, `ye`: change or yank with the same motion and text-object combinations
- `df{char}`, `dF{char}`, `dt{char}`, `dT{char}`: delete using line-local search motions
- `cf{char}`, `ct{char}`, `yf{char}`: change or yank using line-local search motions
- `d%`, `c%`, `y%`: operate through the matching delimiter resolved by `%`
- `diw`: delete inner word
- `ciw`: change inner word (delete and enter insert mode)
- `da(`: delete the smallest surrounding balanced `(...)` region
- `i`: enter insert mode
- `a`: append after cursor (move right and enter insert mode)
- `dd`, `cc`, `yy`: linewise forms of delete, change, and yank
- `p`: paste after the cursor, or below the current line for linewise yanks
- `P`: paste before the cursor, or above the current line for linewise yanks
- `x`: delete character under cursor
- `.`: repeat the last change, including counted normal-mode edits and completed insert/change/open-line sessions
- `u`: undo the most recent change
- `Ctrl+R`: redo the most recently undone change
- Delete-style edits such as `x`, `dw`, `diw`, `da(`, `dd`, and `c...` also replace the unnamed paste buffer
- `<Space>w`: save current file
- `<Space>q`: save current file and quit
- `<Space>b`: open a buffer-switch picker with fuzzy subsequence filtering over open buffers
- `<Space>f`: open a file picker with fuzzy subsequence filtering over files under the current working directory
- `:`: enter command mode
- `/`: enter search mode
- `n`: jump to next search occurrence
- `N`: jump to previous search occurrence
- Counts are supported for Normal-mode motions and operators (for example: `10j`, `5w`, `3fX`, `2dw`, `3dd`, `2diw`, `2d3iw`, `10G`, `10gg`)
- Leading `0` starts a count only after another digit (`20j`), while bare `0` keeps line-start motion
- Counted `f/F/t/T` is all-or-nothing on the current line (if the full count cannot be satisfied, the cursor does not move)
- Counts before `%` use percentage motion (`100%` jumps to the last line)
- Count prefixes are capped at `999999` for repeat-style actions; `N G`/`N gg` use the full parsed line number
- Multi-key shortcuts show a bottom-right discovery popup after the first key, listing available continuations and their actions
- The bottom message line shows the typed prefix while a multi-key sequence or operator is pending
- Pending `f/F/t/T` shows a matching one-key indicator while waiting for the target character
- `%` ignores brackets inside strings/comments during code matching, falls back to plaintext matching when started inside a string/comment, and passively highlights visible matches
- The buffer-switch picker keeps the active buffer unchanged while you move through matches, then switches only after `Enter`; `Esc` cancels the picker
- The top tab strip stays visible while switching buffers and follows the same
  buffer order as `:bn` / `:bp`
- The file picker scans the working directory asynchronously, streams partial results as they arrive, includes hidden paths, and respects `.gitignore` when Ordex is running inside a Git work tree
- The file picker matches against both basenames and relative paths, opens the highlighted file on `Enter`, and cancels on `Esc`
- The hover popup is read-only, opens near the cursor, and dismisses on the next keypress
- Language-server diagnostics render as gutter markers plus curly underlines/highlights in the active buffer
- Picker queries split on spaces, fuzzy-match positive terms as case-insensitive subsequences, and treat `!term` as a literal substring exclusion; bare `!` does nothing

## Visual Mode

Characterwise and linewise selection reuse the existing normal-mode motion set.

- `v`: enter characterwise visual mode
- `V`: enter linewise visual mode
- `gv`: recreate the most recent visual selection from normal mode
- Most normal-mode motions and counts continue to work while adjusting the selection
- `o`: swap the active cursor with the opposite end of the selection
- Multi-key discovery popups also appear for Visual-mode sequences such as `gg`, `g$`, `g0`, `zt`, `zz`, and `zb`
- `d`: delete the active selection and return to normal mode
- `c`: delete the active selection and enter insert mode
- `y`: yank the active selection and return to normal mode
- `Esc`: cancel the selection and return to normal mode

## Insert Mode

Text entry mode.

- Printable characters: insert text at cursor
- Automatic buffer-word completion appears while typing when the current prefix matches 3+ character words already present in the active buffer
- Explicit file-path completion appears in the same popup for `/`, `./`, `../`, and `~/`, and resolves `./` and `../` from the active buffer directory when available
- Completion matches prefixes case-insensitively and previews the selected candidate using the casing stored in the buffer
- `Up` / `Down`: move through completion candidates while the popup is visible; moving back to no selection restores the typed prefix
- `Ctrl+P` / `Ctrl+N`: alternate completion navigation shortcuts for previous / next suggestion
- `Backspace`: delete character before cursor
- `Enter`: insert new line
- `Esc`: return to normal mode
- Leaving Insert mode groups that whole insert session into one undo step, matching Vim-style undo behavior

## Command Mode

Executes editor commands typed after `:`.
See [Commands](./commands.md) for a command reference.

- `:w`: save file
- `:q`: quit editor
- `:q!`: quit without saving
- `:wq`: save and quit
- `:undo`: undo the most recent change
- `:redo`: redo the most recently undone change
- `:rename {new_name}`: request an LSP rename for the symbol under the cursor
- `:diagnostics`: open the active-buffer diagnostics picker
- `:next-diagnostic`: jump to the next active-buffer diagnostic
- `:prev-diagnostic`: jump to the previous active-buffer diagnostic
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
