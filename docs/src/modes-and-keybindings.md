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
| `h` | Move left. | `move-left` |
| `j` | Move down. | `move-down` |
| `k` | Move up. | `move-up` |
| `l` | Move right. | `move-right` |
| `w` | Move to the next word. | `move-word-forward` |
| `W` | Move to the next WORD. | `move-big-word-forward` |
| `b` | Move to the previous word. | `move-word-backward` |
| `B` | Move to the previous WORD. | `move-big-word-backward` |
| `e` | Move to the end of the current or next word. | `move-word-end` |
| `E` | Move to the end of the current or next WORD. | `move-big-word-end` |
| `ge` | Move to the end of the previous word. | `move-word-end-backward` |
| `gE` | Move to the end of the previous WORD. | `move-big-word-end-backward` |
| `_` | Move down `count - 1` lines, then land on the first non-blank character. Without a count, stay on the current line. | `move-down-first-non-blank` |
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
| `gd` (LSP) | Go to the symbol definition. | `goto-definition` |
| `gr` (LSP) | Go to one symbol reference. | `goto-references` |
| `gf` | Open the filename-like token under the cursor relative to the current buffer or working directory. | `goto-file-under-cursor` |
| `gF` | Open the filename-like token under the cursor and jump to a trailing `:line[:column]` when present. | `goto-file-under-cursor-at-position` |
| `ga` | Jump to the most recently visited named buffer that is still open. | `goto-alternate-file` |
| `g.` | Jump to the cursor position after the most recently committed change in the session. | `goto-last-modification` |
| `<Space>a` (LSP) | Open a code-action picker for the current cursor context, even when only one supported action is available. | `open-code-actions` |
| `<Space>r` (LSP) | Prefill command mode with `rename {current_symbol}` using the active syntax profile's identifier rules. | `prompt-rename-symbol` |
| `K` (LSP) | Show hover information for the symbol under the cursor. | `show-hover` |
| `<Space>d` (LSP) | Open the active-buffer diagnostics picker. | `open-diagnostics-picker` |
| `]d` (LSP) | Jump to the next active-buffer diagnostic. | `next-diagnostic` |
| `[d` (LSP) | Jump to the previous active-buffer diagnostic. | `prev-diagnostic` |
| `zt` | Place the current cursor row near the top of the viewport, respecting `scroll_margin`. | `align-viewport-top` |
| `zz` | Place the current cursor row near the center of the viewport. | `align-viewport-center` |
| `zb` | Place the current cursor row near the bottom of the viewport, respecting `scroll_margin`. | `align-viewport-bottom` |
| `Ctrl+F` | Page forward. | `page-down` |
| `Ctrl+B` | Page backward. | `page-up` |
| `Ctrl+D` | Half-page forward. | `half-page-down` |
| `Ctrl+U` | Half-page backward. | `half-page-up` |
| `<Space>b` | Open a buffer-switch picker with fuzzy subsequence filtering over open buffers, pinning the active buffer first and preferring recently visited named buffers after it. | `open-buffer-switcher` |
| `<Space>f` | Open a file picker with fuzzy subsequence filtering over files under the current working directory. | `open-file-picker` |
| `<Space>/` | Open command mode prefilled with `:grep ` to start an async content search. | `prompt-grep` |
| `<Space>*` | Run a whole-word `:grep` for the identifier under the cursor, or for the next same-line identifier when the cursor is on whitespace or punctuation, and open the file search picker immediately. | `grep-word-under-cursor` |
| `:` | Enter command mode. | `enter-command-mode` |
| `/` | Enter search mode. | `enter-search-mode` |
| `<Space>l` | Hide committed search highlighting until the next search action reveals it. | `hide-search-highlighting` |
| `n` | Jump to the next search occurrence. | `search-next` |
| `N` | Jump to the previous search occurrence. | `search-previous` |

### Editing, paste, undo, and macros

