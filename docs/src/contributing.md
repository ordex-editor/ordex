# Contributing LSP Support

Ordex keeps built-in LSP support in the Rust source tree instead of loading an
external server registry at runtime. To add support for another LSP server,
update the catalog, routing, docs, and tests together.

## Files to Update

| File | Responsibility |
| --- | --- |
| `src/lsp/server/catalog.rs` | Define the server id, command, supported languages, project detection markers, and LSP `languageId` mapping |
| `src/lsp/server/routes.rs` | Add the server to one language route, feature ownership, and generated project-description coverage |
| `src/lsp/project/mod.rs` | Reuse existing project-detection behavior tests or add a new route-specific workspace test when needed |
| `docs/src/faq.md` | Add the language and server to the supported-LSP table |
| `tests/` or module tests | Add regression coverage for routing, detection, and any integration behavior you can exercise with a real installed server |

## How to Add a Server

1. Add a new `LspServerId` variant in `src/lsp/server/catalog.rs`.
2. Define one `LspServerDescriptor` with:
   - the executable command and arguments
   - the supported `LanguageId` values
   - feature flags for navigation, hover, rename, and diagnostics
   - `ProjectDetection::RustWorkspace` or `ProjectDetection::MarkerBased`
3. Extend `LspServerDescriptor::lsp_language_id()` when the server needs a new
   `languageId` string.
4. Add the server to the appropriate sync route in `src/lsp/server/routes.rs`.
   Non-sync routes are filtered from the sync route by feature flags, so the
   sync ordering is the source of truth.
5. If the server needs new project markers, add them in the catalog. The
   user-facing project description is generated from those marker arrays.
6. Update the FAQ table so users can discover the new built-in support.

## Choosing Project Markers

- Prefer stable project-root files that tools already expect, such as
  `Cargo.toml`, `go.mod`, `package.json`, or `pubspec.yaml`.
- Use `fallback_to_file_directory = true` only when single-file or loose-file
  workflows are reasonable for that server.
- Keep marker lists small and specific so project ownership stays predictable.

## Testing Expectations

- Add unit tests for route selection and generated project descriptions.
- Add workspace-detection tests when the marker set or fallback behavior matters.
- Add integration tests only for servers that are actually installed in the
  environment.

## Documentation Notes

- If a server only supports a subset of features reliably, reflect that through
  feature flags instead of documenting unsupported actions as available.
