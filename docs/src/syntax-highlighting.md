# Syntax Highlighting

Ordex highlights recognized files automatically when you open them.

## Supported languages

Ordex currently ships with 74 built-in syntax profiles.

| Language | Representative files | Language | Representative files |
| --- | --- | --- | --- |
| Rust | `.rs` | TOML / Ordex config | `.toml`, `Cargo.toml`, `.cfg` |
| Markdown | `.md`, `.markdown`, `README.md` | AsciiDoc | `.adoc`, `.asciidoc`, `.asc` |
| D | `.d` | JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| TypeScript | `.ts`, `.tsx` | Python | `.py`, `.pyi` |
| Java | `.java` | C# | `.cs` |
| C++ | `.cc`, `.cpp`, `.cxx`, `.hpp` | C | `.c`, `.h` |
| Go | `.go` | PHP | `.php`, `.phtml` |
| Bash | `.bash`, `.bashrc`, `.bash_profile` | POSIX sh | `.sh`, `.profile` |
| Zsh | `.zsh`, `.zshrc` | Fish | `.fish`, `config.fish` |
| JSON | `.json` | JSONC | `.jsonc` |
| YAML | `.yaml`, `.yml` | INI | `.ini`, `.gitconfig` |
| CSS | `.css` | SCSS | `.scss` |
| Less | `.less` | Sass | `.sass` |
| XML | `.xml`, `.svg`, `.xsd`, `.xsl` | HTML | `.html`, `.htm` |
| XHTML | `.xhtml` | GraphQL | `.graphql`, `.gql` |
| Protocol Buffers | `.proto` | Thrift | `.thrift` |
| Erlang | `.erl`, `.hrl` | Elm | `.elm` |
| CMake | `CMakeLists.txt`, `.cmake` | Meson | `meson.build`, `meson_options.txt` |
| Ninja | `build.ninja`, `.ninja` | Make | `Makefile`, `GNUmakefile`, `.mk` |
| Dockerfile | `Dockerfile`, `Containerfile`, `.dockerfile` | HCL / Terraform | `.hcl`, `.tf`, `.tfvars` |
| Nix | `.nix`, `default.nix`, `flake.nix` | Kconfig | `Kconfig`, `Kbuild`, `Config.in` |
| PKGBUILD | `PKGBUILD` | Lua | `.lua` |
| Ruby | `.rb`, `Gemfile`, `Rakefile` | Swift | `.swift` |
| Kotlin | `.kt`, `.kts` | Scala | `.scala`, `.sc` |
| R | `.R`, `.r` | SQL | `.sql` |
| Zig | `.zig` | Julia | `.jl` |
| Haskell | `.hs`, `.lhs` | OCaml | `.ml`, `.mli` |
| F# | `.fs`, `.fsi`, `.fsx` | Elixir | `.ex`, `.exs` |
| Groovy | `.groovy`, `.gradle`, `Jenkinsfile` | Dart | `.dart` |
| Perl | `.pl`, `.pm`, `.t` | AWK | `.awk` |
| Solidity | `.sol` | Vala | `.vala`, `.vapi` |
| Nim | `.nim`, `.nims` | Crystal | `.cr` |
| CoffeeScript | `.coffee`, `.litcoffee` | CUE | `.cue` |
| QML | `.qml` | GAS | `.s`, `.S` |
| NASM | `.nasm` | MASM | `.masm` |
| YASM | `.yasm` | Lisp | `.lisp`, `.lsp`, `.cl`, `.el` |
| Git rebase todo | `git-rebase-todo` | Git commit messages | `COMMIT_EDITMSG`, `MERGE_MSG`, `TAG_EDITMSG` |

## What gets highlighted

Across the profiles above, Ordex highlights comments, strings, numbers,
keywords, and punctuation where the generic lexer can recognize them from
profile metadata. Single-quoted character literals in C-family and similar
languages are highlighted with the same string styling when they contain
exactly one scalar and a closing quote. Markdown keeps its separate conservative markup rules.

Git-specific buffers also include targeted rules:
- `git-rebase-todo`: command tokens and following commit hashes are highlighted.
- Git message buffers: `#` comment lines are highlighted, and a non-blank
  second line is marked as invalid.

Syntax highlighting resolves through the active editor theme. The default
theme is `bogster`, and the bundled theme set is:

- `bogster`
- `catppuccin-latte`
- `catppuccin-frappe`
- `catppuccin-macchiato`
- `catppuccin-mocha`
- `gruvbox`
- `kanagawa`
- `nord`
- `onedark`
- `tokyonight`

Theme selection lives in the config file under `[editor] theme = "..."` and can
be reapplied at runtime with `:reload-config`.

## Fallback behavior

If Ordex does not recognize the file name or extension, it falls back to plain text rendering.

Unsupported or ambiguous Markdown constructs also stay plain on purpose. Ordex prefers readable text over misleading color.

## Current limits

Ordex currently does **not** include:

- embedded-language highlighting inside Markdown fences
- embedded-language highlighting inside JavaScript/TypeScript template interpolation or C# interpolated strings
- full heredoc / nowdoc parsing for PHP, shell scripts, Dockerfiles, HCL, Nix, and other heredoc-heavy syntaxes
- advanced Markdown constructs such as tables, task lists, reference links, or HTML blocks
- background lexing threads
- extra runtime parser dependencies

Highlighting is incremental while you edit, but it remains synchronous on the main thread so the screen always reflects the latest stable syntax state.
