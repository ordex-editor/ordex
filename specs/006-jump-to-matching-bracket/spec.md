# Feature Specification: Jump To Matching Bracket

**Feature Branch**: `006-jump-to-matching-bracket`
**Created**: 2026-03-20
**Status**: Draft
**Input**: User description: "Add `%` jump-to-matching-bracket behavior with syntax-aware matching, block-comment support, visible-pair highlighting, and strong rationale for the design choice."

## Clarifications

### Session 2026-03-20

- Q: When `%` is pressed and the cursor is not already on a delimiter, what should happen? -> A: Use Vim-ish behavior and target the next relevant delimiter on the same logical line.
- Q: Which delimiter kinds should v1 support? -> A: Support `()[]{}` and `<>`, plus block-comment open/close delimiters declared by the active syntax profile.
- Q: How should counts interact with `%`? -> A: Preserve Vim count semantics; a count before `%` means percentage-of-file motion, not match-jump repetition.
- Q: Should bracket matching honor nesting? -> A: Yes, for every supported bracket pair, including `<>`.
- Q: How should syntax-aware matching behave in code? -> A: Ignore brackets inside comments and strings.
- Q: What should happen when `%` starts inside a string or comment? -> A: Fall back to plaintext matching within that ignored region.
- Q: How should block-comment matching behave? -> A: `%` should jump between block-comment open/close delimiters, honoring nested block comments where the language profile supports nesting.
- Q: How should passive match highlighting behave? -> A: Only highlight when both endpoints are already visible.
- Q: What should the passive highlight look like? -> A: The delimiter under the cursor is bold; the matching delimiter gets a pale selection-style background, except in Visual mode where a selected mate becomes bold only.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Jump between matching delimiters in code (Priority: P1)

As a developer navigating source code in Ordex, I want `%` to jump between matching brackets so I can move across nested expressions and blocks quickly.

**Why this priority**: `%` is a core Vim navigation command and the primary user-facing value of this feature.

**Independent Test**: Open a supported file with nested bracket structures and verify that `%` jumps from one endpoint to the correct mate.

**Acceptance Scenarios**:

1. **Given** the cursor is on `(`, `[`, `{`, `<`, `)`, `]`, `}`, or `>`, **When** the user presses `%`, **Then** the cursor jumps to the correct matching delimiter and honors nesting.
2. **Given** the cursor is not on a delimiter but there is a supported delimiter later on the same logical line, **When** the user presses `%`, **Then** Ordex matches from that next delimiter on the line.
3. **Given** a count precedes `%`, **When** the user presses `%`, **Then** Ordex keeps existing Vim percentage motion behavior instead of bracket matching.

---

### User Story 2 - Respect syntax and comments (Priority: P1)

As a developer reading real code, I want `%` to ignore misleading delimiters inside strings and comments so the motion lands on the structurally relevant match.

**Why this priority**: Non-syntax-aware matching is visibly wrong in the cases that most undermine trust in the feature.

**Independent Test**: Open a supported file where bracket characters appear in strings, line comments, and block comments, then verify that code-mode `%` ignores those misleading delimiters.

**Acceptance Scenarios**:

1. **Given** the cursor starts in code and a string literal contains extra bracket characters, **When** the user presses `%`, **Then** those string-local brackets do not affect the jump target.
2. **Given** the cursor starts in code and a comment contains extra bracket characters, **When** the user presses `%`, **Then** those comment-local brackets do not affect the jump target.
3. **Given** the cursor starts inside a string or comment, **When** the user presses `%`, **Then** Ordex falls back to plaintext matching within that ignored region instead of doing nothing.

---

### User Story 3 - Match block comments and show the visible pair (Priority: P2)

As a developer navigating languages with block comments, I want `%` to jump between block-comment delimiters and show the visible pair so comment navigation is consistent with bracket navigation.

**Why this priority**: The syntax system already knows block-comment structure, and users explicitly requested parity for block-comment delimiters and lightweight passive feedback.

**Independent Test**: Open a file with block comments, trigger `%` from comment delimiters, and verify that visible pairs highlight correctly without extra off-screen work.

**Acceptance Scenarios**:

1. **Given** the cursor is on or inside a block-comment opener or closer, **When** the user presses `%`, **Then** the cursor jumps to the corresponding comment delimiter.
2. **Given** the active language supports nested block comments, **When** the user presses `%` on a nested opener or closer, **Then** Ordex matches the correct nested mate.
3. **Given** both endpoints of the active match are visible, **When** the cursor rests on the source delimiter, **Then** the source delimiter renders bold and the visible mate renders with the passive match background.

