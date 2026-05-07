# Modes and Keybindings

Ordex is modal. The active mode appears in the status bar.

On terminals that support DECSCUSR cursor-style control, Ordex requests a block
cursor in Normal and Visual modes, and a beam cursor in Insert, Command, and
Search modes.

The tables below use **Command** for the parseable action or command syntax
behind each binding. When a row describes a built-in composite flow rather than
one standalone action name, the column shows the action sequence or `—`.

## Normal Mode

Navigation and mode switching happen here.

The status bar `line:col` position reports logical buffer coordinates. The
always-visible top tab strip lists open buffers and highlights the active one.

### Movement, jumps, and pickers

| Key | Description | Command |
| --- | --- | --- |
| `h`, `j`, `k`, `l` | Move left, down, up, or right. | `move-left`, `move-down`, `move-up`, `move-right` |
| `w` | Move to the next word. | `move-word-forward` |
| `b` | Move to the previous word. | `move-word-backward` |
| `ge` | Move to the end of the previous word. | `move-word-end-backward` |
| `gE` | Move to the end of the previous WORD. | `move-big-word-end-backward` |
| `}` | Move to the next blank separator line. | `move-paragraph-forward` |
| `{` | Move to the previous blank separator line. | `move-paragraph-backward` |
| `f{char}` | Find the next `{char}` on the current line. | `find-forward` |
| `F{char}` | Find the previous `{char}` on the current line. | `find-backward` |
| `t{char}` | Move right until just before the next `{char}` on the current line. | `till-forward` |
| `T{char}` | Move left until just after the previous `{char}` on the current line. | `till-backward` |
| `%` | Jump to the matching `()[]{}` / `<>` bracket or block-comment delimiter. If the cursor is not already on a delimiter, Ordex uses the next delimiter on the current line. | `jump-to-matching-delimiter` |
| `;` | Repeat the last `f` / `F` / `t` / `T` in the same direction. | `repeat-find-forward` |
| `,` | Repeat the last `f` / `F` / `t` / `T` in the opposite direction. | `repeat-find-backward` |
| `gg` | Move to the first line and keep the column when possible. | `move-to-first-line` |
| `g$` | Move to the end of the current line. | `move-line-end` |
| `g0` | Move to the start of the current line. | `move-line-start` |
| `Ctrl+O` | Jump to the previous entry in jump history. | `jump-older` |
| `Tab` / `Ctrl+I` | Jump to the next entry in jump history. | `jump-newer` |
| `gd` | Go to the symbol definition. | `goto-definition` |
| `gr` | Go to one symbol reference. | `goto-references` |
| `gf` | Open the filename-like token under the cursor relative to the current buffer or working directory. | `goto-file-under-cursor` |
| `gF` | Open the filename-like token under the cursor and jump to a trailing `:line[:column]` when present. | `goto-file-under-cursor-at-position` |
| `ga` | Jump to the most recently visited named buffer that is still open. | `goto-alternate-file` |
| `g.` | Jump to the cursor position after the most recently committed change in the session. | `goto-last-modification` |
| `<Space>a` | Open a code-action picker for the current cursor context, even when only one supported action is available. | `open-code-actions` |
| `<Space>r` | Prefill command mode with `rename {current_symbol}` using the active syntax profile's identifier rules. | `prompt-rename-symbol` |
| `K` | Show hover information for the symbol under the cursor. | `show-hover` |
| `<Space>d` | Open the active-buffer diagnostics picker. | `open-diagnostics-picker` |
| `]d` | Jump to the next active-buffer diagnostic. | `next-diagnostic` |
| `[d` | Jump to the previous active-buffer diagnostic. | `prev-diagnostic` |
| `zt` | Place the current cursor row near the top of the viewport, respecting `scroll_margin`. | `align-viewport-top` |
| `zz` | Place the current cursor row near the center of the viewport. | `align-viewport-center` |
| `zb` | Place the current cursor row near the bottom of the viewport, respecting `scroll_margin`. | `align-viewport-bottom` |
| `Ctrl+F` | Page forward. | `page-down` |
| `Ctrl+B` | Page backward. | `page-up` |
| `Ctrl+D` | Half-page forward. | `half-page-down` |
| `Ctrl+U` | Half-page backward. | `half-page-up` |
| `<Space>b` | Open a buffer-switch picker with fuzzy subsequence filtering over open buffers. | `open-buffer-switcher` |
| `<Space>f` | Open a file picker with fuzzy subsequence filtering over files under the current working directory. | `open-file-picker` |
| `:` | Enter command mode. | `enter-command-mode` |
| `/` | Enter search mode. | `enter-search-mode` |
| `n` | Jump to the next search occurrence. | `search-next` |
| `N` | Jump to the previous search occurrence. | `search-previous` |

