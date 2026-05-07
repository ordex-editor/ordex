# Commands

Commands are entered from normal mode by pressing `:`.
While typing a command, inline editing shortcuts are available (`Ctrl+A/E/B/F/W/U/K/H/D`, `Alt+B/F`, arrow keys, and Home/End). `Up` / `Down` traverse history entries matching the typed prefix, while `Ctrl+P` / `Ctrl+N` traverse the full session-local command history.

| Command | Effect | Example |
| --- | --- | --- |
| `:w` | Save current buffer to disk | `:w` |
| `:e {path}` | Open another buffer for a path and switch to it | `:e notes.txt` |
| `:bn` | Switch to the next open buffer | `:bn` |
| `:bp` | Switch to the previous open buffer | `:bp` |
| `:ls` | List open buffers on the message line | `:ls` |
| `:bd` | Close the active buffer; prompts when it has unsaved changes | `:bd` |
| `:q` | Quit editor; prompts to save when there are unsaved changes | `:q` |
| `:q!` | Quit immediately without saving | `:q!` |
| `:wq` | Save, then quit | `:wq` |
| `:undo` | Undo the most recent change | `:undo` |
| `:redo` | Redo the most recently undone change | `:redo` |
| `:rename {new_name}` | Rename the LSP symbol under the cursor | `:rename helper_total` |
| `:diagnostics` | Open the active-buffer diagnostics picker | `:diagnostics` |
| `:next-diagnostic` | Jump to the next diagnostic in the active buffer | `:next-diagnostic` |
| `:prev-diagnostic` | Jump to the previous diagnostic in the active buffer | `:prev-diagnostic` |
| `:reload-config` | Reload the active config file from disk | `:reload-config` |
| `:save-session {name}` | Save the current project session under a name | `:save-session my-worktree` |
| `:open-session {name}` | Reopen a named project session and restore its working directory | `:open-session my-worktree` |
| `:delete-session {name}` | Delete a named project session from disk | `:delete-session my-worktree` |
| `:{number}` | Jump to a line number | `:1`, `:50` |

Long-form aliases are also available: `:edit`, `:buffer-next`, `:buffer-prev`,
`:buffers`, and `:buffer-delete`.

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
