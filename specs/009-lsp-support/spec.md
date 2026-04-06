# Feature Specification: Rust Code Navigation MVP

**Feature Branch**: `009-lsp-support`
**Created**: 2026-04-06
**Status**: Draft
**Input**: User description: "I want to add this feature to ordex: LSP support. Start with a MVP that only support go-to definition and rust-analyzer. During the planning phase, do a research to come up with a solution that won't freeze the UI; also research about how to make sure we don't start more LSP server connections than needed (e.g. when having multiple files opened, possibly from different projects — check if opening files from different projects should opened multiple LSP server connections). It is OK to add a single dependency (without any transitive crate dependencies) for this feature even if the budget is full, for instance, the `json` crate."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Jump to a symbol definition in Rust code (Priority: P1)

As a person editing Rust code in Ordex, I want to jump from a symbol usage to its definition so I can inspect and understand code without manually searching for it.

**Why this priority**: This is the core value of the MVP and the smallest language-aware navigation slice that meaningfully improves day-to-day editing.

**Independent Test**: Open a supported Rust project, place the cursor on a symbol with a known definition, trigger the navigation action, and verify that Ordex opens the correct definition location.

**Acceptance Scenarios**:

1. **Given** a supported Rust file and the cursor on a symbol with one known definition, **When** the user triggers go-to-definition, **Then** Ordex opens the file containing that definition and places the cursor at the target location.
2. **Given** the target definition is in a file that is not already open, **When** the user triggers go-to-definition, **Then** Ordex opens that file and navigates to the definition without requiring a restart or manual file search.

---

### User Story 2 - Get clear feedback when navigation cannot complete (Priority: P2)

As a person using code navigation, I want clear feedback when Ordex cannot take me to a definition so I understand whether the symbol is unresolved, unsupported, or temporarily unavailable.

**Why this priority**: Users need predictable behavior and understandable failure states to trust the feature and recover quickly when navigation does not succeed.

**Independent Test**: Trigger go-to-definition from symbols that have no known definition, from unsupported files, and while language-aware navigation is unavailable, then verify that Ordex preserves the current editing context and explains the outcome clearly.

**Acceptance Scenarios**:

1. **Given** the cursor is on a symbol that does not resolve to a definition, **When** the user triggers go-to-definition, **Then** Ordex keeps the user in the current editing context and reports that no definition was found.
2. **Given** the active file is outside the MVP's supported scope, **When** the user triggers go-to-definition, **Then** Ordex does not attempt misleading navigation and tells the user that the file is not supported by this release.
3. **Given** language-aware navigation for the active file is temporarily unavailable, **When** the user triggers go-to-definition, **Then** Ordex tells the user that navigation is not ready and preserves the current cursor position and buffer.

---

### User Story 3 - Navigate correctly across multiple Rust projects in one session (Priority: P2)

As a person working with files from multiple Rust projects in one Ordex session, I want go-to-definition to use the correct project context for the active file so navigation stays accurate across different codebases.

**Why this priority**: Correct project scoping prevents confusing cross-project results and directly reflects the expected real-world workflow described for planning.

**Independent Test**: Open supported Rust files from multiple projects in one Ordex session, trigger go-to-definition in each project, and verify that each navigation opens a definition that belongs to the active file's project context.

**Acceptance Scenarios**:

1. **Given** Ordex has supported Rust files open from two or more projects, **When** the user triggers go-to-definition in one project's file, **Then** Ordex resolves the definition using that file's project context.
2. **Given** the user switches from one supported Rust project to another in the same session, **When** they trigger go-to-definition in the newly active project, **Then** Ordex returns results appropriate to that project without requiring the first project to be closed.

### Edge Cases

- A symbol may have more than one valid definition target, in which case Ordex must avoid silently taking the user to an arbitrary location.
- The active file may contain unsaved edits when the user requests navigation, and the lookup must remain consistent with the user's current editing context.
- The target definition may be located in a file that is not currently open in Ordex.
- A file may be open from a directory that is not recognized as a supported Rust project, and Ordex must fail clearly instead of guessing a project context.
- A navigation request may be made while background language-aware work for that file is still starting up or reconnecting.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide go-to-definition for Rust source files in the MVP scope of this feature.
- **FR-002**: Users MUST be able to trigger go-to-definition from the current cursor position while editing a supported Rust file.
- **FR-003**: When exactly one definition target is resolved, the system MUST open that target and position the cursor at the definition location.
- **FR-004**: When more than one valid definition target is resolved, the system MUST present a clear way for the user to choose the desired target before navigation occurs.
- **FR-005**: When no definition target is resolved, the system MUST keep the user in the current editing context and provide clear feedback that no definition was found.
- **FR-006**: The system MUST clearly distinguish unsupported-file cases, unavailable-navigation cases, and no-definition-found cases so users can tell why navigation did not succeed.
- **FR-007**: The system MUST keep the editor responsive enough for normal cursor movement and mode changes while a definition lookup is in progress.
- **FR-008**: The system MUST resolve definition lookups within the project context of the active Rust file, even when files from multiple Rust projects are open in the same session.
- **FR-009**: The system MUST allow supported Rust files from multiple projects to remain open in one session and still return project-correct definition targets for each active file.
- **FR-010**: The system MUST be able to navigate to a resolved definition even when the destination file was not already open.
- **FR-011**: The system MUST limit the initial release to go-to-definition and MUST NOT require other language-aware editing features as part of this MVP.

### Dependencies & Assumptions

- This MVP is intentionally limited to Rust editing workflows and does not include language-aware navigation for other languages.
- The delivered scope covers go-to-definition only; hover, completion, rename, diagnostics, and other language-aware features are out of scope for this feature.
- The planning phase must research an interaction model that preserves editor responsiveness while lookups are being prepared and resolved.
- The planning phase must research how language-aware sessions should be shared or separated when users open files from the same project versus different projects.
- If more than one definition target is equally valid, prompting the user to choose is acceptable for this MVP.
- User-facing documentation for the new navigation behavior should be updated in the same implementation change, per the project constitution.

### Key Entities *(include if feature involves data)*

- **Project Context**: The Rust project associated with the active file and used to determine which definition targets are valid for that file.
- **Definition Request**: A user-initiated attempt to resolve the symbol under the cursor to one or more definition targets.
- **Definition Target**: A navigable location that represents a resolved symbol definition.
- **Navigation Feedback**: The success, failure, or disambiguation message shown to the user after a definition request is evaluated.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In acceptance testing for supported Rust projects, users can reach the intended definition in 5 seconds or less in at least 90% of successful lookup attempts.
- **SC-002**: In a test run of 30 consecutive definition lookups, the editor remains responsive to cursor movement and mode changes during 100% of lookup attempts.
- **SC-003**: In testing with supported Rust files open from at least three distinct projects in one session, at least 95% of lookups open a definition from the correct project context.
- **SC-004**: In failure-path testing, 100% of unsupported, unavailable, or unresolved lookups preserve the user's original editing context and display actionable feedback.
- **SC-005**: In first-use acceptance testing, at least 90% of users can complete a go-to-definition task in a supported Rust project without external instructions.