| Key | Description | Command |
| --- | --- | --- |
| `d{motion}` | Delete with a supported motion or text object. | `begin-delete-operator` + operator motion |
| `c{motion}` | Change with a supported motion or text object. | `begin-change-operator` + operator motion |
| `y{motion}` | Yank with a supported motion or text object. | `begin-yank-operator` + operator motion |
| `={motion}` | Reindent with a supported motion or text object. | `begin-reindent-operator` + operator motion |
| `==` | Reindent the current line. | `begin-reindent-operator` |
| `>{motion}` | Indent by one configured shift width with a supported motion or text object. | `begin-indent-operator` + operator motion |
| `>>` | Indent the current line by one configured shift width. | `begin-indent-operator` |
| `<{motion}` | Dedent by one configured shift width with a supported motion or text object. | `begin-dedent-operator` + operator motion |
| `<<` | Dedent the current line by one configured shift width. | `begin-dedent-operator` |
| `i` | Enter Insert mode. | `enter-insert-mode` |
| `a` | Move right, then enter Insert mode. | `insert-after-cursor` |
| `dd` | Delete the current line. | — |
| `cc` | Change the current line and enter Insert mode. | — |
| `yy` | Yank the current line. | `yank-current-line` |
| `p` | Paste after the cursor, or below the current line for linewise yanks. | `paste-after-cursor` |
| `P` | Paste before the cursor, or above the current line for linewise yanks. | `paste-before-cursor` |
| `"+{command}` | Route the next yank, delete, change, or paste command through the system clipboard register. | — |
| `"*{command}` | Route the next yank, delete, change, or paste command through the primary-selection register. | — |
| `<Space>c` | Toggle line comments on the current line or counted line range. In languages without line comments, it falls back to the ordinary block-comment style line-by-line. | `toggle-line-comment` |
| `<Space>C` | Toggle one block comment around the current line or counted line range. | `toggle-block-comment` |
| `<Space>p` | Paste from the `"+` clipboard register after the cursor. | `paste-clipboard-after-cursor` |
| `<Space>P` | Paste from the `"+` clipboard register before the cursor. | `paste-clipboard-before-cursor` |
| `<Space>y` | In Normal mode, start a `"+` yank operator for the next motion/text object. In Visual modes, yank the active selection into `"+`. | `yank-clipboard` |
| `x` | Delete the character under the cursor. | `delete-char-at-cursor` |
| `D` | Delete from the cursor through the end of the current line. | `delete-to-line-end` |
| `C` | Change from the cursor through the end of the current line and enter Insert mode. | `change-to-line-end` |
| `~` | Toggle the case of the character under the cursor and advance. | `toggle-case-at-cursor` |
| `J` | Join the current line with the next line, trimming the next line's leading indentation. Counts join additional following lines. | `join-lines` |
| `r{char}` | Replace the character under the cursor with `{char}`. Counts replace additional characters on the same line. | `begin-replace-char` |
| `*` | Search forward for the next whole-word match of the identifier under the cursor, or of the next same-line identifier when the cursor is on whitespace or punctuation. | `search-word-under-cursor` |
| `Ctrl+A` | Increment the next decimal number on the current line. Counts increase the delta. | `increment-next-number` |
| `Ctrl+X` | Decrement the next decimal number on the current line. Counts increase the delta. | `decrement-next-number` |
| `Ctrl+L` | Force a full redraw of the screen. | `request-full-redraw` |
| `.` | Repeat the last change, including counted normal-mode edits and completed insert, change, and open-line sessions. | `repeat-last-change` |
| `q{register}` | Start recording into lowercase register `{register}`. Press `q` again in Normal mode to stop. | `begin-macro-record` |
| `@{register}` | Replay the contents of lowercase register `{register}`. | `begin-macro-playback` |
| `@@` | Replay the most recently replayed macro again. | `begin-macro-playback` |
| `u` | Undo the most recent change. | `undo` |
| `Ctrl+R` | Redo the most recently undone change. | `redo` |
| `<Space>w` | Save the current file. | `save-current-file` |
| `<Space>q` | Save the current file and quit. | `save-current-file-and-quit` |

### Operator examples

