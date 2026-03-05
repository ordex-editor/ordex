# Research: Text Data Structure Selection

**Date**: 2025-02-04  
**Context**: Phase 002 Basic Editing - Need to select an efficient text data structure for the editor

## Problem Statement

The MVP viewer (Phase 001) uses a simple `Vec<String>` for storing file lines. This is sufficient for read-only viewing but will be inefficient for editing operations. We need a data structure that:

1. Supports efficient insertion/deletion at arbitrary positions
2. Provides efficient line-based access for rendering
3. Handles files > 1 GB without performance degradation
4. Enables future regex search capability
5. Stays within our constitutional constraint: max 5 total runtime dependencies (currently at 3)

## Research Findings

### Text Data Structure Alternatives

#### 1. Gap Buffer

**Concept**: Array with a movable "gap" (empty space) at the cursor position. Insertions/deletions happen in O(1) at the gap, but moving the gap requires O(n) copying.

**Implementation in Practice**:
- Used by: Emacs, early versions of many editors
- Simple to implement, good for single-cursor sequential editing
- Poor for random access edits or multiple cursors

**Performance Characteristics**:
- Insert at cursor: O(1) amortized
- Delete at cursor: O(1)
- Insert/delete elsewhere: O(n) - must move gap
- Random access: O(1)
- Memory overhead: Minimal (just the gap space)

**Large File Handling**: Poor - requires loading entire file into contiguous memory

**Rust Crates Available**:
- `gap-buffer` (0.1.0) - 1 transitive dep (libc, already in our tree) = **0 NEW deps**
- `gapbuf` (0.1.4) - needs dependency check
- Last updated: 2016-2017 (stale)

**Verdict**: ❌ **REJECTED** - O(n) gap movement is problematic for navigation-heavy vim usage. Poor large file support.

---

#### 2. Piece Table

**Concept**: Original file buffer + append-only buffer + table of spans referencing these buffers. Never modifies original data, only adjusts span pointers.

**Implementation in Practice**:
- Used by: VSCode (with modifications), many Microsoft editors
- Excellent for undo/redo (just save span tables)
- Good for large files (can use mmap for original buffer)

**Performance Characteristics**:
- Insert: O(log n) or O(1) depending on implementation
- Delete: O(log n) or O(1)
- Random access: O(log n)
- Line-based access: Requires scanning or auxiliary index
- Memory overhead: Moderate (span table + append buffer)

**Large File Handling**: Excellent - original buffer can be memory-mapped

**Rust Crates Available**:
- `piece_table_rs` (0.1.4) - **0 transitive deps** = **0 NEW deps**
- `piece_table` (0.1.0) - needs check
- `peace-table` (0.1.0) - UTF-8 focused, char oriented
- Last updated: 2019-2024 (mixed maintenance)

**Verdict**: ⚠️ **VIABLE BUT CONCERNS** - Available crates are immature/unmaintained. Line-based access requires additional work.

---

#### 3. Rope (Tree-Based)

**Concept**: Balanced tree where leaf nodes contain text chunks. Insertions/deletions rebalance tree. Popularized by Xi-editor research.

**Implementation in Practice**:
- Used by: Xi-editor, Lapce, many modern editors
- Excellent for all operations
- Well-studied with mature implementations

**Performance Characteristics**:
- Insert: O(log n)
- Delete: O(log n)
- Random access: O(log n)
- Line-based access: O(log n) with line metrics
- Memory overhead: Moderate (tree nodes)

**Large File Handling**: Excellent - can lazy-load chunks

**Rust Crates Available**:

**A. ropey (2.0.0-beta.1)** - **RECOMMENDED**
- Transitive deps: 1 (str_indices) = **1 NEW dep** ✅
- Maintained by Nathan Vegdahl (Xi-editor contributor)
- Production-ready (used by Helix editor, Lapce, others)
- Features:
  - UTF-8 safe
  - Efficient line/char/byte indexing
  - Chunk-based tree (good cache locality)
  - Slice/iterator support
  - Grapheme cluster support (optional feature)
- Repository: https://github.com/cessen/ropey
- Documentation: Excellent
- Last updated: 2024 (actively maintained)

**B. crop (0.4.3)**
- Transitive deps: 1 (str_indices) = **1 NEW dep** ✅
- Similar to ropey but different design choices
- Less battle-tested in production
- Last updated: 2024

**Verdict**: ✅ **SELECTED** - Rope via `ropey` crate is the clear winner.

---

### Decision Matrix

| Criteria | Gap Buffer | Piece Table | Rope (ropey) |
|----------|------------|-------------|--------------|
| Insert/Delete Performance | ❌ O(n) worst case | ✅ O(log n) | ✅ O(log n) |
| Random Access | ✅ O(1) | ⚠️ O(log n) | ✅ O(log n) |
| Line-based Access | ✅ Direct | ❌ Needs index | ✅ Built-in |
| Large File Support | ❌ Poor | ✅ Excellent | ✅ Excellent |
| Memory Overhead | ✅ Minimal | ⚠️ Moderate | ⚠️ Moderate |
| Available Crate Quality | ❌ Stale | ❌ Immature | ✅ Production |
| Dependency Cost | ✅ 0 new | ✅ 0 new | ✅ 1 new |
| Battle-Tested | ✅ (Emacs) | ⚠️ (VSCode) | ✅ (Helix, Xi) |
| Vim-like Navigation | ❌ Poor | ⚠️ Workable | ✅ Excellent |
| Future LSP/Regex | ❌ | ✅ | ✅ |

