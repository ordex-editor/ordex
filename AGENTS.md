# Ordex Agent Instructions

## Purpose
Shared instructions for all agent contexts in this repository.

## Project Context
- Language: Rust stable (edition 2024)
- Existing dependencies: `termion`, `ropey`, `libc`
- Structure: `src/`, `tests/`
- Follow the project constitution at `./.specify/memory/constitution.md`

## Workflow
- Prefer human-reviewable, focused changes.
- Keep instructions and code updates concise and non-redundant.
- Run relevant checks after edits (for example: `cargo test`, `cargo clippy`).

## Editing Policy
- NEVER use Python, awk, sed, or bash to modify files.
- ALWAYS modify files using direct file edits.
- ALWAYS produce a readable unified diff.
- Changes must be human-reviewable.
- Shell tools may be used only for running builds/tests, never for editing files.

## Coding Rules
- Every function MUST have a doc-comment.
- Functions longer than 10 lines MUST contain inline comments.
- Complex logic MUST be commented.
- NEVER use `#[allow(dead_code)]`; remove dead code or make test-only helpers `#[cfg(test)]` instead.

## Comment Rules
- Agents must NOT write comments that reference previous versions of the code.
- Comments must NOT reference past behavior (previous, existing, legacy, etc.).
- Comments must NOT use diff-style phrasing ("now", "changed", "keeps").
- Comments must be valid without git history.
- Comments must describe present behavior directly instead of narrating refactors, moves, or file-role transitions.
- Module and entry-point comments must state the current responsibility plainly without comparing the file to another file or an earlier structure.
