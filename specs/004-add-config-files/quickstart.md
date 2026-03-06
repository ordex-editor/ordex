# Quickstart: Implementing Resilient Config Files

**Feature**: 004-add-config-files  
**Audience**: Contributors implementing this feature

## 1) Create config subsystem modules

Add:

- `src/config.rs`
- `src/config/parser.rs`
- `src/config/include_loader.rs`
- `src/config/validator.rs`
- `src/config/loader.rs`
- `src/config/warnings.rs`
- `src/config/keymap_merge.rs`

Do not add a parser dependency; implement parser and recovery logic in-repo.

## 2) Implement parsing and recovery flow

1. Read main config source (and optional includes).
2. Parse TOML-like syntax into intermediate parsed sections/items (home-made parser).
3. Validate per-section and per-item.
4. Merge valid sections into runtime config.
5. Default invalid/missing values.
6. Preserve valid key mappings even when unrelated sections fail.
7. Emit warnings to startup stderr/console only.

## 3) Implement clarified behaviors

- Unknown keys/sections: ignore + warning.
- Duplicate key mapping conflicts: deterministic last-definition-wins + warning.
- Missing include files: skip include, default affected settings, warning, continue startup.
- Non-key-mapping section failure: continue startup with defaults/recovered values.
- `#` comments are supported and ignored when outside quoted strings.

## 4) Tests to add

Unit tests (in new config modules):
- Tokenization and parsing of valid/mixed-invalid documents
- Validation and default fallback logic
- Key mapping conflict merge semantics
- Warning message generation

Integration tests (`tests/`):
- `config_loading_test.rs`: mixed known/unknown settings apply correctly
- `config_keymap_resilience_test.rs`: key mappings survive unrelated section failures
- `config_include_missing_test.rs`: missing includes are recoverable with warnings

## 5) Documentation updates

Update docs site in same change (constitution requirement):
- Add `docs/src/configuration.md`
- Link it from `docs/src/SUMMARY.md`
- Update troubleshooting guidance for configuration warnings and recovery

## 6) Validation commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test -- --test-threads=1
```
