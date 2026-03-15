# Data Model: Syntax Highlighting

**Date**: 2026-03-11  
**Feature**: 005-syntax-highlighting

## Overview

The syntax-highlighting design adds reusable language metadata and derived document highlight state on top of Ordex's existing rope-backed text buffer. The model keeps language definitions declarative, makes edit-time relexing incremental, and preserves future hooks for themes, nested syntaxes, and comment commands without forcing those future features into phase 1.

## Supporting Value Types

### SyntaxClass

Core reusable categories shared across all supported languages:

- `Plain`
- `Comment`
- `String`
- `Number`
- `Keyword`
- `Punctuation`
- `Markup`

### SyntaxModifier

Optional refinements layered on top of `SyntaxClass`:

- `DocComment`
- `Heading`
- `Emphasis`
- `Strong`
- `InlineCode`
- `CodeFence`
- `ListMarker`
- `Quote`
- `Link`

These remain semantic so future themes can restyle them without redefining language rules.

## Entities

### 1. LanguageProfile

- **Purpose**: Describes how one supported language should be detected and highlighted.
- **Fields**:
  - `id` — canonical identifier (`rust`, `toml`, `markdown`, `d`)
  - `display_name`
  - `detection: LanguageDetection`
  - `classes_used: Vec<SyntaxClass>`
  - `modifiers_used: Vec<SyntaxModifier>`
  - `keywords: KeywordSet`
  - `comment_styles: Vec<CommentStyle>`
  - `preferred_comment_style: Option<CommentStyleId>`
  - `nested_hooks: Vec<NestedLanguageHook>`
- **Validation rules**:
  - Must define at least one filename or extension detection rule.
  - If more than one ordinary comment style exists, `preferred_comment_style` is required.
  - `nested_hooks` may exist in phase 1 but must default to safe no-op behavior.

### 2. LanguageDetection

- **Purpose**: Describes how phase 1 selects a profile from a file path.
- **Fields**:
  - `exact_filenames: Vec<String>`
  - `extensions: Vec<String>`
  - `match_precedence: ExactFilenameFirst`
- **Validation rules**:
  - Detection uses filename and extension matching only.
  - Exact filename matches override extension matches.
  - Unsupported paths return no profile and fall back to plain text.

### 3. CommentStyle

- **Purpose**: Captures reusable comment behavior for highlighting and future comment commands.
- **Fields**:
  - `id`
  - `flavor: Ordinary | Documentation`
  - `kind: Line | Block`
  - `open`
  - `close: Option<String>`
  - `nests: bool`
  - `usable_for_continue: bool`
  - `usable_for_toggle: bool`
- **Validation rules**:
  - Block styles require both `open` and `close`.
  - Only one ordinary style per language may be marked as the preferred default.
  - Nested block comments must explicitly declare `nests = true`.

### 4. HighlightSpan

- **Purpose**: Represents one styled region of a logical buffer line.
- **Fields**:
  - `line_index`
  - `start_col`
  - `end_col`
  - `class: SyntaxClass`
  - `modifier: Option<SyntaxModifier>`
- **Validation rules**:
  - Ranges are ordered, non-overlapping, and end-exclusive.
  - Unsupported constructs are omitted rather than guessed.
  - Multi-line constructs are represented as line-local spans plus line state, not as one giant cross-line span.

### 5. LineLexState

- **Purpose**: Carries the entry/exit context required to lex one line correctly.
- **Fields**:
  - `line_index`
  - `entry_mode`
  - `exit_mode`
  - `revision`
  - `stable: bool`
- **Validation rules**:
  - A line with identical text and identical `entry_mode` must produce the same spans and `exit_mode`.
  - Relexing proceeds forward until `exit_mode` stabilizes again.
  - This state does not imply independent line-by-line lexing; the initial lex pass still walks the whole file from top to bottom so multiline strings, comments, and fences inherit context correctly.

### 6. DocumentHighlightState

- **Purpose**: Owns syntax state for the currently open document.
- **Fields**:
  - `active_profile: Option<LanguageProfileId>`
  - `detection_source: MatchByFilename | MatchByExtension | PlainFallback`
  - `line_states: Vec<LineLexState>`
  - `spans_by_line: Vec<Vec<HighlightSpan>>`
  - `dirty_start_line: Option<usize>`
  - `generation: u64`
  - `fully_lexed: bool`
- **Validation rules**:
  - Supported files must reach `fully_lexed = true` after the initial lex pass.
  - Every text edit must mark a dirty line and increment `generation` after relexing.
  - Plain-text fallback still maintains a valid document state with empty or plain spans.

### 7. NestedLanguageHook

- **Purpose**: Reserved metadata for a later phase that supports embedded syntaxes.
- **Fields**:
  - `host_modifier`
  - `target_hint`
  - `fallback_behavior: PlainHostContent`
- **Validation rules**:
  - Hooks must never force nested highlighting in phase 1.
  - Fallback behavior must keep host content readable when no nested engine is active.

## Initial Built-In Profiles

| Profile | Detection | Comment styles | Phase-1 notes |
|---------|-----------|----------------|---------------|
| Rust | `.rs` | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | Keywords, strings, numbers, punctuation, comments, distinct doc comments |
| config/TOML | `.toml`, exact config filenames as needed | `#` | Comments, strings, numbers, punctuation, key/value syntax |
| Markdown | `.md`, `.markdown` | none | Conservative core constructs only, implemented on the generic engine with helper predicates |
| D | `.d` | `//`, `///`, `/* */`, `/** */`, `/+ +/`, `/++ +/` | Requires nested block comments and distinct doc comments |

## Relationships

- `LanguageProfile` 1 -> 1 `LanguageDetection`
- `LanguageProfile` 1 -> * `CommentStyle`
- `LanguageProfile` 1 -> * `NestedLanguageHook`
- `DocumentHighlightState` 1 -> 0..1 `LanguageProfile`
- `DocumentHighlightState` 1 -> * `LineLexState`
- `DocumentHighlightState` 1 -> * `HighlightSpan` (grouped by line)

## State Transitions

### DocumentHighlightState

1. `Uninitialized`
2. `ProfileSelected` or `PlainFallback`
3. `FullLexInProgress`
4. `Stable`
5. `DirtyFromEdit`
6. `RelexingForward`
7. `Stable`

### LineLexState

1. `Dirty`
2. `Lexed`
3. `Stable`

If a line's `exit_mode` changes, the next line returns to `Dirty` and the forward relex continues until stabilization.

## Deferred Models

- **IndentMetadata**: Deferred to a future indentation feature so highlighting data stays focused and lightweight.
- **BracketMetadata**: Static bracket-pair definitions may be added later, but phase 1 does not store live matched-bracket locations in highlight output.
