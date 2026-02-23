# Search

Press `/` in normal mode to start search input, then type a pattern and press `Enter`.

Example:

```text
/TODO
```

Search behavior:

- Matches literal strings
- Is case-sensitive
- Wraps to the beginning of the document if needed
- `n` jumps to the next occurrence of the last search
- `N` jumps to the previous occurrence of the last search

Press `Esc` to leave search input without executing.
