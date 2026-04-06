# Quickstart: Rust Code Navigation MVP

## Prerequisites

1. Install `rust-analyzer` so it is available on `PATH`.
2. Use a Rust project that has a recognizable workspace root through `Cargo.toml` or `rust-project.json`.

## Smoke Test

1. Start Ordex with a Rust file from a supported project.
2. Place the cursor on a symbol whose definition is known to exist.
3. Trigger go-to-definition with the planned Normal-mode binding `g d`.
4. Confirm that Ordex remains responsive while the lookup is prepared.
5. Confirm that the editor jumps to the definition and opens the destination file if it was not already open.

## Multi-Project Reuse Test

1. Open a Rust file from one Cargo workspace.
2. Open a Rust file from a second Cargo workspace in the same Ordex session.
3. Trigger `g d` in the first file, then in the second file.
4. Confirm that each navigation resolves within the active file's project context instead of crossing projects.

## Failure-Path Test

1. Trigger `g d` on a symbol with no definition target and confirm that Ordex stays in the current buffer and shows a clear message.
2. Trigger `g d` in a non-Rust file and confirm that Ordex reports that the file is outside the MVP scope.
3. Trigger `g d` in a Rust file outside a recognized workspace and confirm that Ordex reports unsupported project context instead of guessing.

## Documentation Touchpoints

- Document the `g d` binding and user-facing lookup feedback in `docs/src/commands.md`.
- Update the long-term direction note in `docs/src/faq.md` so the docs reflect that the first LSP-backed navigation slice exists.
