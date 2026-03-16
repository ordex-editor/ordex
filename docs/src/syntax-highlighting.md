# Syntax Highlighting

Ordex highlights recognized files automatically when you open them.

## Supported languages

- Rust (`.rs`)
- TOML and common config files (`.toml`, `Cargo.toml`)
- Markdown (`.md`, `.markdown`, `README.md`)
- D (`.d`)
- JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`)
- TypeScript (`.ts`, `.tsx`)
- Python (`.py`, `.pyi`)
- Java (`.java`)
- C# (`.cs`)
- C++ (`.cc`, `.cpp`, `.cxx`, `.hpp`, `.hh`, `.hxx`)
- Go (`.go`)
- C (`.c`, `.h`)
- PHP (`.php`, `.phtml`)
- AsciiDoc (`.adoc`, `.asciidoc`, `.asc`)

## What gets highlighted

- Rust
- TOML
- Markdown
- D
- JavaScript
- TypeScript
- Python
- Java
- C#
- C++
- Go
- C
- PHP
- AsciiDoc

Syntax colors are currently hardcoded, but the styling pipeline is semantic and theme-ready.

## Fallback behavior

If Ordex does not recognize the file name or extension, it falls back to plain text rendering.

Unsupported or ambiguous Markdown constructs also stay plain on purpose. Ordex prefers readable text over misleading color.

## Current limits

Ordex currently does **not** include:

- embedded-language highlighting inside Markdown fences
- embedded-language highlighting inside JavaScript/TypeScript template interpolation or C# interpolated strings
- full heredoc / nowdoc parsing for PHP
- character-literal-specific styling for languages where single quotes are not strings
- advanced Markdown constructs such as tables, task lists, reference links, or HTML blocks
- background lexing threads
- extra runtime parser dependencies

Highlighting is incremental while you edit, but it remains synchronous on the main thread so the screen always reflects the latest stable syntax state.
