# Feature Specification: Syntax Highlighting

**Feature Branch**: `005-syntax-highlighting`
**Created**: 2026-03-11
**Status**: Draft
**Input**: User description: "Add syntax highlighting to Ordex with strong large-file performance and future-ready language metadata for comments, themes, and mixed-language documents."

## Clarifications

### Session 2026-03-11

- Q: Which language set should phase 1 explicitly commit to? → A: Rust, config/TOML, Markdown, and D.
- Q: For large files, what highlighting guarantee should phase 1 promise immediately after open or scroll? → A: Full-file highlighting must be correct immediately after open or scroll.
- Q: What level of Markdown support should phase 1 commit to? → A: Conservative core Markdown highlighting only.
- Q: When a language supports multiple comment styles, should the language profile declare one preferred default style for future comment actions? → A: Yes, each such language profile must declare one preferred default comment style.
- Q: How should phase 1 choose a language profile? → A: Use filename and extension matching only.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Read supported code faster (Priority: P1)

As a developer opening a supported source, configuration, or documentation file in Ordex, I want comments, strings, keywords, numbers, and structural punctuation to be visually distinct so I can understand the file at a glance.

**Why this priority**: Immediate readability is the primary value of syntax highlighting. If the editor cannot make supported files easier to scan on first open, the feature does not deliver its core benefit.

**Independent Test**: Open representative supported files and verify that the expected syntax categories are visually distinct before making any edits.

**Acceptance Scenarios**:

1. **Given** a supported source file containing comments, strings, numbers, keywords, and brackets, **When** the user opens the file, **Then** those syntax elements are visually distinguished from normal text.
2. **Given** a supported configuration file containing inline comments and quoted values, **When** the user opens the file, **Then** comments are visually distinct without misclassifying comment markers inside quoted text.
3. **Given** a Markdown document containing core phase-1 constructs, **When** the user opens the file, **Then** those core constructs are visually distinguished while unsupported advanced constructs remain readable.

---

### User Story 2 - Keep highlighting correct while editing large files (Priority: P2)

As a developer editing a large supported file, I want highlighting to remain accurate and responsive while I type, scroll, and navigate so I can keep working without lag or visual corruption.

**Why this priority**: Incorrect or sluggish highlighting erodes trust in the editor and gets in the way of normal editing, especially for large files where performance matters most.

**Independent Test**: Edit and scroll through a supported file of up to 50,000 lines and verify that highlighting updates without reopening the file and without freezing the editor.

**Acceptance Scenarios**:

1. **Given** a supported file of up to 50,000 lines, **When** the user opens the file or scrolls to any location, **Then** highlighting is correct across the full document without delayed off-screen catch-up and the editor remains responsive.
2. **Given** a supported language that uses more than one comment style, **When** the user edits inside or next to those comments, **Then** the affected text is reclassified correctly after the edit.
3. **Given** a multi-line string or comment near the end of a file, **When** the user adds or removes a closing delimiter, **Then** highlighting updates to reflect the new open or closed state correctly.

---

### User Story 3 - Fail safely on mixed or unsupported documents (Priority: P3)

As a developer opening a document whose syntax is unsupported, incomplete, or mixed with embedded content, I want Ordex to prefer stable, conservative coloring over misleading output so the file stays readable and trustworthy.

**Why this priority**: A safe fallback is better than incorrect coloring because misleading visual signals can cause editing mistakes and make the editor feel unreliable.

**Independent Test**: Open unsupported files, mixed-content files, and irregular syntax fixtures and verify that the display remains readable without obviously incorrect coloring spreading through unrelated text.

**Acceptance Scenarios**:

1. **Given** a file with no matching supported language profile, **When** the user opens it, **Then** Ordex shows the file in plain text or minimal fallback styling instead of applying incorrect syntax colors.
2. **Given** a supported host document that may contain embedded content, **When** the user opens it in this phase, **Then** the host document remains stably highlighted and readable even if embedded sections are not yet fully differentiated.

### Edge Cases

