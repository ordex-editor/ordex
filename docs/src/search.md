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
- `*` searches the current buffer for the next whole-word match of the identifier under the cursor, or of the next same-line identifier when the cursor is on whitespace or punctuation
- `<Space>*` runs a whole-word `:grep` for the identifier under the cursor, or for the next same-line identifier when the cursor is on whitespace or punctuation, and opens the file search picker
- Command-mode substitute supports `:s<delim>pattern<delim>replacement<delim>` on the current line
- Command-mode substitute supports `:%s<delim>pattern<delim>replacement<delim>` across the whole file
- Valid `:s` and `:%s` input previews replacement text live while you type, even before the final delimiter
- `:%s` preview follows live-search navigation: it moves to the next match and recenters only when the match is outside the viewport
- `:s` preview keeps cursor and viewport at the command-entry location
- `Enter` commits the previewed substitute and keeps the centered viewport
- `Esc` cancels substitute preview and restores the original cursor and viewport
- Search patterns accept Vim-style `\n` to match a line break, while `\\n` stays literal
- Substitute replacement text accepts Vim-style `\r` to insert a line break, while `\\r` stays literal
- Substitute is **global by default** inside its scope, so every match is replaced without a separate `g` flag
- The final delimiter is optional when nothing follows the replacement text
- Substitute accepts alternate delimiters such as `#`, and replacement text supports capture references like `$1` and `$name`

## Match count

After confirming a search with `Enter`, the current match number and total count appear on the right side of the message bar, for example `[3/42]`. The count stays visible alongside any status message on the left. The total is computed by a background scan of the full document and updates incrementally with a spinner while scanning.

- `n` and `N` update the position without re-scanning when the buffer has not changed
- Editing the buffer clears the count; pressing `n` or `N` starts a fresh scan
- Counts above 1,000,000 are capped and shown as `[3/1000000+]`
- `[??/42]` is shown when the total is known but the current match has not been determined yet
- The count indicator is hidden when there are no matches

Example patterns:

```text
/a.c
/alpha\nbeta
/(?i)todo
:s/TODO|FIXME/DONE/
:%s/foo/bar\rbaz/
:%s/TODO/DONE
:%s#([a-z]+)-(\d+)#$2:$1#
```

Unsupported constructs:

- Look-around assertions such as `(?=...)` and `(?<=...)`
- Pattern-side backreferences such as `\1`

Press `Esc` to leave search input without executing. Canceling a preview leaves the previous committed search highlights unchanged.
