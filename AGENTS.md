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