### Editing, paste, undo, and macros

| Key | Description | Command |
| --- | --- | --- |
| `d{motion}`, `c{motion}`, `y{motion}`, `={motion}` | Combine delete, change, yank, or manual indent with supported motions and text objects. | `begin-delete-operator`, `begin-change-operator`, `begin-yank-operator`, `begin-indent-operator` + operator motion |
| `dw`, `de`, `db` | Delete by word motions. | `begin-delete-operator` + `word-forward`, `word-end`, `word-backward` |
| `dW`, `dE`, `dB` | Delete by WORD motions. | `begin-delete-operator` + `big-word-forward`, `big-word-end`, `big-word-backward` |
| `cw`, `cE`, `yw`, `ye` | Change or yank with the same motion combinations. | `begin-change-operator` / `begin-yank-operator` + matching operator motion |
| `df{char}`, `dF{char}`, `dt{char}`, `dT{char}` | Delete using line-local search motions. | `begin-delete-operator` + `find-forward`, `find-backward`, `till-forward`, `till-backward` |
| `cf{char}`, `ct{char}`, `yf{char}` | Change or yank using line-local search motions. | `begin-change-operator` / `begin-yank-operator` + matching operator motion |
| `d%`, `c%`, `y%` | Operate through the matching delimiter resolved by `%`. | operator + `jump-to-matching-delimiter` |
| `diw` | Delete the inner word. | `begin-delete-operator` + `text-object-inner` + `word-forward` |
| `ciw` | Change the inner word, then enter Insert mode. | `begin-change-operator` + `text-object-inner` + `word-forward` |
| `da(` | Delete the smallest surrounding balanced `(...)` region. | `begin-delete-operator` + `text-object-around` |
| `==` | Reindent the current line. | `begin-indent-operator` |
| `=iw` | Reindent the lines touched by the current text object. | `begin-indent-operator` + `text-object-inner` + `word-forward` |
| `i` | Enter Insert mode. | `enter-insert-mode` |
| `a` | Move right, then enter Insert mode. | `insert-after-cursor` |
| `dd` | Delete the current line. | — |
| `cc` | Change the current line and enter Insert mode. | — |
| `yy` | Yank the current line. | `yank-current-line` |
| `p` | Paste after the cursor, or below the current line for linewise yanks. | `paste-after-cursor` |
| `P` | Paste before the cursor, or above the current line for linewise yanks. | `paste-before-cursor` |
| `x` | Delete the character under the cursor. | `delete-char-at-cursor` |
| `.` | Repeat the last change, including counted normal-mode edits and completed insert, change, and open-line sessions. | `repeat-last-change` |
| `q{register}` | Start recording into lowercase register `{register}`. Press `q` again in Normal mode to stop. | `begin-macro-record` |
| `@{register}` | Replay the contents of lowercase register `{register}`. | `begin-macro-playback` |
| `@@` | Replay the most recently replayed macro again. | `begin-macro-playback` |
| `u` | Undo the most recent change. | `undo` |
| `Ctrl+R` | Redo the most recently undone change. | `redo` |
| `<Space>w` | Save the current file. | `save-current-file` |
| `<Space>q` | Save the current file and quit. | `save-current-file-and-quit` |

### Normal-mode behavior notes

- Jump history records LSP definition and reference jumps, `gf` / `gF`,
  search-result jumps, `gg` / `G` / `:{number}`, and diagnostic jumps.
- Plain local motions such as `h`, `j`, `k`, `l`, `w`, and `b` do not create
  jump-history entries.
- `:s<delim>pattern<delim>replacement<delim>` replaces every regex match on the
  current line.
- `:%s<delim>pattern<delim>replacement<delim>` replaces every regex match in the
  current buffer.
- Substitute is global by default inside its scope. The trailing delimiter may
  be omitted when nothing follows the replacement.