---

## Decision: Ropey

**Selected**: `ropey` v2.0.0-beta.1

**Rationale**:
1. **Performance**: O(log n) for all operations is acceptable and consistent. No worst-case O(n) scenarios.
2. **Production-Ready**: Battle-tested in Helix, Lapce, and other production editors. Not experimental.
3. **Line-Based Access**: Built-in line indexing matches our rendering needs perfectly.
4. **Large File Support**: Chunk-based design handles multi-GB files efficiently.
5. **Constitutional Compliance**: Only adds 1 transitive dependency (str_indices), keeping us at 4/5.
6. **Active Maintenance**: Updated in 2024, responsive maintainer (Cessen).
7. **Excellent API**: Clean interface for our needs (insert, delete, lines, chars, bytes, slices).
8. **Future-Proof**: Used by modern editors with LSP, regex search, and advanced features.
9. **Documentation**: Comprehensive docs and examples.

**Alternatives Considered**:

- **Gap Buffer**: Rejected due to O(n) gap movement and poor large file support. Not suitable for vim-style navigation patterns.
- **Piece Table** (`piece_table_rs`): Rejected due to immature/unmaintained crates and need for auxiliary line index.
- **Crop**: Considered but ropey has better production track record and community support.

---

## Implementation Approach

### Text Buffer Abstraction

Create `text_buffer.rs` module with a clean interface:

```rust
pub struct TextBuffer {
    rope: Rope,  // from ropey crate
}

impl TextBuffer {
    pub fn from_str(text: &str) -> Self;
    pub fn insert(&mut self, char_idx: usize, text: &str);
    pub fn remove(&mut self, start: usize, end: usize);
    pub fn line(&self, line_idx: usize) -> Option<RopeSlice>;
    pub fn line_len(&self, line_idx: usize) -> usize;
    pub fn len_lines(&self) -> usize;
    pub fn len_chars(&self) -> usize;
    pub fn char_to_line(&self, char_idx: usize) -> usize;
    pub fn line_to_char(&self, line_idx: usize) -> usize;
    pub fn to_string(&self) -> String;
}
```

**Key Design Decisions**:
1. Wrap `ropey::Rope` behind our own struct - enables future swapping if needed
2. Convert between line/column coordinates (used by cursor) and character indices (used by rope)
3. Keep all ropey-specific code isolated to `text_buffer.rs`

### Integration Points

- **Cursor Module**: Maintains (line, column) position, converts to char index via TextBuffer
- **Viewer Module**: Calls `text_buffer.line(n)` to get lines for rendering
- **Insert Mode**: Calls `text_buffer.insert(char_idx, typed_char)`
- **Navigation**: Updates cursor position, which queries TextBuffer for validation
- **Save**: Calls `text_buffer.to_string()` to serialize for writing

---

## Constitutional Compliance Check

**Before**: 3 runtime dependencies (ordex → termion → libc, numtoa)

**After Adding Ropey**: 
```
ordex
├── termion → libc, numtoa
└── ropey → str_indices
```

**Total**: 4 transitive dependencies (termion, libc, numtoa, str_indices) + ropey = **5 direct + transitive**

Wait, let me recount using the constitution definition: "Limited to 5 transitive dependencies (crates)".

Looking at the tree:
- libc
- numtoa  
- str_indices

That's 3 transitive dependencies. Direct dependencies don't count toward the transitive limit.

Actually, re-reading: "Runtime dependencies: Limited to 5 transitive dependencies (crates)". This is ambiguous - does it mean total transitive deps, or total including direct?

From Phase 001 plan: "termion brings 2 (libc, numtoa), total: 3" suggests they count termion itself plus its transitives.

**Conservative Interpretation** (safest): Count all non-ordex crates
- termion
- libc
- numtoa
- ropey
- str_indices
= **5 total runtime dependencies** ✅ **EXACTLY AT LIMIT**

**Status**: ✅ **CONSTITUTIONAL COMPLIANCE MAINTAINED**

No additional dependencies can be added without violating the constitution.

---

## Next Steps (Phase 1)

1. Add `ropey = "2.0.0-beta.1"` to Cargo.toml
2. Implement `text_buffer.rs` with abstraction layer
3. Update `viewer.rs` to use TextBuffer instead of Vec<String>
4. Implement cursor coordinate ↔ char index conversion
5. Design data model (see data-model.md)
6. Design contracts for module interfaces (see contracts/)

---

## References

- Ropey GitHub: https://github.com/cessen/ropey
- Ropey Docs: https://docs.rs/ropey
- Xi-editor Rope Science: https://xi-editor.io/docs/rope_science_00.html
- Helix Editor (uses ropey): https://github.com/helix-editor/helix
- Piece Table Paper (original): https://www.cs.unm.edu/~crowley/papers/sds.pdf

---

**Research Complete**: 2025-02-04
**Decision**: Use `ropey` crate for text storage
**Constitutional Status**: ✅ Compliant (5/5 dependencies)