### Edge Cases

- `%` must be a no-op when there is no supported delimiter on or after the cursor on the current logical line and no delimiter under the cursor.
- `<>` matching may over-match in languages where angle brackets are not parser-level delimiters; v1 still treats them as ordinary bracket pairs everywhere.
- Plaintext fallback inside ignored regions must stop at ignored-region boundaries so matching does not jump from inside a string/comment into surrounding code.
- Block-comment matching must respect whether the active language profile allows nested comments.
- Passive highlighting must not trigger additional off-screen scans just for UI feedback.
- In Visual mode, a match endpoint that is already selected must not get an extra background overlay.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST bind `%` to jump-to-matching-delimiter behavior when no count prefix is active.
- **FR-002**: The system MUST preserve Vim count semantics for `%`, where a count before `%` means percentage-of-file motion rather than match-jump behavior.
- **FR-003**: The system MUST support bracket pairs `()`, `[]`, `{}`, and `<>`.
- **FR-004**: The system MUST honor nesting for all supported bracket pairs, including `<>`.
- **FR-005**: The system MUST support block-comment opener/closer matching for languages whose syntax profile defines block comments.
- **FR-006**: The system MUST honor nested block-comment behavior when the active language profile supports nested block comments.
- **FR-007**: When the cursor is not already on a matchable delimiter, the system MUST match from the next supported delimiter on the same logical line.
- **FR-008**: In code-mode matching, the system MUST ignore bracket characters that occur inside strings and comments.
- **FR-009**: If matching starts inside a string or comment and not on a block-comment delimiter, the system MUST fall back to plaintext delimiter matching within that ignored region.
- **FR-010**: The system MUST cache resolved endpoint pairs for repeated `%` use within the current document generation.
- **FR-011**: The system MUST invalidate cached endpoint pairs after edits that change the buffer generation or syntax generation.
- **FR-012**: The system MUST provide passive visible-pair highlighting only when both endpoints are already visible.
- **FR-013**: Passive highlighting MUST render the delimiter under the cursor in bold.
- **FR-014**: Passive highlighting MUST render the visible matching delimiter with a dedicated pale-match background, unless that delimiter is currently selected in Visual mode.
- **FR-015**: In Visual mode, when the visible matching delimiter is selected, passive highlighting MUST render it bold only and MUST NOT add the passive background overlay.
- **FR-016**: The system MUST avoid mutating or invalidating the visible syntax span cache solely to answer `%` matching queries.

### Dependencies & Assumptions

- The feature builds on the existing syntax-highlighting engine and language-profile metadata introduced in `005-syntax-highlighting`.
- V1 uses the existing hand-written lexer and sparse checkpoint model; it does not add tree-sitter or other parser dependencies.
- V1 treats `<>` as ordinary bracket pairs everywhere, without parser-level disambiguation for comparisons or generics.
- SIMD scanning and chunk-summary indexing are deferred until profiling shows the syntax-aware scan path is a real bottleneck.
- User-facing documentation for `%` should be updated in the same change that implements the feature, per the repository constitution.

### Key Entities *(include if feature involves data)*

- **Match Candidate**: The bracket or block-comment delimiter that `%` resolves from the current cursor position before searching for its mate.
- **Match Endpoint Cache**: A generation-scoped map from one resolved delimiter endpoint to its opposite endpoint.
- **Ignored Region**: A syntax-classified string or comment region where code-mode bracket matching does not count delimiters.
- **Passive Match Highlight**: A visible-only UI overlay applied to the active match pair without changing selection semantics.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In acceptance testing, `%` correctly matches nested pairs for `()`, `[]`, `{}`, and `<>` in representative fixtures.
- **SC-002**: In syntax-heavy fixtures, code-mode `%` does not jump to brackets that appear only inside strings or comments.
- **SC-003**: In supported block-comment languages, `%` correctly jumps between block-comment delimiters, including nested comment pairs where applicable.
- **SC-004**: Repeated `%` use across the same endpoint pair is observably responsive after the first resolution and remains correct after cache invalidation on edit.
- **SC-005**: Passive highlighting appears only for visible pairs and does not introduce incorrect styling over existing Visual selections.