| Key | Description | Command |
| --- | --- | --- |
| `dw` | Delete by moving to the next word. | `begin-delete-operator` + `word-forward` |
| `de` | Delete by moving to the end of the current word. | `begin-delete-operator` + `word-end` |
| `db` | Delete by moving to the previous word. | `begin-delete-operator` + `word-backward` |
| `dW` | Delete by moving to the next WORD. | `begin-delete-operator` + `big-word-forward` |
| `dE` | Delete by moving to the end of the current WORD. | `begin-delete-operator` + `big-word-end` |
| `dB` | Delete by moving to the previous WORD. | `begin-delete-operator` + `big-word-backward` |
| `cw` | Change by moving to the next word. | `begin-change-operator` + `word-forward` |
| `cE` | Change by moving to the end of the current WORD. | `begin-change-operator` + `big-word-end` |
| `yw` | Yank by moving to the next word. | `begin-yank-operator` + `word-forward` |
| `ye` | Yank by moving to the end of the current word. | `begin-yank-operator` + `word-end` |
| `df{char}` | Delete using a forward line-local search. | `begin-delete-operator` + `find-forward` |
| `dF{char}` | Delete using a backward line-local search. | `begin-delete-operator` + `find-backward` |
| `dt{char}` | Delete until just before a forward line-local match. | `begin-delete-operator` + `till-forward` |
| `dT{char}` | Delete until just after a backward line-local match. | `begin-delete-operator` + `till-backward` |
| `cf{char}` | Change using a forward line-local search. | `begin-change-operator` + `find-forward` |
| `ct{char}` | Change until just before a forward line-local match. | `begin-change-operator` + `till-forward` |
| `yf{char}` | Yank using a forward line-local search. | `begin-yank-operator` + `find-forward` |
| `d%` | Delete through the matching delimiter resolved by `%`. | `begin-delete-operator` + `jump-to-matching-delimiter` |
| `c%` | Change through the matching delimiter resolved by `%`. | `begin-change-operator` + `jump-to-matching-delimiter` |
| `y%` | Yank through the matching delimiter resolved by `%`. | `begin-yank-operator` + `jump-to-matching-delimiter` |
| `diw` | Delete the inner word. | `begin-delete-operator` + `text-object-inner` + `word-forward` |
| `ciw` | Change the inner word, then enter Insert mode. | `begin-change-operator` + `text-object-inner` + `word-forward` |
| `da(` | Delete the smallest surrounding balanced `(...)` region. | `begin-delete-operator` + `text-object-around` |
| `=iw` | Reindent the lines touched by the current text object. | `begin-reindent-operator` + `text-object-inner` + `word-forward` |
| `>iw` | Indent the lines touched by the current text object by one configured shift width. | `begin-indent-operator` + `text-object-inner` + `word-forward` |

### Normal-mode behavior notes

- Jump history records LSP definition and reference jumps, `gf` / `gF`,
  search-result jumps, `gg` / `G` / `:{number}`, and diagnostic jumps.
- Plain local motions such as `h`, `j`, `k`, `l`, `w`, and `b` do not create
  jump-history entries.
- `/` highlights all visible matches live while you type, keeps the cursor fixed
  until `Enter`, and restores the previous committed highlights if you cancel
  with `Esc`.
- `<Space>l` hides only committed search highlights; `n`, `N`, `*`, and a new
  `/` search reuse the last query and show the highlights again.
- `:s<delim>pattern<delim>replacement<delim>` replaces every regex match on the
  current line.
- `:%s<delim>pattern<delim>replacement<delim>` replaces every regex match in the
  current buffer.
- Valid `:s` and `:%s` input previews replacement text live while you type,
  recenters the viewport on the first affected match, and keeps the logical
  cursor fixed until `Enter` commits or `Esc` cancels.
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
- Counts before `J`, `~`, `Ctrl+A`, `Ctrl+X`, `_`, `W`, `B`, `E`, and `r{char}`
  reuse the same Normal-mode count parsing as other motions and edits.
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
- `"+` and `"*` apply only to the next yank, delete, change, or paste command.
- `"+` targets the system clipboard, while `"*` targets the primary selection.
- On Wayland, `"*` shows an explicit error when the clipboard tool cannot access
  a distinct primary selection.
