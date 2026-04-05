# Feature Specification: Swap File Safety

**Feature Branch**: `008-swap-file-safety`
**Created**: 2026-04-05
**Status**: Draft
**Input**: User description: "I want to add this feature to ordex: swap files (with proper file syncing after saving the normal text files to only delete the swap file when we're sure the file was properly saved). Add a configuration setting to exclude some file extensions from having swap files. During the research in the planning phase, consider different file formats. Also analyze whether swap files should be the mechanism which will, in a future version, make ordex warn about opening twice the same file in different ordex instances (it seems to be the mechanism used in vim)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Recover unsaved text edits safely (Priority: P1)

As a person editing a normal text file in Ordex, I want the editor to keep a swap copy of my in-progress work so I can recover changes after a crash or unexpected shutdown.

**Why this priority**: The main user value is protecting work from accidental loss, which is the core reason to add swap files at all.

**Independent Test**: Open a normal text file, make unsaved edits, interrupt the editing session unexpectedly, reopen the same file in Ordex, and verify that the interrupted work can be detected and recovered.

**Acceptance Scenarios**:

1. **Given** a user is editing a normal text file with unsaved changes, **When** Ordex records in-progress recovery data, **Then** the user has a recoverable swap copy for that file.
2. **Given** a prior editing session ended unexpectedly and left recovery data behind, **When** the user opens the corresponding normal text file again, **Then** Ordex clearly indicates that recovery data exists and allows the user to restore the unsaved work.

---

### User Story 2 - Keep swap files until the save is durable (Priority: P1)

As a person saving a text file, I want Ordex to remove the swap file only after the real file save is fully confirmed so I do not lose my last recovery point if saving fails partway through.

**Why this priority**: A swap file that disappears before the real save is durable would undermine trust in both saving and recovery.

**Independent Test**: Edit a normal text file, save it, and verify that recovery data remains available until the save is fully completed; simulate an interrupted or failed save and verify that the swap file is not removed prematurely.

**Acceptance Scenarios**:

1. **Given** a user saves a normal text file with unsaved changes, **When** Ordex has not yet confirmed that the saved file is durably written, **Then** the swap file remains available.
2. **Given** a save completes successfully and durability is confirmed, **When** Ordex finishes the save workflow, **Then** the swap file for that editing session is removed.
3. **Given** a save attempt fails or is interrupted before durability is confirmed, **When** the user returns to the file, **Then** the swap file is still present for recovery.

---

### User Story 3 - Exclude selected file types from swap files (Priority: P2)

As a person configuring Ordex, I want to exclude selected file extensions from swap-file creation so I can avoid recovery files for formats where swap files are unnecessary or undesirable.

**Why this priority**: Configuration matters, but it is secondary to the baseline data-protection behavior for ordinary text editing.

**Independent Test**: Configure one or more file extensions to be excluded, edit matching and non-matching files, and verify that swap files are skipped only for the excluded extensions.

**Acceptance Scenarios**:

1. **Given** a file's extension is listed in the user's swap-file exclusion settings, **When** the user edits that file, **Then** Ordex does not create a swap file for it.
2. **Given** a file's extension is not listed in the user's swap-file exclusion settings and the file is otherwise in scope for swap protection, **When** the user edits that file, **Then** Ordex creates and maintains a swap file normally.
3. **Given** exclusion settings contain `log`, **When** the user edits files named `notes.LOG`, `server.log`, and `archive.tar.gz`, **Then** Ordex skips swap files for the first two files, continues protecting the third file based on its final extension, and does not treat extensionless files as excluded by that rule.

### Edge Cases

- If a file extension is excluded after a swap file already exists for that file, the existing recovery data must remain available until the current editing session ends or reaches a durably completed save.
- Files that are not normal text files must not receive swap files unless they are explicitly determined to be in scope by future planning work.
- If a previous session leaves a swap file behind and the main file was also saved later by another process, Ordex must present recovery information clearly without silently discarding either version.
- If a save succeeds functionally but durability confirmation is not completed, the swap file must remain available until confirmation is obtained.
- If the user saves repeatedly during one editing session, swap-file handling must remain consistent and must not remove the active recovery copy too early.
- Files without an extension, files with uppercase extensions, and files with multi-part extensions must follow a clear and predictable exclusion-matching rule.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST create and maintain swap-file-based recovery data for normal text files while they contain unsaved changes.
- **FR-002**: The system MUST make interrupted editing sessions recoverable when swap data remains from a previous session.
- **FR-003**: When recovery data exists for a file being opened, the system MUST notify the user and offer a recovery path before the swap data is discarded.
- **FR-004**: The system MUST refresh swap data during active editing rather than waiting until a normal save, so recovery can include unsaved edits from the interrupted session.
- **FR-005**: The system MUST keep the swap file available until the corresponding file save has been confirmed as durably completed.
- **FR-006**: The system MUST remove the swap file after a successful durable save when no unsaved changes remain for that editing session.
- **FR-007**: The system MUST preserve the swap file when a save fails, is cancelled, or cannot be durably confirmed.
- **FR-008**: The system MUST provide a user-configurable setting that excludes specified file extensions from swap-file creation.
- **FR-009**: The system MUST apply the exclusion setting only to files whose extensions match the configured exclusions.
- **FR-010**: The system MUST leave non-excluded normal text files protected by swap files even when other extensions are excluded.
- **FR-011**: The system MUST match excluded file extensions without regard to letter case, MUST evaluate multi-part filenames by their final extension segment, and MUST treat files without an extension as non-matching unless a separate future rule is introduced.
- **FR-012**: The system MUST scope the initial release to normal text files and MUST NOT require swap-file support for other file-format categories in this feature.

### Dependencies & Assumptions

- "Normal text files" refers to files Ordex treats as ordinary editable text rather than binary or specialized structured formats.
- The initial release focuses on protecting local editing work and recovery; warning about the same file being opened in multiple Ordex instances is future work, not part of this feature's delivered behavior.
- The planning phase should evaluate how different file-format categories might affect future swap-file applicability, but the present feature is intentionally limited to normal text files.
- The planning phase should also assess whether swap files are the right future mechanism for duplicate-open warnings across Ordex instances, without expanding the current implementation scope.
- Excluding file extensions is intended as a user preference for swap-file creation, not as a broader rule about whether those files may be opened or edited.
- User-facing documentation for swap-file behavior and exclusion settings should be updated in the same implementation change, per the project constitution.

### Key Entities *(include if feature involves data)*

- **Swap File**: Recovery data associated with an open normal text file and intended to preserve unsaved edits.
- **Durable Save Confirmation**: The point at which Ordex can treat the main file save as safely completed and can remove recovery data.
- **Extension Exclusion Rule**: A user-defined rule that prevents swap-file creation for files whose extensions match configured exclusions.
- **Recovery Session**: The user interaction that occurs when Ordex detects leftover swap data and offers restoration options.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In recovery testing for representative normal text files, users can restore unsaved edits from an interrupted session in at least 95% of test cases where swap data exists.
- **SC-002**: In save-interruption testing, 100% of cases where durable save confirmation is not reached retain the swap file for later recovery.
- **SC-003**: In successful-save testing, 100% of cases where the file save is durably confirmed remove the corresponding swap file once no unsaved changes remain.
- **SC-004**: In configuration testing, excluded file extensions prevent swap-file creation in 100% of matching test cases while non-excluded normal text files still receive swap protection.
- **SC-005**: In acceptance testing, users can understand whether recovery is available and what action to take without additional documentation in at least 90% of observed recovery prompts.
