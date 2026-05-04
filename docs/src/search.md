# Search

Press `/` in normal mode to start search input, then type a pattern and press `Enter`.

Example:

```text
/TODO|FIXME
```

Search behavior:

- Uses Rust `regex` syntax
- Is case-sensitive
- Wraps to the beginning of the document if needed
- `n` jumps to the next occurrence of the last search
- `N` jumps to the previous occurrence of the last search

Example patterns:

```text
/a.c
/(?i)todo
```

Unsupported constructs:

- Look-around assertions such as `(?=...)` and `(?<=...)`
- Pattern-side backreferences such as `\1`

Press `Esc` to leave search input without executing.
