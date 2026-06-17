# Configuration

Ordex can load a configuration file from the default XDG location:

- `$XDG_CONFIG_HOME/ordex/config.cfg` (when `XDG_CONFIG_HOME` is set)
- `$HOME/.config/ordex/config.cfg` (fallback)

You can also pass an explicit file path with `--config`.

```bash
ordex --config /path/to/ordex.cfg [file]
```

After startup, use `:reload-config` to re-read the active config file without
restarting the editor. If Ordex was started without an active config path, the
command reports that no config file is available to reload.

## Format

The format is TOML-like:

Ordex syntax-highlights these `.cfg` files when you open them in the editor.

- Sections use `[section]` headers
- Keys use `key = value`
- String values use double quotes
- Arrays of string values are supported, including multiline arrays
- Integer and boolean values are supported
- `#` starts a comment when outside quoted strings

Example:

```toml
[editor]
soft_wrap = false
auto_reload_external_changes = true
scroll_margin = 2
horizontal_scroll_margin = 4
indent_width = 4
indent_with_tabs = false
tab_width = 8
file_picker_max_files = 1000000
relative_line_numbers = true
visible_whitespace = ["nbsp", "tab", "trailing-space"]
theme = "bogster"

[keymap.normal]
z = "move-right"

[keymap.operator]
é = "word-forward"

[include]
extra = "extra.cfg"
```

## Supported Settings

### `[editor]`

| Setting | Value | Default | Description |
| --- | --- | --- | --- |
| `scroll_margin` | non-negative integer | `3` | Keeps a vertical margin around the cursor when the viewport scrolls. |
| `horizontal_scroll_margin` | non-negative integer | `5` | Keeps a horizontal margin around the cursor when horizontal scrolling is active. |
| `relative_line_numbers` | boolean | `false` | When `true`, Ordex keeps the current line's absolute number in the gutter and shows relative distances for the surrounding lines. |
| `soft_wrap` | boolean | `true` | When enabled, long lines are shown across multiple screen rows, `j` / `k` move by wrapped screen rows, and horizontal scrolling is disabled. Set `soft_wrap = false` to keep long lines on one screen row and re-enable horizontal scrolling. |
| `auto_reload_external_changes` | boolean | `true` | When `true`, Ordex automatically reloads clean file-backed buffers after on-disk changes and defers the notice for hidden buffers until you activate them. Set it to `false` to ask before reloading even clean buffers. |
| `indent_width` | positive integer | `4` | Shift-width commands such as `>>`, `<<`, Visual `>` / `<`, and Insert-mode `Ctrl+T` / `Ctrl+D` treat this value as one indentation step. Language-aware reindent commands such as `==` and Visual `=` also use it when rebuilding indentation prefixes. |
| `indent_with_tabs` | boolean | `false` | When `true`, manual indent emits tabs for full indentation steps and uses spaces only for any remaining columns. |
| `tab_width` | integer between `1` and `9999` | `8` | Display width of the tab character. |
| `file_picker_max_files` | positive integer | `1000000` | Ordex stops collecting additional paths after that many file-picker entries so very large trees do not grow memory usage without bound. |
| `sequence_discovery_popup` | boolean | `true` | Set this to `false` to disable the shortcut-discovery overlay for pending multi-key sequences. |
| `visible_whitespace` | `"all"`, `"none"`, token, or array of tokens | `"none"` | Highlights special characters in main buffer content. Supported tokens: `"nbsp"` (shows `⍽`), `"tab"` (shows `▸` while preserving tab width), `"trailing-space"` (shows trailing ASCII spaces as `·`). |
| `theme` | string theme name | `bogster` | Selects the bundled theme used for syntax highlighting and broader UI surfaces such as the gutter, current-line highlight, status line, message line, and sequence-discovery popup. Theme changes are picked up by `:reload-config`. |

Ordex ships these bundled themes:

- `bogster`
- `catppuccin-latte`
- `catppuccin-frappe`
- `catppuccin-macchiato`
- `catppuccin-mocha`
- `gruvbox`
- `kanagawa`
- `nord`
- `onedark`
- `tokyonight`

