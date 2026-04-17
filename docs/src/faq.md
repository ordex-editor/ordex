# FAQ

## Is Ordex a full Vim replacement?

No. Ordex has its own direction and focuses on a strong editing experience with sane defaults.

## Does search support regular expressions?

No. Search currently uses case-sensitive literal matching.

## Can I open large files?

Ordex uses a rope data structure and is designed for responsive editing on large files.

## What is the long-term product direction?

Ordex aims to provide sane defaults and supports modern features like LSP and fuzzy finding without plugins.

## What LSP support is available today?

Ordex currently provides built-in LSP support for Rust with `rust-analyzer`, Python with
`ty`, `ruff`, and `pylsp`, C/C++ with `clangd`, JavaScript and TypeScript with
`typescript-language-server`, Go with `gopls`, Java with `jdtls`, PHP with `phpactor`,
shell scripts with `bash-language-server`, HTML with `vscode-html-language-server`,
CSS/SCSS/Less with `vscode-css-language-server`, JSON/JSONC with
`vscode-json-language-server`, and YAML with `yaml-language-server`. Go-to-definition (`gd`),
go-to-references (`gr`), rename through `<Space>r` or `:rename {new_name}`, and hover through
`K` are available when the active language server supports them.

For Python, Ordex routes navigation, hover, and rename to `ty` when available and falls back
to `pylsp` when `ty` is unavailable. Diagnostics may be published by both `ruff` and `pylsp`.

JavaScript and TypeScript share one built-in route through `typescript-language-server`.
Some servers primarily contribute hover and diagnostics rather than full navigation and rename;
Ordex enables the subset of features each built-in server reliably supports.

Opened buffers keep their document state synchronized with the language server, including
incremental unsaved edits while you continue editing. Proactive sync is debounced briefly so
ordinary typing does not send one request per keystroke. While language servers are doing
background work, Ordex shows a small bounded LSP progress overlay above the bottom bars. Hover
results open in a read-only popup near the cursor and dismiss on the next keypress. Rename
applies the server-provided workspace edit directly, opens touched files as buffers when needed,
and does not require a separate reload step. The relevant language-server binaries must be
available on `PATH`.

## Where should I report issues?

Use the repository issue tracker.