- Terminal bracketed paste inserts the full payload as text in Insert mode,
  pastes text after the cursor in Normal mode instead of replaying pasted bytes
  as commands, and replaces the active Visual selection before returning to
  Normal mode with the pasted text. Command and Search prompts accept only the
  first pasted line.
- The buffer-switch picker keeps the active buffer unchanged while you move
  through matches, then switches only after `Enter`. `Esc` cancels the picker.
- With an empty buffer-switch query, the active buffer stays pinned first and
  other named buffers follow the same recent-access history used by `ga`.
- The top tab strip stays visible while switching buffers and follows the same
  buffer order as `:bn` / `:bp`.
- On wide terminals, the buffer-switch picker, file picker, and multi-target LSP
  definition/reference pickers show a right-side syntax-highlighted content
  preview for the selected row.
- The file picker scans the working directory asynchronously, streams partial
  results as they arrive, includes hidden paths, and respects `.gitignore` when
  Ordex is running inside a Git work tree.
- The file picker matches against both basenames and relative paths, opens the
  highlighted file on `Enter`, and cancels on `Esc`.
- File-picker and LSP picker previews load from disk asynchronously when needed,
  while already-open files use the live in-memory buffer so unsaved edits remain
  visible in the preview.
- `:grep` and `<Space>/` open an async content-search picker that streams
  results, fuzzy-filters them with the picker query, skips hidden and ignored
  files by default, lists one entry per matching line, and centers the
  destination line after `Enter`.
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

Characterwise, linewise, and blockwise selection reuse the existing normal-mode
motion set. Blockwise selections are rectangular, truncate to real characters on
short lines, and do not extend through virtual spaces past end-of-line.

| Key | Description | Command |
| --- | --- | --- |
| `v` | Enter characterwise Visual mode. | `enter-visual-mode` |
| `V` | Enter linewise Visual mode. | `enter-visual-line-mode` |
| `Ctrl-V` | Enter blockwise Visual mode. | `enter-visual-block-mode` |
| `gv` | Recreate the most recent visual selection from Normal mode. | `recreate-last-selection` |
| Most Normal-mode motions and counts | Continue to work while adjusting the selection. | — |
| `o` | Swap the active cursor with the opposite end of the selection. | `swap-visual-anchor` |
| `d` | Delete the active selection and return to Normal mode. | `delete-selection` |
| `c` | Delete the active selection and enter Insert mode. | `change-selection` |
| `"+y`, `"+d`, `"+c` | Apply the Visual command and also target the `"+` clipboard register. | — |
| `"*y`, `"*d`, `"*c` | Apply the Visual command and also target the `"*` primary-selection register. | — |
| `<Space>c` | Toggle line comments on every line touched by the active selection and return to Normal mode. In languages without line comments, it falls back to line-by-line block comments. | `toggle-line-comment` |
| `<Space>C` | Toggle one block comment around the active selection and return to Normal mode. Characterwise selections wrap the selected span; linewise and blockwise selections wrap the touched lines once. | `toggle-block-comment` |
| `I` | In blockwise Visual mode, enter Insert mode and mirror text at the left edge of the selected block on every touched line. | `visual-insert-block-start` |
| `A` | In blockwise Visual mode, enter Insert mode and mirror text just after the right edge of the selected block on every touched line. | `visual-append-block-end` |
| `y` | Yank the active selection and return to Normal mode. | `yank-selection` |
| `=` | Reindent every line touched by the active selection and return to Normal mode. | `reindent-selection` |
| `>` | Indent every line touched by the active selection by one configured shift width and return to Normal mode. | `indent-selection` |
| `<` | Dedent every line touched by the active selection by one configured shift width and return to Normal mode. | `dedent-selection` |
| `Esc` | Cancel the selection and return to Normal mode. | `exit-to-normal-mode` |

Blockwise Visual mode mirrors `I` / `A` across every touched line while keeping
one real cursor on the first touched line. Characterwise and linewise Visual
mode leave `I` / `A` unavailable.

