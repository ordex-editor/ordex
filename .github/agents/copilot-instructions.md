# ordex Development Guidelines

Auto-generated from all feature plans. Last updated: 2026-03-11

## Active Technologies

- Rust stable (edition 2024) + Existing runtime dependencies only (`termion` 4.0.6, `ropey` 2.0.0-beta.1, `libc` 0.2.180); no new runtime crates planned (001-syntax-highlighting)

## Project Structure

```text
src/
tests/
```

## Commands

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test -- --test-threads=1
```

## Code Style

Rust stable (edition 2024): Follow standard conventions

## Recent Changes

- 001-syntax-highlighting: Added Rust stable (edition 2024) + Existing runtime dependencies only (`termion` 4.0.6, `ropey` 2.0.0-beta.1, `libc` 0.2.180); no new runtime crates planned

<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
