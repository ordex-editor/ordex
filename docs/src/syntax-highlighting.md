# Syntax Highlighting

Ordex highlights recognized files automatically when you open them.

## Supported languages

- Rust (`.rs`)
- TOML and common config files (`.toml`, `Cargo.toml`)
- Markdown (`.md`, `.markdown`, `README.md`)
- D (`.d`)

## What gets highlighted

- **Rust / D**: keywords, strings, numbers, punctuation, comments, and distinct documentation comments
- **TOML**: bare keys, strings, numbers, punctuation, and comments
- **Markdown**: headings, fenced blocks, inline code, block quotes, list markers, simple emphasis, and simple inline links/images

Syntax colors are currently hardcoded, but the styling pipeline is semantic and theme-ready.

## Fallback behavior

If Ordex does not recognize the file name or extension, it falls back to plain text rendering.

Unsupported or ambiguous Markdown constructs also stay plain on purpose. Ordex prefers readable text over misleading color.

## Current limits

Ordex currently does **not** include:

- embedded-language highlighting inside Markdown fences
- advanced Markdown constructs such as tables, task lists, reference links, or HTML blocks
- background lexing threads
- extra runtime parser dependencies

Highlighting is incremental while you edit, but it remains synchronous on the main thread so the screen always reflects the latest stable syntax state.
