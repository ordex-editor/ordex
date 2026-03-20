# Research: Jump To Matching Bracket

**Date**: 2026-03-20  
**Feature**: 006-jump-to-matching-bracket

## Decision 1

- **Decision**: Implement `%` as an on-demand syntax-aware scan backed by existing syntax checkpoints, not as a global pair map recorded by the lexer.
- **Rationale**: Ordex already stores sparse lexer restart state and a viewport-only exact span cache. That architecture is a good fit for replaying syntax state on demand, but not for storing unbounded matching-pair state. Matching pairs require nesting-aware state and broad invalidation after edits, while the existing lexer checkpoints only capture resumable multiline syntax mode. Keeping `%` in navigation/editor state reuses current syntax knowledge without distorting the highlighting design.
- **Alternatives considered**:
  - Recording full matching pairs in the lexer for the whole file.
  - Storing a dense whole-file pair table updated on every edit.
  - Ignoring the syntax engine entirely and using a pure text motion.

## Decision 2

- **Decision**: Use syntax-aware matching in code, but add a plaintext fallback when `%` starts inside a string or comment.
- **Rationale**: The user explicitly wants syntax-aware matching, and that is the main defense against bad jumps caused by brackets in comments and strings. At the same time, a strict "ignored means no-op" rule would make `%` feel broken inside strings/comments. The chosen behavior keeps code-mode matching structurally correct while still letting `%` work locally when the user intentionally starts inside ignored syntax.
- **Alternatives considered**:
  - Ignore strings/comments entirely and do naive matching everywhere.
  - Make `%` a no-op whenever it starts inside a string or comment.
  - Support comment delimiters only inside ignored regions and otherwise do nothing.

## Decision 3

- **Decision**: Support `()`, `[]`, `{}`, and `<>`, and honor nesting for every supported bracket pair.
- **Rationale**: The final clarified requirement explicitly includes `<` and `>`, and the user wants nesting to be honored. V1 therefore treats angle brackets like the other bracket pairs even though that is less precise than a parser-aware interpretation. This keeps the model uniform and implementable with the current lexer architecture.
- **Alternatives considered**:
  - Excluding `<` and `>` from v1 because they are parser-ambiguous in many languages.
  - Supporting `<` and `>` only at the starting cursor position but not during depth tracking.
  - Adding parser-level disambiguation for generics and comparisons in v1.

## Decision 4

- **Decision**: Support block-comment opener/closer matching using syntax-profile metadata, including nested block comments where the profile supports nesting.
- **Rationale**: The syntax system already understands block-comment styles and nested-comment capability. Reusing that metadata makes `%` useful in real languages without introducing a parser. Matching block comments also aligns with the user's expectation that `%` should navigate comment blocks, not only single-character brackets.
- **Alternatives considered**:
  - Brackets only in v1.
  - Matching block comments only for non-nested languages.
  - Treating block-comment matching as a later enhancement after basic brackets ship.

## Decision 5

- **Decision**: Resolve `%` from the next matchable delimiter on the current logical line when the cursor is not already on one.
- **Rationale**: The chosen behavior is intentionally Vim-ish and avoids requiring pixel-perfect cursor placement before every `%`. It still keeps the targeting rule local and cheap by limiting the implicit candidate search to the current logical line.
- **Alternatives considered**:
  - Strict under-cursor only behavior.
  - Fuzzy "enclosing scope" matching when the cursor is inside a region.
  - Searching beyond the current line for an implicit starting delimiter.

## Decision 6

- **Decision**: Preserve Vim count semantics for `%` instead of reinterpreting counts as repeated match-jumps.
- **Rationale**: Count behavior is a user-visible compatibility rule that should remain stable. Reusing `%` counts for another meaning would create surprising behavior and complicate interaction with existing normal-mode count handling.
- **Alternatives considered**:
  - Repeating the match-jump `count` times.
  - Ignoring counts for `%` entirely.
  - Making `%` always mean bracket matching, even with a count prefix.

## Decision 7

- **Decision**: Keep v1 caching limited to resolved endpoint pairs for the current generation.
- **Rationale**: Endpoint caching captures the highest-value repeated-use case with very small invalidation logic. The user asked whether to cache all unrelated brackets seen during a scan, but v1 does not need that complexity. A pair cache is easy to clear on edit and does not require maintaining partial scan indexes or per-line delimiter inventories.
- **Alternatives considered**:
  - Caching every syntax-valid delimiter seen during a scan.
  - Building a per-line or per-chunk delimiter cache immediately.
  - No caching at all.

## Decision 8

- **Decision**: Defer chunk-summary indexing as a follow-up rather than making it part of v1.
- **Rationale**: Xi-style summaries are the most promising long-term optimization for far-away matches, but they introduce more state, more invalidation complexity, and more design surface than the current feature needs. V1 should first establish correct semantics and a clean scan API. If profiling later shows that far-distance scans are too slow, a chunk-summary layer can be added without changing the `%` user model.
- **Alternatives considered**:
  - Build a chunk-summary index now using per-bracket `net_delta` and `min_prefix`.
  - Build a full bracket tree or AST-like structure up front.
  - Skip all structural indexing work permanently.

## Decision 9