Multi-key discovery popups also appear for Visual-mode sequences such as `gg`,
`g$`, `g0`, `zt`, `zz`, and `zb`.

## Insert Mode

Text entry mode.

| Key | Description | Command |
| --- | --- | --- |
| Printable characters | Insert text at the cursor. | — |
| `Up` | Move to the previous completion candidate while the popup is visible. Moving back to no selection restores the typed prefix. | `completion-select-up` |
| `Down` | Move to the next completion candidate while the popup is visible. Moving back to no selection restores the typed prefix. | `completion-select-down` |
| `Ctrl+P` | Alternate shortcut for the previous completion suggestion. | `completion-select-up` |
| `Ctrl+N` | Alternate shortcut for the next completion suggestion. | `completion-select-down` |
| `Backspace` | Delete the character before the cursor. | `delete-char-backward` |
| `Ctrl+T` | Indent the current line by one configured shift width. | `indent-current-line` |
| `Ctrl+D` | Dedent the current line by one configured shift width. | `dedent-current-line` |
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

| Command | Description |
| --- | --- |
| `:w` | Save the file. |
| `:wall` / `:wa` | Save every modified named buffer. |
| `:q` | Quit the editor. |
| `:q!` | Quit without saving. |
| `:wq` | Save and quit. |
| `:x` | Save only when modified, then quit. |
| `:new` | Open a new unnamed buffer. |
| `:undo` | Undo the most recent change. |
| `:redo` | Redo the most recently undone change. |
| `:rename {new_name}` (LSP) | Request an LSP rename for the symbol under the cursor. |
| `:diagnostics` (LSP) | Open the active-buffer diagnostics picker. |
| `:next-diagnostic` (LSP) | Jump to the next active-buffer diagnostic. |
| `:prev-diagnostic` (LSP) | Jump to the previous active-buffer diagnostic. |
| `:{number}` | Jump to a line. |

### Prompt editing

| Key | Description | Command |
| --- | --- | --- |
| `Ctrl+A`, `Home` | Move the input cursor to the start. | `move-input-start` |
| `Ctrl+E`, `End` | Move the input cursor to the end. | `move-input-end` |
| `Up` | Traverse command-history entries matching the typed prefix toward older entries. | `prompt-history-prev` |
| `Down` | Traverse command-history entries matching the typed prefix toward newer entries. | `prompt-history-next` |
| `Ctrl+P` | Traverse the full command history toward older entries. | `prompt-history-prev-full` |
| `Ctrl+N` | Traverse the full command history toward newer entries. | `prompt-history-next-full` |
| `Tab`, `Ctrl+I` | Cycle forward through command completions while leaving `Enter` bound to command execution. | `command-completion-next` |
| `Shift+Tab` | Cycle backward through command completions. | `command-completion-prev` |
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
Command completion appears automatically for command names plus supported file-path
and session-name arguments, packs multiple suggestions onto one popup row when
space allows, and previews the highlighted entry directly in the prompt.

## Search Mode

Find text in the buffer.

| Key | Description | Command |
| --- | --- | --- |
| `/pattern` then `Enter` | Find the next occurrence. | `enter-search-mode` + `execute-command` |
| `n` | Repeat the search forward. | `search-next` |
| `N` | Repeat the search backward. | `search-previous` |
| `Up` | Traverse search-history entries matching the typed prefix toward older entries. | `prompt-history-prev` |
| `Down` | Traverse search-history entries matching the typed prefix toward newer entries. | `prompt-history-next` |
| `Ctrl+P` | Traverse the full search history toward older entries. | `prompt-history-prev-full` |
| `Ctrl+N` | Traverse the full search history toward newer entries. | `prompt-history-next-full` |
| `Esc` | Cancel search input. | `cancel-command` |

- Search is case-sensitive and literal.
- Search wraps to the beginning of the file.
- Search input supports the same inline editing key bindings as command mode.
- Ordex keeps search history separate from command history, with the same
  session-local retention, adjacent-deduplication, and `999999` entry cap.
