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
- Integer and boolean values are supported
- `#` starts a comment when outside quoted strings

Example:

```toml
[editor]
scroll_margin = 2
horizontal_scroll_margin = 4

[keymap.normal]
z = "MoveRight"

[include]
extra = "extra.cfg"
```

## Supported Settings

### `[editor]`

- `scroll_margin` = non-negative integer
- `horizontal_scroll_margin` = non-negative integer

### `[keymap.<mode>]`

Modes: `normal`, `insert`, `command`, `search`.

Each key is a key name and each value is an action string:

```toml
[keymap.normal]
h = "MoveLeft"
l = "MoveRight"
```

Key examples:

- single character: `z`
- named space key: `space`
- control: `ctrl-f`
- alt: `alt-b`
- named keys: `left`, `right`, `up`, `down`, `home`, `end`, `pageup`, `pagedown`, `delete`

### `[include]`

Each key in this section should be a string path to another config file:

```toml
[include]
extra = "keymaps.cfg"
```

Relative paths are resolved from the main config file directory.

## Resilience Behavior

- Unknown sections/keys are ignored with warnings
- Invalid values default safely with warnings
- Missing include files are recoverable with warnings
- Duplicate key definitions use last-definition-wins with warning
- Valid key mappings remain active even if unrelated sections fail
