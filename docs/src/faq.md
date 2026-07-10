# FAQ

## Is Ordex a full Vim replacement?

No. Ordex has its own direction and focuses on a strong editing experience with sane defaults.

## Does search support regular expressions?

Yes. Search uses Rust `regex` syntax and is case-sensitive unless the pattern enables flags such as `(?i)`.

Look-around assertions and pattern-side backreferences are not supported.

## Can I open large files?

Ordex does not currenty support opening large files. Large file support is planned for the future.

## What is the long-term product direction?

Ordex aims to provide sane defaults and supports modern features like LSP and fuzzy finding without plugins.

## What LSP support is available today?

Ordex ships built-in LSP defaults for these supported languages:

| Language | LSP servers |
| --- | --- |
| Rust | `rust-analyzer` |
| Python | `ty`, `ruff`, `pylsp` |
| C, C++ | `clangd` |
| C# | `csharp-ls` |
| JavaScript, TypeScript | `typescript-language-server` |
| Go | `gopls` |
| Java | `jdtls` |
| PHP | `phpactor` |
| Bash, POSIX shell, Zsh, Fish | `bash-language-server` |
| Markdown | `marksman` |
| TOML | `taplo` |
| HTML, XHTML | `vscode-html-language-server` |
| CSS, SCSS, Less | `vscode-css-language-server` |
| JSON, JSONC | `vscode-json-language-server` |
| YAML | `yaml-language-server` |
| XML | `lemminx` |
| GraphQL | `graphql-lsp` |
| Dockerfile | `docker-langserver` |
| HCL/Terraform | `terraform-ls` |
| Nix | `nil` |
| Lua | `lua-language-server` |
| Ruby | `solargraph` |
| Swift | `sourcekit-lsp` |
| Kotlin | `kotlin-lsp` |
| Scala | `metals` |
| R | `LanguageServer` via `R --slave -e languageserver::run()` |
| SQL | `sqls` |
| Zig | `zls` |
| Julia | `LanguageServer.jl` via `julia -e "using LanguageServer; runserver()"` |
| Haskell | `haskell-language-server-wrapper` |
| OCaml | `ocamllsp` |
| F# | `fsautocomplete` via `dotnet fsautocomplete --background-service-enabled` |
| Dart | `dart language-server --protocol=lsp` |
| Perl | `perlnavigator` |
| CMake | `cmake-language-server` |
| Elm | `elm-language-server` |
| Erlang | `erlang_ls` |
| CUE | `cue lsp serve` |
| Solidity | `nomicfoundation-solidity-language-server` |
| QML | `qmlls` |

Go-to-definition (`gd`), go-to-references (`gr`), code actions through `<Space>a`,
rename through `<Space>r` or `:rename {new_name}`, hover through `K`, and insert-mode
signature help for supported calls are
available when the active language server supports them.

For Python, Ordex routes navigation, hover, and rename to `ty` when available and falls back
to `pylsp` when `ty` is unavailable. Diagnostics may be published by both `ruff` and `pylsp`.

JavaScript and TypeScript share one built-in route through `typescript-language-server`.
Some servers primarily contribute hover and diagnostics rather than full navigation and rename,
and Ordex enables only the subset of features each built-in server reliably supports.

Opened buffers keep their document state synchronized with the language server, including
incremental unsaved edits while you continue editing. Proactive sync is debounced briefly so
ordinary typing does not send one request per keystroke. While language servers are doing
background work, Ordex shows a small bounded LSP progress overlay above the bottom bars. Hover
results open in a read-only popup near the cursor and dismiss on the next keypress. Signature help
opens automatically in Insert mode from server-provided trigger characters and refreshes while you
move through arguments, showing the server-selected overload and active parameter when available.
Rename and edit-bearing code actions apply the server-provided workspace edit directly, open
touched files as buffers when needed, and do not require a separate reload step. The relevant
language-server binaries must be available on `PATH`.

## Where should I report issues?

Use the repository issue tracker.
