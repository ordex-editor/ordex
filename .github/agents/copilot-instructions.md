# ordex Development Guidelines

Auto-generated from all feature plans. Last updated: 2026-02-04

## Active Technologies
- Rust stable (edition 2024) + Existing runtime deps only (`termion`, `ropey`, `libc`); new config parser implemented with Rust stdlib only (004-add-config-files)
- Local filesystem config file(s) and optional included files (004-add-config-files)
- Rust stable (edition 2024) + Existing runtime deps only (`termion`, `ropey`, `libc`); dependency-free TOML-like config parser (004-add-config-files)
- Local filesystem config file(s) and optional include files (004-add-config-files)
- Rust stable (edition 2024) + Existing runtime deps only (`termion`, `ropey`, `libc`); no new parser crate (004-add-config-files)

- Rust (stable), edition 2024 + termion 4.0.6 (terminal handling), ropey 2.0.0-beta.1 (text rope) (002-basic-editing)

## Project Structure

```text
src/
tests/
```

## Commands

cargo test [ONLY COMMANDS FOR ACTIVE TECHNOLOGIES][ONLY COMMANDS FOR ACTIVE TECHNOLOGIES] cargo clippy

## Code Style

Rust (stable), edition 2024: Follow standard conventions

## Recent Changes
- 004-add-config-files: Added Rust stable (edition 2024) + Existing runtime deps only (`termion`, `ropey`, `libc`); no new parser crate
- 004-add-config-files: Added dependency-free TOML-like config parsing with no new parser crate dependencies
- 004-add-config-files: Added Rust stable (edition 2024) + Existing runtime deps only (`termion`, `ropey`, `libc`); new config parser implemented with Rust stdlib only


<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
