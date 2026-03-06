# Data Model: Resilient Configuration Files

**Date**: 2026-03-05  
**Feature**: 004-add-config-files

## Overview

The configuration subsystem models user-provided config sources, parsed TOML-like sections, validation outcomes, and the final runtime configuration that drives editor behavior while preserving key mappings during partial failures.

## Entities

### 1. ConfigSource

- **Purpose**: Represents one physical config input file.
- **Fields**:
  - `path: PathBuf` (absolute or resolved path)
  - `kind: Main | Include`
  - `exists: bool`
  - `readable: bool`
  - `content: Option<String>`
- **Validation rules**:
  - Missing/unreadable include sources are recoverable with warning.
  - Missing/unreadable main source falls back to defaults with warning.

### 2. ParsedConfigDocument

- **Purpose**: Parsed TOML-like structure from one source produced by the in-repo parser.
- **Fields**:
  - `source_path: PathBuf`
  - `sections: Vec<ParsedSection>`
  - `unknown_top_level: Vec<String>`
  - `parse_errors: Vec<ParseIssue>`
- **Validation rules**:
  - Unknown keys/sections are retained for reporting but not fatal.
  - Syntax issues are scoped to the section/key when possible.

### 3. ParsedSection

- **Purpose**: Intermediate section payload before validation/merge.
- **Fields**:
  - `name: String` (canonical section name)
  - `items: Vec<ParsedItem>`
  - `valid: bool`
  - `issues: Vec<ValidationIssue>`
- **Validation rules**:
  - Section-level invalidation must not invalidate unrelated sections.
  - Key mapping section can remain valid even when other sections fail.

### 4. ParsedItem

- **Purpose**: One key/value declaration in a section.
- **Fields**:
  - `key: String`
  - `raw_value: String`
  - `normalized_value: Option<ConfigValue>`
  - `known_key: bool`
  - `issue: Option<ValidationIssue>`
- **Validation rules**:
  - Invalid known values default and emit warning.
  - Unknown keys are ignored and emit warning.

### 5. ParserDiagnostic

- **Purpose**: Represents syntax/lexing/parser recovery events from the home-made parser.
- **Fields**:
  - `kind: UnexpectedToken | InvalidHeader | InvalidAssignment | UnterminatedString`
  - `line: usize`
  - `column: usize`
  - `recoverable: bool`
  - `message: String`
- **Validation rules**:
  - Recoverable diagnostics must not stop parsing of subsequent sections.
  - Diagnostics are translated into user-visible warning events for startup output.

### 6. RuntimeConfig

- **Purpose**: Effective settings used by editor runtime.
- **Fields**:
  - `editor: EditorSettings`
  - `ui: UiSettings`
  - `navigation: NavigationSettings`
  - `key_mappings: KeyMappingSet`
  - `source_fingerprint: Vec<PathBuf>` (applied source list)
- **Validation rules**:
  - Every field must be populated (user value or default).
  - Runtime config is always constructible unless critical bootstrap error occurs.

### 7. KeyMappingSet

- **Purpose**: Effective action bindings across mode contexts.
- **Fields**:
  - `bindings: HashMap<(ModeContext, KeyInput), Action>`
  - `conflicts: Vec<KeyMappingConflict>`
  - `applied_count: usize`
- **Validation rules**:
  - Duplicate binding conflicts resolved last-definition-wins.
  - Conflicts produce warnings but do not block startup.

### 8. LoadResultReport

- **Purpose**: Structured summary surfaced to startup stderr/console and tests.
- **Fields**:
  - `startup_allowed: bool`
  - `applied_sections: Vec<String>`
  - `skipped_sections: Vec<String>`
  - `defaulted_keys: Vec<String>`
  - `ignored_unknown_keys: Vec<String>`
  - `warnings: Vec<WarningEvent>`
- **Validation rules**:
  - Must include all recoverable issues emitted during load.
  - Warning list is stable/deterministic for reproducible tests.

## Relationships

- `ConfigSource` 1..* -> 0..1 `ParsedConfigDocument`
- `ParsedConfigDocument` 1 -> * `ParsedSection`
- `ParsedSection` 1 -> * `ParsedItem`
- `ParsedConfigDocument` 1 -> * `ParserDiagnostic`
- `RuntimeConfig` 1 -> 1 `KeyMappingSet`
- `LoadResultReport` aggregates outcomes from all `ConfigSource` and `ParsedSection` records

## State Transitions

`ConfigSource` lifecycle:
1. `Discovered` -> 2. `ReadAttempted` -> 3a. `Parsed` or 3b. `RecoverableFailure`

`ParsedSection` lifecycle:
1. `Unvalidated` -> 2a. `Applied`
2. `Unvalidated` -> 2b. `SkippedWithDefaults`

`LoadResultReport.startup_allowed`:
- `true` when at least baseline runtime config can be built (defaults and any valid sections).
- `false` only for unrecoverable bootstrap errors (outside this feature's normal failure model).
