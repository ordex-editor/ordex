# Quickstart: Implementing Syntax Highlighting

**Feature**: 005-syntax-highlighting  
**Audience**: Contributors implementing this feature

## 1) Build the syntax subsystem without adding dependencies

Add a new `src/syntax/` subsystem and keep all highlighting logic in-repo:

- `src/syntax.rs`
- `src/syntax/engine.rs`
- `src/syntax/profile.rs`
- `src/syntax/helpers.rs`
- `src/syntax/profiles/mod.rs`
- `src/syntax/profiles/rust.rs`
- `src/syntax/profiles/toml.rs`
- `src/syntax/profiles/markdown.rs`
- `src/syntax/profiles/d.rs`

Do not add runtime crates. The current dependency budget is already full.

## 2) Implement the language-profile layer first

Start with shared metadata:

- `SyntaxClass` and `SyntaxModifier`
- `LanguageProfile`
- `LanguageDetection`
- `CommentStyle`
- One built-in profile module per language: Rust, config/TOML, Markdown, and D

Phase-1 rules to preserve:

- Filename and extension matching only
- One preferred default ordinary comment style when a language supports multiple ordinary comment styles
- Rust and D documentation comments receive a distinct syntax modifier
- Markdown is conservative-core only
- Future nested-language hooks may exist, but stay inactive in phase 1

## 3) Build the incremental highlighting engine

1. Detect the active language profile when a file is opened.
2. Run a full-document top-to-bottom lex pass on load to satisfy the full-document correctness guarantee.
3. Cache per-line spans and line-entry/exit state.
4. After edits, relex from the first dirty line forward until exit state stabilizes again.
5. Fall back to plain text when no supported profile matches.

Keep derived highlight state in `EditorState`, not in `TextBuffer`.

`Line-state` does **not** mean lexing each line independently. It means each line stores the continuation state it inherited from the previous line so multi-line strings, block comments, nested comments, and Markdown fences still work correctly.

Do **not** introduce a background lexing thread in phase 1 unless profiling later proves the single-threaded design misses the large-file targets. A worker would add stale-highlight latency and coordination complexity without reducing the CPU work the lexer must do.

## 4) Integrate highlighting into rendering

Update:

- `src/editor_state.rs` to own syntax state and trigger relexing on edits/load
- `src/main.rs` so `RenderSnapshot` notices highlight-generation changes
- `render_row_content()` to merge syntax styles with existing selection/cursor styling
- `src/tui.rs` to map syntax classes/modifiers to ANSI output in a theme-ready way

The render path must work for both wrapped and unwrapped lines.

## 5) Cover the clarified language behaviors

Required phase-1 behaviors:

- Rust: keywords, strings, numbers, punctuation, ordinary comments, and distinct documentation comments
- config/TOML: strings, numbers, comments, punctuation, key/value syntax
- D: multiple comment styles including nested block comments and distinct documentation comments
- Markdown: headings, fenced blocks, inline code, block quotes, list markers, simple emphasis, simple inline links/images, thematic breaks, implemented through the same generic engine with helper predicates for boundary-sensitive rules

Leave complex or ambiguous Markdown constructs plain instead of risking misleading colors.

## 6) Add tests before broad integration

Inline unit tests in syntax modules:

- language detection precedence
- comment-style parsing, including nested D block comments
- documentation-comment classification
- line-state stabilization across multiline strings/comments
- conservative Markdown recognition and fallback behavior

Integration tests in `tests/`:

- `syntax_highlighting_test.rs` for ANSI output on supported files
- `syntax_large_file_test.rs` for 50,000-line open/edit/scroll behavior
- updates to `soft_wrap_test.rs` for wrapped-row token boundaries
- updates to `editing_test.rs` for edit-driven cache invalidation

## 7) Update user-facing docs in the same change

Add or update:

- `docs/src/syntax-highlighting.md`
- `docs/src/SUMMARY.md`
- `docs/src/index.md`

Document supported languages, Markdown limits, fallback behavior, and the fact that highlighting is automatic for recognized file names/extensions.

## 8) Validate the finished implementation

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test -- --test-threads=1
```

Optional dependency-budget check:

```bash
cargo tree --edges normal --prefix none
```