Ordex supports both 256-color and truecolor terminals. By default it renders
through the xterm 256-color palette; set `ORDEX_TRUECOLOR=1` to opt into
24-bit output, or use a terminal that advertises direct color through `TERM`.

### `[keymap.<mode>]`

Modes: `normal`, `visual`, `insert`, `command`, `search`.

Each key is a key name and each value is either:

- an action string
- an `@`-prefixed replay string that replays typed keys through the normal input pipeline
- an array of action strings

Examples:

```toml
[keymap.normal]
h = "move-left"
l = "move-right"
c = "@diw"
y = ["move-down", "move-right"]
yu = "move-down"
<space>s = "@/\b\b<Left><Left>"
```

Action names are case-sensitive and use kebab-case. For example, use
`move-down`, `page-up`, and `save-current-file`, not `MoveDown`, `move_down`,
or `movedown`.

Valid action names:

| Navigation |  |  |  |
| --- | --- | --- | --- |
| `move-left` | `move-right` | `move-up` | `move-down` |
| `move-word-forward` | `move-word-backward` | `move-word-end` | `move-word-end-backward` |
| `move-big-word-end-backward` | `move-paragraph-forward` | `move-paragraph-backward` | `move-line-start` |
| `move-line-end` | `move-past-line-end` | `move-first-non-blank` | `move-to-first-line` |
| `move-to-last-line` | `align-viewport-top` | `align-viewport-center` | `align-viewport-bottom` |
| `page-up` | `page-down` | `half-page-up` | `half-page-down` |
| `find-forward` | `find-backward` | `till-forward` | `till-backward` |
| `repeat-find-forward` | `repeat-find-backward` | `jump-older` | `jump-newer` |
| `goto-definition` | `goto-references` | `goto-file-under-cursor` | `goto-file-under-cursor-at-position` |
| `goto-alternate-file` | `goto-last-modification` | `show-hover` | `open-code-actions` |

| Mode and file actions |  |  |  |
| --- | --- | --- | --- |
| `enter-insert-mode` | `enter-visual-mode` | `enter-visual-line-mode` | `enter-visual-block-mode` |
| `insert-after-cursor` | `open-line-below` | `open-line-above` | `swap-visual-anchor` |
| `enter-command-mode` | `enter-search-mode` | `exit-to-normal-mode` | `hide-search-highlighting` |
| `search-next` | `search-previous` | `save-current-file` | `save-current-file-and-quit` |

| Editing actions |  |  |  |
| --- | --- | --- | --- |
| `delete-char-backward` | `delete-char-forward` | `delete-char-at-cursor` | `delete-word-backward` |
| `delete-to-line-start` | `insert-newline` | `reindent-selection` | `begin-reindent-operator` |
| `indent-selection` | `dedent-selection` | `begin-indent-operator` | `begin-dedent-operator` |
| `toggle-line-comment` | `toggle-block-comment` | `paste-clipboard-after-cursor` | `paste-clipboard-before-cursor` |
| `yank-clipboard` |  |  |  |

| Command/search input actions |  |  |  |
| --- | --- | --- | --- |
| `execute-command` | `cancel-command` | `delete-input-char` | `delete-input-char-forward` |
| `delete-input-word-backward` | `delete-input-to-start` | `delete-input-to-end` | `move-input-start` |
| `move-input-end` | `move-input-left` | `move-input-right` | `move-input-word-left` |
| `move-input-word-right` |  |  |  |

Replay strings let one config binding behave like typing a key sequence. This
reuses the ordinary modal input flow, so operator sequences such as `diw`,
search prompts, command prompts, and other existing key-driven behavior keep
working:

```toml
[keymap.normal]
c = "@diw"
q = "@:q!<Enter>"
```

Angle-bracket tokens spell non-printable keys inside replay strings. The same
named-key syntax used on the left-hand side also works inside replay strings,
with `Enter` added for command execution:

- `@diw`
- `@:w<Enter>`
- `@<Tab>`
- `@g<Ctrl-Home>`

Replay bindings are whole-string values only. Arrays stay action-only.

Array-valued bindings execute each action in order. This works for both direct
bindings and multi-key sequences:

```toml
[keymap.normal]
y = ["move-down", "move-right"]
yu = ["move-down", "move-right"]
```

If you use a numeric count prefix before a multi-action binding, Ordex repeats
the whole sequence. For example, `3y` runs `move-down`, `move-right`,
`move-down`, `move-right`, `move-down`, `move-right`.

If you use a numeric count prefix before a replay binding, Ordex replays the
whole key sequence that many times.

If a single-key binding and a multi-key sequence share the same first key, the
exact single-key binding wins immediately. Use distinct prefixes when you want
both forms to remain reachable.

When you type the first key of a multi-key sequence, Ordex shows a
bottom-right discovery popup that lists the remaining continuations and their
action labels. Config-defined sequences appear in the popup the same way built-in
sequences do.

If any action name in an array is invalid, Ordex ignores the entire binding and
prints the usual startup warning.

If a replay binding directly or indirectly triggers itself, Ordex stops that
replay and reports a recursion warning in the message line.

When a multi-key sequence includes named non-printable keys such as Space, Tab,
or modified navigation keys, use angle-bracket tokens on the left-hand side:

```toml
[keymap.normal]
<space>s = "move-right"
<tab><space>a = "move-down"
<ctrl-home>x = "move-to-last-line"
```

Single-key named bindings can keep the shorter forms such as `space`, `tab`,
and `ctrl-home`. Bare text such as `space-s` still means the literal character
sequence `s`, `p`, `a`, `c`, `e`, `-`, `s`; use `<space>s` when the sequence
should start with the Space key.

Key examples:

- single character: `z`
- single Unicode character: `é`
- control: `ctrl-f`
- alt: `alt-b`
- modified named keys: `ctrl-home`, `ctrl-left`, `alt-right`, `shift-tab`
- named keys also include `tab`
- use `-` between modifier and key; `+` is not supported
- named keys: `space`, `left`, `right`, `up`, `down`, `home`, `end`, `pageup`, `pagedown`, `delete`
- multi-key sequences are supported (for example `zu`, `<space>s`, or `<tab><space>a`)

### `[keymap.operator]`

This section customizes the keys used after starting an operator such as `d`,
`c`, `y`, `=`, `>`, or `<`. Each key must be a single key name, and each value must be
one operator action string.

```toml
[keymap.operator]
é = "word-forward"
g = "paragraph-forward"
```

Valid operator action names:

| Operator motions and prefixes |  |  |  |
| --- | --- | --- | --- |
| `word-forward` | `big-word-forward` | `word-end` | `big-word-end` |
| `word-backward` | `big-word-backward` | `paragraph-forward` | `paragraph-backward` |
| `find-forward` | `find-backward` | `till-forward` | `till-backward` |
| `jump-to-matching-delimiter` | `text-object-inner` | `text-object-around` |  |
| `line-end` | `line-start` | `first-non-blank` |  |

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

### `[swap]`

Use this section to exclude file paths from swap-file protection, especially for
sensitive data that should never be copied into an editor-managed recovery file
or concurrent-open marker:

```toml
[swap]
exclude = ["/dev/shm/gopass*", "*.gpg"]
```

- `exclude` must be an array of strings.
- Arrays can be written on one line or across multiple lines.
- Patterns are matched against the **full absolute file path**.
- `*` matches any sequence of characters, including `/`.
- Empty strings are ignored with a startup warning.

Ordex still honors an already-existing swap file for recovery even when the
current config would exclude that path from creating a new swap file.

## Resilience Behavior

- Unknown sections/keys are ignored with warnings
- Invalid values default safely with warnings
- Missing include files are recoverable with warnings
- Duplicate key definitions use last-definition-wins with warning
- Valid key mappings remain active even if unrelated sections fail
- Startup warnings are printed before the TUI is opened, including source location and line content