- Counts are supported for Normal-mode motions, operators, counted
  insert-entry commands, and counted search or command entry, for example:
  `10j`, `5w`, `3fX`, `2dw`, `3dd`, `2diw`, `2d3iw`, `10G`, `10gg`,
  `3iabc<Esc>`, `2A!<Esc>`, `5:`, and `3/word<Enter>`.
- Leading `0` starts a count only after another digit, so `20j` is a count
  while bare `0` keeps line-start motion.
- Counted `f` / `F` / `t` / `T` is all-or-nothing on the current line. If the
  full count cannot be satisfied, the cursor does not move.
- Counts before `%` use percentage motion, so `100%` jumps to the last line.
- Counts before `@{register}` repeat macro playback that many times.
- Count prefixes are capped at `999999` for repeat-style actions. `N G` and
  `N gg` use the full parsed line number.
- Multi-key shortcuts show a bottom-right discovery popup after the first key
  and list available continuations and their actions.
- The bottom message line shows the typed prefix while a multi-key sequence or
  operator is pending.
- Pending `f` / `F` / `t` / `T` shows a matching one-key indicator while
  waiting for the target character.
- Active macro recording shows a `recording @{register}` indicator on the
  message line.
- `%` ignores brackets inside strings and comments during code matching, falls
  back to plaintext matching when started inside a string or comment, and
  passively highlights visible matches.
- Delete-style edits such as `x`, `dw`, `diw`, `da(`, `dd`, and `c...` also
  replace the unnamed paste buffer.
- The buffer-switch picker keeps the active buffer unchanged while you move
  through matches, then switches only after `Enter`. `Esc` cancels the picker.
- The top tab strip stays visible while switching buffers and follows the same
  buffer order as `:bn` / `:bp`.
- The file picker scans the working directory asynchronously, streams partial
  results as they arrive, includes hidden paths, and respects `.gitignore` when
  Ordex is running inside a Git work tree.
- The file picker matches against both basenames and relative paths, opens the
  highlighted file on `Enter`, and cancels on `Esc`.
- The code-action picker applies only edit-bearing actions that Ordex can
  perform locally. `Esc` cancels without changing the buffer.
- The hover popup is read-only, opens near the cursor, and dismisses on the
  next keypress.
- Language-server diagnostics render as gutter markers plus curly
  underlines and highlights in the active buffer.
- Macros are session-local, support Normal, Insert, Visual, Command, and Search
  flows, and intentionally do not support recursive playback or picker-dialog
  interactions.
- Picker queries split on spaces, fuzzy-match positive terms as
  case-insensitive subsequences, and treat `!term` as a literal substring
  exclusion. Bare `!` does nothing.

## Visual Mode

Characterwise and linewise selection reuse the existing normal-mode motion set.

| Key | Description | Command |
| --- | --- | --- |
| `v` | Enter characterwise Visual mode. | `enter-visual-mode` |
| `V` | Enter linewise Visual mode. | `enter-visual-line-mode` |
| `gv` | Recreate the most recent visual selection from Normal mode. | `recreate-last-selection` |
| Most Normal-mode motions and counts | Continue to work while adjusting the selection. | — |
| `o` | Swap the active cursor with the opposite end of the selection. | `swap-visual-anchor` |
| `d` | Delete the active selection and return to Normal mode. | `delete-selection` |
| `c` | Delete the active selection and enter Insert mode. | `change-selection` |
| `y` | Yank the active selection and return to Normal mode. | `yank-selection` |
| `=` | Reindent every line touched by the active selection and return to Normal mode. | `indent-selection` |
| `Esc` | Cancel the selection and return to Normal mode. | `exit-to-normal-mode` |

Multi-key discovery popups also appear for Visual-mode sequences such as `gg`,
`g$`, `g0`, `zt`, `zz`, and `zb`.

## Insert Mode

Text entry mode.

| Key | Description | Command |
| --- | --- | --- |
| Printable characters | Insert text at the cursor. | — |
| `Up` / `Down` | Move through completion candidates while the popup is visible. Moving back to no selection restores the typed prefix. | `completion-select-up`, `completion-select-down` |
| `Ctrl+P` / `Ctrl+N` | Alternate completion navigation shortcuts for previous and next suggestion. | `completion-select-up`, `completion-select-down` |
| `Backspace` | Delete the character before the cursor. | `delete-char-backward` |
| `Enter` | Insert a new line. | `insert-newline` |
| `Esc` | Return to Normal mode. | `exit-to-normal-mode` |

