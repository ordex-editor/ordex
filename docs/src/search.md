# Search

Press `/` in normal mode to start search input, then type a pattern and press `Enter`.

Example:

```text
/TODO|FIXME
```

Search behavior:

- Uses Rust `regex` syntax
- Is case-sensitive
- Highlights all visible matches live while typing in `/` search mode
- Keeps the cursor in place during search preview until you press `Enter`
- Keeps highlighting the active search matches after `Enter` until a later search replaces them
- Wraps to the beginning of the document if needed
- `n` jumps to the next occurrence of the last search
- `N` jumps to the previous occurrence of the last search
- Command-mode substitute supports `:s<delim>pattern<delim>replacement<delim>` on the current line
- Command-mode substitute supports `:%s<delim>pattern<delim>replacement<delim>` across the whole file
- Substitute is **global by default** inside its scope, so every match is replaced without a separate `g` flag
- The final delimiter is optional when nothing follows the replacement text
- Substitute accepts alternate delimiters such as `#`, and replacement text supports capture references like `$1` and `$name`

Example patterns:

```text
/a.c
/(?i)todo
:s/TODO|FIXME/DONE/
:%s/TODO/DONE
:%s#([a-z]+)-(\d+)#$2:$1#
```

Unsupported constructs:

- Look-around assertions such as `(?=...)` and `(?<=...)`
- Pattern-side backreferences such as `\1`

Press `Esc` to leave search input without executing. Canceling a preview leaves the previous committed search highlights unchanged.
