# Configuration

Ordex can load a configuration file from the default XDG location:

- `$XDG_CONFIG_HOME/ordex/config.cfg` (when `XDG_CONFIG_HOME` is set)
- `$HOME/.config/ordex/config.cfg` (fallback)

You can also pass an explicit file path with `--config`.

```bash
ordex --config /path/to/ordex.cfg [file]
```

## Format

The format is TOML-like:

- Sections use `[section]` headers
- Keys use `key = value`
- String values use double quotes
- Arrays of string values are supported
- Integer and boolean values are supported
- `#` starts a comment when outside quoted strings

Example:

```toml
[editor]
scroll_margin = 2
horizontal_scroll_margin = 4

[keymap.normal]
z = "move-right"

[include]
extra = "extra.cfg"
```

## Supported Settings

### `[editor]`

- `scroll_margin` = non-negative integer
- `horizontal_scroll_margin` = non-negative integer

### `[keymap.<mode>]`

Modes: `normal`, `insert`, `command`, `search`.

Each key is a key name and each value is either an action string or an array of
action strings:

```toml
[keymap.normal]
h = "move-left"
l = "move-right"
z = ["move-down", "move-right"]
zu = "move-down"
```

Action names are case-sensitive and use kebab-case. For example, use
`move-down`, `page-up`, and `save-current-file`, not `MoveDown`, `move_down`,
or `movedown`.

Valid action names:

| Navigation |  |  |  |
| --- | --- | --- | --- |
| `move-left` | `move-right` | `move-up` | `move-down` |
| `move-word-forward` | `move-word-backward` | `move-word-end` | `move-paragraph-forward` |
| `move-paragraph-backward` | `move-line-start` | `move-line-end` | `move-past-line-end` |
| `move-first-non-blank` | `move-to-first-line` | `move-to-last-line` | `page-up` |
| `page-down` | `half-page-up` | `half-page-down` | `find-forward` |
| `find-backward` | `till-forward` | `till-backward` | `repeat-find-forward` |
| `repeat-find-backward` |  |  |  |

| Mode and file actions |  |  |  |
| --- | --- | --- | --- |
| `enter-insert-mode` | `insert-after-cursor` | `open-line-below` | `open-line-above` |
| `enter-command-mode` | `enter-search-mode` | `exit-to-normal-mode` | `search-next` |
| `search-previous` | `save-current-file` | `save-current-file-and-quit` |  |

| Editing actions |  |  |  |
| --- | --- | --- | --- |
| `delete-char-backward` | `delete-char-forward` | `delete-char-at-cursor` | `delete-word-backward` |
| `delete-to-line-start` | `insert-newline` | `change-inner-word` | `delete-inner-word` |
| `delete-around-paren` |  |  |  |

| Command/search input actions |  |  |  |
| --- | --- | --- | --- |
| `execute-command` | `cancel-command` | `delete-input-char` | `delete-input-char-forward` |
| `delete-input-word-backward` | `delete-input-to-start` | `delete-input-to-end` | `move-input-start` |
| `move-input-end` | `move-input-left` | `move-input-right` | `move-input-word-left` |
| `move-input-word-right` |  |  |  |

Array-valued bindings execute each action in order. This works for both direct
bindings and multi-key sequences:

```toml
[keymap.normal]
z = ["move-down", "move-right"]
zu = ["move-down", "move-right"]
```

If you use a numeric count prefix before a multi-action binding, Ordex repeats
the whole sequence. For example, `3z` runs `move-down`, `move-right`,
`move-down`, `move-right`, `move-down`, `move-right`.

If any action name in an array is invalid, Ordex ignores the entire binding and
prints the usual startup warning.

Key examples:

- single character: `z`
- single Unicode character: `é`
- control: `ctrl-f`
- alt: `alt-b`
- modified named keys: `ctrl-home`, `ctrl-left`, `alt-right`, `shift-tab`
- use `-` between modifier and key; `+` is not supported
- named keys: `space`, `left`, `right`, `up`, `down`, `home`, `end`, `pageup`, `pagedown`, `delete`
- multi-key sequences are supported (for example `zu`)

### `[include]`

Each value in this section should be a string path to another config file:

```toml
[include]
extra = "keymaps.cfg"
```

Relative paths are resolved from the main config file directory. Included files are
loaded after the entire main config file has been parsed, so included values
overwrite main-file values even when the main file sets them after the `[include]`
section.

## Resilience Behavior

- Unknown sections/keys are ignored with warnings
- Invalid values default safely with warnings
- Missing include files are recoverable with warnings
- Duplicate key definitions use last-definition-wins with warning
- Valid key mappings remain active even if unrelated sections fail
- Startup warnings are printed before the TUI is opened, including source location and line content
