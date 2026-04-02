# Quickstart: Completion Support

## Goal

Validate the planned MVP behavior for automatic buffer-text completion before and after implementation.

## Prerequisites

```bash
cargo build
```

Launch Ordex against a file with repeated identifiers or words:

```bash
cargo run -- src/editor_state/mod.rs
```

## Planned default interaction

- Completion is active only in **Insert mode**
- Suggestions appear automatically after the **first typed character**
- Only candidate words with **3 or more characters** are eligible
- Matching is **case-insensitive**
- Previewed text preserves the candidate’s **original casing**
- Changing the selection updates the **editor text immediately**
- Cancellation happens by moving **Up/Down** until **no item is selected**

## Happy-path check

1. Open a file containing repeated words such as `matching`, `message`, or `buffer`.
2. Move to a new insertion point and enter Insert mode.
3. Type the first character of a repeated 3+ character word.
4. Confirm that suggestions appear automatically without leaving Insert mode.
5. Use Up/Down to move between candidates.
6. Confirm that each selection change updates the buffer text immediately.
7. Confirm that only the typed prefix is replaced and the previewed text keeps the casing stored in the buffer.

## Edge-case checks

1. Type a lowercase prefix for an uppercase or mixed-case buffer word and confirm the suggestion still appears.
2. Narrow the list until only one candidate remains and confirm you can still move to **no selection** and restore the original prefix.
3. Move the selection to **no selected item** and confirm the preview disappears and the original prefix returns.
4. Type or navigate so the active prefix becomes invalid and confirm the popup disappears instead of showing stale suggestions.
5. Ensure words shorter than 3 characters never appear as candidates.
6. Repeat the flow in a large file and confirm typing remains responsive.

## Validation commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --quiet
```
