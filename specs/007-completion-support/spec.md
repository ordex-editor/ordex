# Feature Specification: Completion Support

**Feature Branch**: `007-completion-support`
**Created**: 2026-04-02
**Status**: Draft
**Input**: User description: "I want to add the following feature to this project: completion support. In the future, I want to support file paths completion in buffer, buffer text completion, and make it extensible for future features such as LSP and plugin completions. Choose which completion between file paths in buffer or buffer text completion would be the simplest for this MVP. In the planning phase, research about what could be the best approach to avoid freezing the UI and check what other text editors do."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Complete words already in the buffer (Priority: P1)

As an editor user writing or editing text, I want Ordex to suggest words that already exist in the current buffer so I can finish repeated terms faster without leaving the keyboard.

**Why this priority**: Buffer text completion is the simplest completion source for an MVP because it relies only on content already loaded in the current editing session and does not depend on path parsing or external providers.

**Independent Test**: Open a document with repeated words, type a matching prefix near a new occurrence, trigger completion, and verify that the expected in-buffer suggestion can be inserted and used productively on its own.

**Acceptance Scenarios**:

1. **Given** the current buffer already contains one or more words matching the prefix under the cursor, **When** the user requests completion, **Then** Ordex presents matching suggestions drawn from the current buffer.
2. **Given** a suggestion list is visible, **When** the user selects a suggestion, **Then** Ordex inserts the selected completion at the cursor without changing unrelated text.
3. **Given** no words in the current buffer match the current prefix, **When** the user requests completion, **Then** Ordex leaves the existing text unchanged and gives a clear indication that no suggestions are available.

---

### User Story 2 - Keep typing fluid while using completion (Priority: P1)

As an editor user typing continuously, I want completion to stay responsive so suggestion gathering does not interrupt cursor movement or text entry.

**Why this priority**: Completion that stalls editing would make the editor feel worse even when suggestions are accurate, so responsiveness is part of the core user value for the first release.

**Independent Test**: In a large document, trigger completion repeatedly while typing and navigating, and verify that the editor continues to accept input and update the screen without noticeable stalls.

**Acceptance Scenarios**:

1. **Given** a large buffer with many repeated words, **When** the user requests completion, **Then** suggestions appear quickly enough that normal typing and navigation remain uninterrupted.
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
- Repeated matches that differ only by case or nearby punctuation must not produce confusing duplicates in the suggestion list.
- In very large buffers, completion must remain usable without causing visible editor stalls.
- Accepting a completion must replace only the intended prefix and must not delete adjacent characters that are not part of the completion target.
- Moving the cursor, switching buffers, or editing the underlying text while suggestions are visible must not leave stale suggestions on screen.
- File path completion is explicitly out of scope for this MVP and must not appear partially implemented in a way that confuses users.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST provide an MVP completion feature based on text already present in the current buffer.
- **FR-002**: The system MUST identify the completion target from the cursor position and use the current partial word as the basis for matching suggestions.
- **FR-003**: The system MUST present only suggestions that extend the current partial word rather than unrelated text.
- **FR-004**: The system MUST allow the user to accept one of the offered suggestions and insert it at the current cursor location.
- **FR-005**: The system MUST allow the user to dismiss completion without changing the buffer contents.
- **FR-006**: When no matching suggestions exist, the system MUST leave the buffer unchanged and communicate that no completion is available.
- **FR-007**: The system MUST keep ordinary typing, cursor movement, and cancellation responsive while completion is requested, displayed, updated, or dismissed.
- **FR-008**: The system MUST update or discard visible suggestions when the cursor position or relevant buffer text changes so that stale suggestions are not shown.
- **FR-009**: The system MUST avoid presenting duplicate suggestions that would appear identical to the user.
- **FR-010**: The system MUST keep the completion interaction model extensible so future sources such as file paths, language-aware suggestions, and plugins can participate without redefining the core user flow.
- **FR-011**: The system MUST preserve clear source boundaries so the MVP can ship with buffer text completion only while future phases add additional completion sources independently.
- **FR-012**: The system MUST keep file path completion out of scope for this MVP while allowing it to be added as a later completion source.

### Dependencies & Assumptions

- The MVP chooses buffer text completion over file path completion because it can be derived from the active buffer alone and should require fewer special-case parsing rules.
- The first release focuses on a single-buffer completion experience; ranking, multi-source blending, and advanced context awareness are deferred to later phases.
- The planning phase must evaluate approaches used by other text editors to keep completion work from interrupting interactive editing.
- Future phases may add file path, language-aware, or plugin-driven suggestions, but those sources must reuse the same user-facing completion workflow defined here.
- User-facing documentation for completion should be added in the same implementation change, per the project constitution.

### Key Entities *(include if feature involves data)*

- **Completion Request**: The user's attempt to obtain suggestions for the partial word at the current cursor position.
- **Completion Candidate**: A suggested text value that can replace or extend the current partial word.
- **Completion Session**: The active period during which suggestions are visible, can be updated, and may be accepted or cancelled.
- **Completion Source**: A provider of candidates, such as the current buffer in the MVP and file paths, language-aware engines, or plugins in later phases.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In acceptance testing, users can trigger completion for repeated words in the current buffer and insert the intended suggestion successfully in at least 90% of representative trials on the first attempt.
- **SC-002**: In representative large-buffer trials, completion requests do not cause visible editor stalls and users can continue typing or moving the cursor without interruption.
- **SC-003**: When no matching buffer text exists, 100% of tested completion requests leave the existing text unchanged.
- **SC-004**: In validation across representative editing scenarios, accepting a suggestion replaces only the intended partial word in at least 95% of trials.
- **SC-005**: Reviewers can describe a clear path for adding at least two future completion sources, such as file paths and language-aware suggestions, without changing the core completion interaction model.