- **Decision**: Defer SIMD scanning as a profiling-gated follow-up.
- **Rationale**: SIMD only helps if the final bottleneck is raw delimiter discovery in large scan loops. V1 first needs correct syntax-aware semantics and stable tests. The constitution also requires that any unsafe optimization be isolated and documented, which makes speculative SIMD work a poor first step.
- **Alternatives considered**:
  - Implement SIMD in v1 for bracket detection.
  - Add a new dependency for optimized text scanning.
  - Pursue unsafe low-level optimization before validating higher-level algorithmic choices.

## Decision 10

- **Decision**: Provide passive highlighting only when both endpoints are already visible.
- **Rationale**: The user explicitly chose visible-only passive highlighting. This keeps the UI cheap and predictable and avoids doing extra off-screen work merely to decorate the screen. It also keeps passive highlighting separate from the actual `%` motion path.
- **Alternatives considered**:
  - Always resolve the mate for highlighting, even when off-screen.
  - Skip passive highlighting in v1.
  - Highlight only the current delimiter and not the mate.

## Decision 11

- **Decision**: Style the current delimiter in bold and the visible mate with a pale-match background, except in Visual mode where a selected mate becomes bold only.
- **Rationale**: The user asked for Vim-like feedback rather than a second selection overlay. A dedicated pale-match background keeps passive matching visually distinct from real selection, while the bold-only Visual-mode rule avoids conflicting selection cues.
- **Alternatives considered**:
  - Reusing full selection background for passive matches.
  - Underline-only passive highlighting.
  - Highlighting only the mate and leaving the source delimiter unchanged.

## Decision 12

- **Decision**: Add a dedicated read-only syntax replay API for delimiter classification instead of reusing the visible-window rendering path directly.
- **Rationale**: The existing syntax engine keeps dense exact spans only for the prepared visible window. `%` should not thrash that cache or force rendering code to serve navigation directly. A read-only replay API preserves cache ownership boundaries while still reusing checkpointed lexer logic.
- **Alternatives considered**:
  - Calling visible-window preparation during `%` matching.
  - Materializing dense syntax spans for the whole file.
  - Duplicating syntax classification logic inside navigation.

## Editor Research Summary

### Helix

- **Observed approach**: Tree-sitter-based matching with a bounded plaintext fallback.
- **Relevant evidence**:
  - `helix-core/src/match_brackets.rs` uses `syntax.tree_for_byte_range(...)`, tree-sitter nodes, and falls back to `find_matching_bracket_plaintext`.
  - `MAX_PLAINTEXT_SCAN` is capped at 10,000 characters.
  - Helix changelog entries mention tree-sitter bracket matching, injection syntax trees, and plaintext fallback.
- **Why Ordex does not copy it directly**: Helix already has a real incremental parse tree. Ordex has a syntax highlighter with sparse lexer checkpoints, not a parser tree.

### Xi

- **Observed approach**: Rope-summary design based on monoids that track total and minimum nesting, extended with comment/string state.
- **Relevant evidence**:
  - `docs/docs/rope_science_04.md` describes storing `(total, minimum)` summaries in rope nodes to search for matching transitions in `O(log n)`.
- **Why it matters for Ordex**: Xi validates the chunk-summary direction as a future optimization, but that is a separate index, not a lexer pair table.

### Neovim / Vim

- **Observed approach**: Built-in `%` remains mostly search-based, while `searchpair()` and `matchit` add syntax-aware skip behavior.
- **Relevant evidence**:
  - `runtime/doc/motion.txt` documents `%`, `matchpairs`, comment support, and `matchit`.
  - `runtime/doc/vimfn.txt` documents `searchpair()` and syntax-based skip expressions.
  - `matchit.txt` documents skipping matches in comments/strings and treesitter-capture support.
- **Why it matters for Ordex**: This is the closest precedent for "scan plus syntax skip" without a full parser.

### VS Code

- **Observed approach**: Dedicated incremental bracket-pair tree built from tokenizer output, with a custom parser and AST.
- **Relevant evidence**:
  - Official bracket-pair colorization write-up describes a recursive-descent parser over tokenizer output, skipping brackets in comments/strings and targeting logarithmic-ish update/query behavior.
  - Source under `bracketPairsTextModelPart/bracketPairsTree/` contains the tree, tokenizer, parser, and AST nodes.
- **Why it matters for Ordex**: VS Code supports the idea that scalable matching comes from a dedicated structural index, not from recording full pairs in the lexer.

### Emacs

- **Observed approach**: Syntax-table-aware scan and parse-state logic, with comment-aware parsing controls.
- **Relevant evidence**:
  - Emacs parsing docs describe `scan-sexps`, `scan-lists`, and `parse-sexp-ignore-comments`.
- **Why it matters for Ordex**: Emacs reinforces that syntax-aware scanning is a legitimate design even without a parser tree.

## Final Choice

- **Chosen design**: Syntax-aware on-demand scan in code, plaintext fallback inside ignored regions, block-comment matching, visible-only passive highlight, and generation-scoped endpoint caching.
- **Why this is the best fit now**:
  - Fits Ordex's current syntax engine instead of fighting it.
  - Avoids new dependencies and premature parser architecture.
  - Preserves a path to later chunk-summary optimization if profiling demands it.
  - Keeps user-visible behavior coherent with the clarified Vim-like requirements.
