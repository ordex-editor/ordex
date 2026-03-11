# Behavioral Contract: Markdown Scope

**Module**: `src/syntax/markdown.rs`  
**Purpose**: Define what phase-1 Markdown highlighting recognizes and what it intentionally leaves plain  
**Date**: 2026-03-11

## Supported in Phase 1

The Markdown profile must recognize these conservative core constructs:

- ATX headings
- fenced code blocks (fence and body as Markdown code-fence markup only)
- inline code
- block quotes
- unordered and ordered list markers
- simple one-line emphasis and strong emphasis
- simple inline links and images
- unmistakable thematic breaks

## Explicitly Deferred

These constructs may remain plain or minimally styled in phase 1:

- HTML blocks
- tables
- task lists
- footnotes
- YAML front matter
- reference-style links
- indented-code ambiguity
- nested or complex emphasis edge cases
- embedded-language highlighting inside fenced blocks

## Required Behaviors

- The Markdown implementation must prefer leaving text plain over introducing misleading colors
- Fenced-block state may cross lines; most other Markdown rules should stay line-local and conservative
- Unsupported advanced constructs must not poison unrelated lines with incorrect styling

## Testing Requirements

- headings, fences, inline code, and list markers render distinctly
- unsupported advanced constructs stay readable
- punctuation-heavy prose and unusual delimiter patterns do not trigger obviously incorrect coloring
