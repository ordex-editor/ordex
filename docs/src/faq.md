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

Ordex currently supports Rust go-to-definition through the `gd` (LSP) normal-mode shortcut.
Opened Rust buffers keep their document state synchronized with the language server, including
incremental unsaved edits while you continue editing. Proactive sync is debounced briefly so
ordinary typing does not send one request per keystroke. While the language server is doing
background work, Ordex shows a small bounded LSP progress overlay above the bottom bars.

## Where should I report issues?

Use the repository issue tracker.
