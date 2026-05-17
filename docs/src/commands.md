# Commands

Commands are entered from normal mode by pressing `:`.
While typing a command, inline editing shortcuts are available (`Ctrl+A/E/B/F/W/U/K/H/D`, `Alt+B/F`, arrow keys, and Home/End). `Up` / `Down` traverse history entries matching the typed prefix, while `Ctrl+P` / `Ctrl+N` traverse the full session-local command history.

| Command | Effect | Example |
| --- | --- | --- |
| `:w` | Save current buffer to disk, ensuring the file ends with a newline | `:w` |
| `:e {path}` | Open another buffer for a path and switch to it | `:e notes.txt` |
| `:new` | Open a new unnamed buffer in the current single-pane editor | `:new` |
| `:bn` | Switch to the next open buffer | `:bn` |
| `:bp` | Switch to the previous open buffer | `:bp` |
| `:ls` | List open buffers on the message line | `:ls` |
| `:bd` | Close the active buffer; prompts when it has unsaved changes | `:bd` |
| `:q` | Quit editor; prompts to save when there are unsaved changes | `:q` |
| `:q!` | Quit immediately without saving | `:q!` |
| `:wq` | Save, then quit | `:wq` |
| `:wall` / `:wa` | Save every modified named buffer and restore the original active buffer | `:wall` |
| `:x` | Save the current file only when modified, then quit | `:x` |
| `:undo` / `:u` | Undo the most recent change | `:u` |
| `:redo` / `:red` | Redo the most recently undone change | `:red` |
| `:rename {new_name}` / `:ren {new_name}` | Rename the LSP symbol under the cursor | `:ren helper_total` |
| `:diagnostics` / `:dia` | Open the active-buffer diagnostics picker | `:dia` |
| `:next-diagnostic` / `:dn` | Jump to the next diagnostic in the active buffer | `:dn` |
| `:prev-diagnostic` / `:dp` | Jump to the previous diagnostic in the active buffer | `:dp` |
| `:reload-config` / `:rc` | Reload the active config file from disk | `:rc` |
| `:save-session {name}` / `:ss {name}` | Save the current project session under a name | `:ss my-worktree` |
| `:open-session {name}` / `:os {name}` | Reopen a named project session and restore its working directory | `:os my-worktree` |
| `:delete-session {name}` / `:ds {name}` | Delete a named project session from disk | `:ds my-worktree` |
| `:{number}` | Jump to a line number | `:1`, `:50` |

Long-form aliases are also available: `:edit`, `:buffer-next`, `:buffer-prev`,
`:buffers`, and `:buffer-delete`. Short aliases are available for most longer
commands, including `:cq`, `:up`, `:u`, `:red`, `:ren`, `:dia`, `:dn`, `:dp`,
`:rc`, `:ss`, `:os`, and `:ds`. `:wa` is an alias for `:wall`.

LSP rename applies the returned workspace edit directly in Ordex. Open buffers are
updated in memory, and unopened files touched by the rename are opened as buffers
and edited there instead of being written on disk immediately.

`<Space>a` opens an LSP code-action picker for the current cursor context. Ordex
applies supported edit-bearing actions through the same workspace-edit path as
rename, and command-driven or resource-operation actions are unsupported.

When the active language server publishes diagnostics, Ordex stores them per file, renders
gutter markers plus curly underlines for the active buffer, and exposes them
through `:diagnostics`, `:next-diagnostic`, and `:prev-diagnostic`.

Open buffers also appear in the persistent top-row tab strip, which follows the
same open-buffer order as `:bn` and `:bp`. The buffer switcher pins the active
buffer first and then prefers recently visited named buffers.