- Very large files of 50,000 lines or more must remain navigable without requiring syntax highlighting to be manually disabled or reduced to viewport-only correctness.
- Languages that allow multiple comment styles, including nested block comments, must not leak comment coloring into surrounding code once delimiters close.
- Unterminated strings or comments at end of file must affect only the relevant text and recover once the structure is closed.
- Unsupported or ambiguous files must fall back safely instead of miscoloring the entire buffer.
- Documents with unusual punctuation or unconventional lexical patterns must remain readable even if highlighting is intentionally conservative.
- Documents that mix host and embedded syntaxes must remain stable now and must not block a future expansion to nested highlighting.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST automatically apply syntax highlighting when a file matches a supported language profile by filename or extension, starting with Rust, config/TOML, Markdown, and D in this phase.
- **FR-002**: The system MUST visually differentiate, when defined by a supported language profile, comments, strings, numbers, keywords, and structural punctuation from normal text.
- **FR-003**: The system MUST keep file opening, scrolling, and editing responsive for supported files up to 50,000 lines while maintaining correct full-document highlighting.
- **FR-004**: The system MUST refresh highlighting after edits without requiring the file to be reopened or manually reloaded.
- **FR-005**: The system MUST support languages that use more than one comment style in the same file, including nested block comments when the language requires them.
- **FR-006**: The system MUST maintain correct classification across line boundaries for multi-line constructs such as strings and comments.
- **FR-007**: The system MUST provide a safe fallback for unsupported or ambiguous files that preserves readability and avoids obviously incorrect coloring.
- **FR-008**: The system MUST keep language-specific comment rules, including a preferred default comment style when multiple styles exist, in a reusable form so future comment-continuation and comment-toggle features can use the same definitions.
- **FR-009**: The system MUST define highlighting in reusable syntax categories so future theme support can restyle the output without redefining each language.
- **FR-010**: The system MUST preserve compatibility with a future phase that adds nested highlighting for one language embedded inside another.
- **FR-011**: The system MUST allow the supported language set to grow through additional language profiles without changing the user experience for existing languages.
- **FR-012**: The system MUST behave conservatively for document types whose structure is only partially supported in this phase, favoring stable readability over aggressive but incorrect coloring.
- **FR-013**: For Markdown in this phase, the system MUST highlight core document structures while leaving unsupported advanced constructs readable rather than miscolored.

### Dependencies & Assumptions

- This phase covers syntax highlighting behavior only; theme selection, embedded-language highlighting, indentation behavior, and bracket-navigation behavior remain future phases.
- The initial rollout will support Rust, config/TOML, Markdown, and D, covering Ordex's common source, configuration, and documentation use cases while validating nested-comment behavior.
- Markdown support in this phase is limited to conservative core constructs; more complex Markdown behavior may remain plain or minimally styled until a later phase.
- Phase-1 language selection uses filename and extension matching only; richer detection hints and manual overrides remain future enhancements.
- The detailed design chosen during planning must respect Ordex's minimal-dependency philosophy.
- Detailed research on how to analyze text efficiently, how to handle irregular document types, and whether additional reusable language metadata should be stored is intentionally deferred to planning.

### Key Entities *(include if feature involves data)*

- **Supported Language Profile**: The description of how one file type should be visually classified, including its syntax categories, comment styles, and a preferred default comment style when multiple styles are supported.
- **Syntax Category**: A reusable meaning such as comment, string, number, keyword, structural punctuation, or normal text that can later be restyled by themes.
- **Highlighted Range**: A contiguous portion of text assigned one syntax category for display.
- **Comment Style**: A language-specific rule describing how single-line, multi-line, and nested comments begin, end, and interact.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In acceptance testing, representative Rust, config/TOML, and D files, plus Markdown files covering core phase-1 constructs, show the expected syntax categories on first open.
- **SC-002**: Users can open a supported file up to 50,000 lines and obtain correct full-document highlighting within 3 seconds, with no delayed off-screen catch-up required after scrolling.
- **SC-003**: During validation trials on supported 50,000-line files, 95% of single-line insertions and deletions show corrected highlighting in the affected area within 0.2 seconds and without missed input.
- **SC-004**: In validation across approved edge-case files, users do not observe lingering incorrect coloring after fixing nested-comment or unterminated multi-line errors in any approved fixture.
- **SC-005**: In user evaluation of representative supported files, at least 4 out of 5 reviewers report that the editor is easier to scan with highlighting enabled than without it.
