# Feature Specification: Completion Support

**Feature Branch**: `007-completion-support`
**Created**: 2026-04-02
**Status**: Draft
**Input**: User description: "I want to add the following feature to this project: completion support. In the future, I want to support file paths completion in buffer, buffer text completion, and make it extensible for future features such as LSP and plugin completions. Choose which completion between file paths in buffer or buffer text completion would be the simplest for this MVP. In the planning phase, research about what could be the best approach to avoid freezing the UI and check what other text editors do."

## Clarifications

### Session 2026-04-02

- Q: Should the MVP show completion only on explicit request, or automatically while typing? → A: Automatically while typing.
- Q: When should automatic completion appear while typing? → A: After 1 typed character, but only for candidate words that are at least 3 characters long.
- Q: Should matching be case-sensitive? → A: Match case-insensitively, but insert the candidate using its original casing from the buffer.
- Q: How should selection and cancellation work? → A: Changing the selection updates the editor text immediately, and cancellation happens by moving Up or Down until no item is selected.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Complete words already in the buffer (Priority: P1)

As an editor user writing or editing text, I want Ordex to suggest words that already exist in the current buffer so I can finish repeated terms faster without leaving the keyboard.

**Why this priority**: Buffer text completion is the simplest completion source for an MVP because it relies only on content already loaded in the current editing session and does not depend on path parsing or external providers.

**Independent Test**: Open a document with repeated words, type a matching prefix near a new occurrence, and verify that the expected in-buffer suggestion appears automatically and can be inserted productively on its own.

**Acceptance Scenarios**:

1. **Given** the current buffer already contains one or more candidate words of at least 3 characters that match the prefix under the cursor without regard to case, **When** the user types the first character of that prefix, **Then** Ordex presents matching suggestions drawn from the current buffer automatically.
2. **Given** a suggestion list is visible, including when it contains only one suggestion, **When** the user changes the selected suggestion, **Then** Ordex updates the buffer text immediately to preview the selected completion without changing unrelated text.
3. **Given** a suggestion is currently previewed in the buffer, **When** the user moves the selection until no item is selected, **Then** Ordex restores the original typed prefix and cancels the completion preview.
4. **Given** no candidate words of at least 3 characters in the current buffer match the current prefix, **When** the user types, **Then** Ordex leaves the existing text unchanged and gives a clear indication that no suggestions are available.

---

### User Story 2 - Keep typing fluid while using completion (Priority: P1)

As an editor user typing continuously, I want completion to stay responsive so suggestion gathering does not interrupt cursor movement or text entry.

**Why this priority**: Completion that stalls editing would make the editor feel worse even when suggestions are accurate, so responsiveness is part of the core user value for the first release.

**Independent Test**: In a large document, let completion appear repeatedly while typing and navigating, and verify that the editor continues to accept input and update the screen without noticeable stalls.

**Acceptance Scenarios**:

1. **Given** a large buffer with many repeated words, **When** the user types a matching prefix, **Then** suggestions appear automatically quickly enough that normal typing and navigation remain uninterrupted.
2. **Given** a suggestion list is visible, **When** the user continues typing, cancels completion, or moves the cursor, **Then** the editor responds immediately and the completion UI updates or closes appropriately.

---

### User Story 3 - Preserve room for future completion sources (Priority: P2)

As a product maintainer, I want the first completion feature to fit a reusable completion model so future sources such as file paths, language-aware suggestions, or plugins can be added without redefining the user experience from scratch.

**Why this priority**: The user explicitly wants completion to grow beyond the MVP, and locking the first version to a one-off interaction model would slow future work.

**Independent Test**: Review the documented completion behavior and verify that it separates the user-facing completion flow from the specific source of suggestions, allowing additional sources to follow the same interaction pattern.

**Acceptance Scenarios**:

1. **Given** the MVP ships with buffer text completion only, **When** future completion sources are planned, **Then** they can follow the same request, suggestion, selection, and cancellation flow.
2. **Given** multiple completion sources are added in later phases, **When** a user invokes completion, **Then** the interaction model remains consistent regardless of where suggestions originate.

### Edge Cases

