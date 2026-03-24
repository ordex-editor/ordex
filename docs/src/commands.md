# Commands

Commands are entered from normal mode by pressing `:`.
While typing a command, inline editing shortcuts are available (`Ctrl+A/E/B/F/W/U/K/H/D`, `Alt+B/F`, arrow keys, and Home/End).

| Command | Effect | Example |
| --- | --- | --- |
| `:w` | Save current buffer to disk | `:w` |
| `:q` | Quit editor; prompts to save when there are unsaved changes | `:q` |
| `:q!` | Quit immediately without saving | `:q!` |
| `:wq` | Save, then quit | `:wq` |
| `:undo` | Undo the most recent change | `:undo` |
| `:redo` | Redo the most recently undone change | `:redo` |
| `:reload-config` | Reload the active config file from disk | `:reload-config` |
| `:{number}` | Jump to a line number | `:1`, `:50` |