- Automatic buffer-word completion appears while typing when the current prefix
  matches 3+ character words already present in the active buffer.
- LSP-backed completion suggestions join the same popup for saved files in
  supported projects, with kind labels such as `function` and `variable`.
- LSP signature help opens automatically after supported trigger characters such
  as `(` and `,`, highlights the active parameter, and can appear alongside
  completion when both fit.
- Explicit file-path completion appears in the same popup for `/`, `./`,
  `../`, and `~/`, and resolves `./` and `../` from the active buffer directory
  when available.
- Completion matches prefixes case-insensitively and previews the selected
  candidate using the casing stored in the buffer.
- Local suggestions appear immediately while ordinary LSP completion requests
  stay debounced so typing does not block on language-server work.
- Leaving Insert mode groups that whole insert session into one undo step,
  matching Vim-style undo behavior.

## Command Mode

Executes editor commands typed after `:`.
See [Commands](./commands.md) for a command reference.

### Commands

| Key | Description | Command |
| --- | --- | --- |
| `:w` | Save the file. | `:w` |
| `:q` | Quit the editor. | `:q` |
| `:q!` | Quit without saving. | `:q!` |
| `:wq` | Save and quit. | `:wq` |
| `:undo` | Undo the most recent change. | `:undo` |
| `:redo` | Redo the most recently undone change. | `:redo` |
| `:rename {new_name}` | Request an LSP rename for the symbol under the cursor. | `:rename {new_name}` |
| `:diagnostics` | Open the active-buffer diagnostics picker. | `:diagnostics` |
| `:next-diagnostic` | Jump to the next active-buffer diagnostic. | `:next-diagnostic` |
| `:prev-diagnostic` | Jump to the previous active-buffer diagnostic. | `:prev-diagnostic` |
| `:{number}` | Jump to a line. | `:{number}` |

### Prompt editing

| Key | Description | Command |
| --- | --- | --- |
| `Ctrl+A`, `Home` | Move the input cursor to the start. | `move-input-start` |
| `Ctrl+E`, `End` | Move the input cursor to the end. | `move-input-end` |
| `Up` / `Down` | Traverse command-history entries matching the typed prefix. | `prompt-history-prev`, `prompt-history-next` |
| `Ctrl+P` / `Ctrl+N` | Traverse the full command history. | `prompt-history-prev-full`, `prompt-history-next-full` |
| `Ctrl+B`, `Left` | Move the input cursor left. | `move-input-left` |
| `Ctrl+F`, `Right` | Move the input cursor right. | `move-input-right` |
| `Alt+B` | Move the input cursor one Vim-style word left. | `move-input-word-left` |
| `Alt+F` | Move the input cursor one Vim-style word right. | `move-input-word-right` |
| `Ctrl+W` | Delete the previous Vim-style word. | `delete-input-word-backward` |
| `Ctrl+U` | Delete from the cursor to the start of the input. | `delete-input-to-start` |
| `Ctrl+K` | Delete from the cursor to the end of the input. | `delete-input-to-end` |
| `Ctrl+H` or `Backspace` | Delete the character before the cursor. | `delete-input-char` |
| `Ctrl+D` or `Delete` | Delete the character under the cursor. | `delete-input-char-forward` |
| `Esc` | Cancel command input. | `cancel-command` |

Ordex keeps a separate session-local `:` history, ignores empty submissions,
deduplicates adjacent duplicates, and caps the history at `999999` entries.

## Search Mode

Find text in the buffer.

| Key | Description | Command |
| --- | --- | --- |
| `/pattern` then `Enter` | Find the next occurrence. | `enter-search-mode` + `execute-command` |
| `n` | Repeat the search forward. | `search-next` |
| `N` | Repeat the search backward. | `search-previous` |
| `Up` / `Down` | Traverse search-history entries matching the typed prefix. | `prompt-history-prev`, `prompt-history-next` |
| `Ctrl+P` / `Ctrl+N` | Traverse the full search history. | `prompt-history-prev-full`, `prompt-history-next-full` |
| `Esc` | Cancel search input. | `cancel-command` |

- Search is case-sensitive and literal.
- Search wraps to the beginning of the file.
- Search input supports the same inline editing key bindings as command mode.
- Ordex keeps search history separate from command history, with the same
  session-local retention, adjacent-deduplication, and `999999` entry cap.