- Completion must behave predictably when the cursor is at the start of a word, in the middle of a word, or at a word boundary with no active prefix.
- Candidate words shorter than 3 characters must not appear as suggestions, even when they match the typed prefix.
- Repeated matches that differ only by case or nearby punctuation must not produce confusing duplicates in the suggestion list.
- Case differences between the typed prefix and buffer text must not prevent a valid suggestion from appearing.
- In very large buffers, completion must remain usable without causing visible editor stalls.
- The currently selected suggestion must be the only text preview applied to the buffer at a time.
- Moving the selection to a state with no selected item must restore the original typed prefix.
- Applying a previewed completion must replace only the intended prefix and must not delete adjacent characters that are not part of the completion target.
- Moving the cursor, switching buffers, or editing the underlying text while suggestions are visible must not leave stale suggestions on screen.
- File path completion is explicitly out of scope for this MVP and must not appear partially implemented in a way that confuses users.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide an MVP completion feature based on text already present in the current buffer.
- **FR-002**: The system MUST identify the completion target from the cursor position and use the current partial word as the basis for matching suggestions.
- **FR-003**: The system MUST present only suggestions that extend the current partial word rather than unrelated text.
- **FR-004**: The system MUST automatically present completion suggestions after the user types the first character of a matching partial word.
- **FR-005**: The system MUST update the buffer text immediately when the selected suggestion changes.
- **FR-006**: The system MUST provide a state where no suggestion is selected.
- **FR-007**: When no matching suggestions exist, the system MUST leave the buffer unchanged and communicate that no completion is available.
- **FR-008**: The system MUST keep ordinary typing, cursor movement, and cancellation responsive while completion is requested, displayed, updated, or dismissed.
- **FR-009**: The system MUST update or discard visible suggestions when the cursor position or relevant buffer text changes so that stale suggestions are not shown.
- **FR-010**: The system MUST avoid presenting duplicate suggestions that would appear identical to the user.
- **FR-011**: The system MUST only suggest candidate words that are at least 3 characters long.
- **FR-012**: The system MUST match typed prefixes against candidate words without regard to case.
- **FR-013**: When a suggestion is previewed, the system MUST insert the candidate using the original casing found in the buffer.
- **FR-014**: When the user moves the selection to no selected item, the system MUST restore the original typed prefix without leaving preview text behind.
- **FR-015**: When there is only one matching suggestion, the system MUST still allow the user to move to no selected item instead of forcing that suggestion to remain applied.
- **FR-016**: The system MUST keep the completion interaction model extensible so future sources such as file paths, language-aware suggestions, and plugins can participate without redefining the core user flow.
- **FR-017**: The system MUST preserve clear source boundaries so the MVP can ship with buffer text completion only while future phases add additional completion sources independently.
- **FR-018**: The system MUST keep file path completion out of scope for this MVP while allowing it to be added as a later completion source.

### Dependencies & Assumptions

- The MVP chooses buffer text completion over file path completion because it can be derived from the active buffer alone and should require fewer special-case parsing rules.
- The first release focuses on a single-buffer completion experience; ranking, multi-source blending, and advanced context awareness are deferred to later phases.
- Automatic suggestion display is part of the MVP user experience rather than an optional later enhancement.
- Automatic suggestions may appear after a single typed character, but only candidates with at least 3 characters are in scope for the MVP suggestion set.
- Prefix matching ignores case, while previewed suggestions preserve the casing of the chosen buffer text.
- Selection is a live preview: changing the selected item updates the buffer immediately, and the user cancels by navigating to a state with no selected item.
- The planning phase must evaluate approaches used by other text editors to keep completion work from interrupting interactive editing.
- Future phases may add file path, language-aware, or plugin-driven suggestions, but those sources must reuse the same user-facing completion workflow defined here.
- User-facing documentation for completion should be added in the same implementation change, per the project constitution.

### Key Entities *(include if feature involves data)*

- **Completion Request**: The user's attempt to obtain suggestions for the partial word at the current cursor position.
- **Completion Candidate**: A suggested text value that can replace or extend the current partial word.
- **Completion Session**: The active period during which suggestions are visible, can be updated, and may preview one selected candidate or none.
- **Completion Source**: A provider of candidates, such as the current buffer in the MVP and file paths, language-aware engines, or plugins in later phases.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In acceptance testing, users can trigger completion for repeated words in the current buffer and insert the intended suggestion successfully in at least 90% of representative trials on the first attempt.
- **SC-002**: In representative large-buffer trials, completion requests do not cause visible editor stalls and users can continue typing or moving the cursor without interruption.
- **SC-003**: When no matching buffer text exists, 100% of tested completion requests leave the existing text unchanged.
- **SC-004**: In validation across representative editing scenarios, changing the selected suggestion replaces only the intended partial word in at least 95% of trials, and navigating back to no selection restores the original prefix correctly.
- **SC-005**: Reviewers can describe a clear path for adding at least two future completion sources, such as file paths and language-aware suggestions, without changing the core completion interaction model.
