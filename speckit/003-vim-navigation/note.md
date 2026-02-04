# Phase 003: Vim Navigation Keys

## Objective

Add the most common vim navigation key bindings that are currently missing.

**Currently implemented:** h, j, k, l, w, b, Ctrl+F, Ctrl+B

**To add:**
- `0` - Move to start of line
- `$` - Move to end of line
- `^` - Move to first non-whitespace character
- `gg` - Go to first line of file
- `G` - Go to last line of file
- `e` - Move to end of word

## Constraints

- Reuse existing cursor/navigation infrastructure
- No new architecture changes
- Add unit tests for new navigation functions

## Implementation

1. Add new Action variants: MoveLineStart, MoveLineEnd, MoveFirstNonBlank, MoveToFirstLine, MoveToLastLine, MoveWordEnd
2. Add key bindings in keybindings.rs
3. Add cursor methods in cursor.rs: move_to_line_start (exists), move_to_line_end (exists), move_to_first_non_blank (new)
4. Add navigation function: find_word_end in navigation.rs
5. Handle actions in editor_state.rs

## Acceptance Criteria

- `0` moves cursor to column 0
- `$` moves cursor to last character of line
- `^` moves cursor to first non-whitespace character
- `gg` moves cursor to line 1
- `G` moves cursor to last line
- `e` moves cursor to end of current/next word
- All existing tests pass
- New unit tests for each navigation function
