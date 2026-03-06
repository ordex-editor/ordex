# Research: Resilient Configuration Files

**Date**: 2026-03-05  
**Feature**: 004-add-config-files

## Decision 1

- **Decision**: Keep TOML-like declarative syntax (including `#` line comments) and implement parser logic in-repo (no new crate).
- **Rationale**: This satisfies the explicit no-dependency constraint while preserving readable section/key configuration semantics for users.
- **Alternatives considered**:
  - Full strict TOML crate integration.
  - Non-TOML formats (JSON/YAML).

## Decision 2

- **Decision**: Do not adopt `boml` as the parser for this feature.
- **Rationale**: `boml` is zero-dependency by default and fast, but it lacks explicit recovery guarantees needed for resilient partial loading and documents that some invalid TOML may parse as valid. For this feature, parser behavior under malformed input must be tightly controlled in project code.
- **Alternatives considered**:
  - Wrapping `boml` with custom section recovery.
  - Using `boml` only for strict mode and separate fallback parser.

## Decision 3

- **Decision**: No mature parser crate was selected under the combined constraints of dependency-free runtime + resilience-oriented recovery semantics.
- **Rationale**: Surveyed options either require dependencies (`toml`, `toml_edit`, `toml_parser`, `tomling`, `toml-span`, `toml-spanner`) or are too immature/limited for reliable production behavior (`simple-toml-parser`, legacy `tomllib`).
- **Alternatives considered**:
  - `simple-toml-parser` (zero-dependency, but minimal maturity/documentation).
  - `tomllib` (legacy, dependency-heavy, outdated stack).

## Decision 4

- **Decision**: Build a home-made TOML-like resilient parser pipeline with phases: tokenize (with `#` comment stripping outside strings) -> parse-by-section -> validate -> merge defaults -> emit warnings.
- **Rationale**: This architecture directly supports section-level fault isolation, unknown-key tolerance, and deterministic warning/reporting without external dependencies.
- **Alternatives considered**:
  - Fail-fast parser with no partial recovery.
  - Per-line ad-hoc parsing mixed into loader logic.

## Decision 5

- **Decision**: Keep clarified runtime semantics: last-definition-wins for duplicate key mappings; missing includes are recoverable; warnings go to startup stderr/console.
- **Rationale**: These decisions are already accepted in clarification and are core to resilience and UX expectations.
- **Alternatives considered**:
  - Reject duplicate key mappings.
  - Fail startup on missing include files.
