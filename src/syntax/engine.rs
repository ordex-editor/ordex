//! Incremental syntax-highlighting engine.
//!
//! The engine keeps editor-owned derived state for the currently open document
//! and re-lexes forward from the first dirty line until the line-exit state
//! stabilizes again.

use crate::syntax::profile::{LanguageId, LanguageProfile, SyntaxClass, SyntaxModifier};
use crate::syntax::profiles::detect_language_details;
use crate::text_buffer::TextBuffer;
use std::cmp::Ordering;
use std::path::Path;

/// One styled region within a logical buffer line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightSpan {
    /// Source line index.
    pub(crate) line_index: usize,
    /// Inclusive start column.
    pub(crate) start_col: usize,
    /// Exclusive end column.
    pub(crate) end_col: usize,
    /// Semantic syntax class for this span.
    pub(crate) class: SyntaxClass,
    /// Optional semantic modifier layered on top of the class.
    pub(crate) modifier: Option<SyntaxModifier>,
}

impl HighlightSpan {
    /// Return whether this span covers `column`.
    pub(crate) fn covers(&self, column: usize) -> bool {
        (self.start_col..self.end_col).contains(&column)
    }
}

/// Edit-range description passed from editor mutations into the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BufferEdit {
    /// First affected logical line.
    pub(crate) start_line: usize,
    /// Last affected logical line before the edit.
    pub(crate) old_end_line: usize,
    /// Last affected logical line after the edit.
    pub(crate) new_end_line: usize,
}

/// How the active profile was detected, or that plain fallback was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetectionSource {
    /// Detection matched an exact filename.
    MatchByFilename,
    /// Detection matched a file extension.
    MatchByExtension,
    /// No profile matched and rendering fell back to plain text.
    PlainFallback,
}

/// Carry-over lexer state inherited from the previous line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LineLexMode {
    /// No multiline construct is currently open.
    #[default]
    Plain,
    /// Rust nested block comment state.
    RustBlockComment {
        /// Current block nesting depth.
        depth: usize,
        /// Whether this block comment is documentation-flavored.
        doc: bool,
    },
    /// Rust raw-string state carried across lines.
    RustRawString {
        /// Number of `#` markers used by the raw-string delimiter.
        hashes: usize,
    },
    /// TOML multiline basic string state.
    TomlBasicMultiString,
    /// TOML multiline literal string state.
    TomlLiteralMultiString,
    /// D block comment state.
    DBlockComment {
        /// Whether the comment uses `/+ +/` nesting rules.
        nested: bool,
        /// Current block nesting depth.
        depth: usize,
        /// Whether this block comment is documentation-flavored.
        doc: bool,
    },
    /// Markdown fenced-code block state.
    MarkdownFence {
        /// Fence marker character, either `` ` `` or `~`.
        marker: char,
        /// Minimum fence length required to close the block.
        count: usize,
    },
}

/// Per-line lex result used by profile callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineParseResult {
    /// Ordered, non-overlapping line-local spans.
    pub(crate) spans: Vec<HighlightSpan>,
    /// Exit mode inherited by the next logical line.
    pub(crate) exit_mode: LineLexMode,
}

/// Cached state for one logical line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineLexState {
    /// Source line index.
    pub(crate) line_index: usize,
    /// Entry mode inherited from the previous line.
    pub(crate) entry_mode: LineLexMode,
    /// Exit mode produced after lexing this line.
    pub(crate) exit_mode: LineLexMode,
    /// Generation number that produced this state.
    pub(crate) revision: u64,
    /// Whether this line is currently stable with respect to its entry mode.
    pub(crate) stable: bool,
}

impl Default for LineLexState {
    /// Build a plain, stable line state.
    fn default() -> Self {
        Self {
            line_index: 0,
            entry_mode: LineLexMode::Plain,
            exit_mode: LineLexMode::Plain,
            revision: 0,
            stable: true,
        }
    }
}

/// Highlight state for the current document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentHighlightState {
    /// Currently active profile, if any.
    pub(crate) active_profile: Option<LanguageId>,
    /// Source of the active detection result.
    pub(crate) detection_source: DetectionSource,
    /// Cached per-line entry and exit modes.
    pub(crate) line_states: Vec<LineLexState>,
    /// Cached per-line spans.
    pub(crate) spans_by_line: Vec<Vec<HighlightSpan>>,
    /// First dirty line waiting for relexing, if any.
    pub(crate) dirty_start_line: Option<usize>,
    /// Monotonic syntax-generation counter.
    pub(crate) generation: u64,
    /// Whether the document has reached full lex correctness.
    pub(crate) fully_lexed: bool,
}

impl Default for DocumentHighlightState {
    /// Build an empty plain-text highlight state.
    fn default() -> Self {
        Self {
            active_profile: None,
            detection_source: DetectionSource::PlainFallback,
            line_states: vec![LineLexState::default()],
            spans_by_line: vec![Vec::new()],
            dirty_start_line: None,
            generation: 0,
            fully_lexed: true,
        }
    }
}

/// Stateful syntax-highlighting engine owned by `EditorState`.
#[derive(Debug, Clone, Default)]
pub(crate) struct SyntaxEngine {
    document: DocumentHighlightState,
}

impl SyntaxEngine {
    /// Create a fresh syntax engine with plain fallback state.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Open a document, detect its profile, and fully lex it top-to-bottom.
    pub(crate) fn open_document(&mut self, path: Option<&Path>, buffer: &TextBuffer) {
        self.document.generation = self.document.generation.saturating_add(1);
        let line_count = buffer.lines_count().max(1);
        self.document.line_states = vec![LineLexState::default(); line_count];
        self.document.spans_by_line = vec![Vec::new(); line_count];
        self.document.dirty_start_line = None;
        match detect_language_details(path) {
            Some((profile, source)) => {
                self.document.active_profile = Some(profile.id);
                self.document.detection_source = source;
                self.lex_all(buffer, profile);
            }
            None => {
                self.document.active_profile = None;
                self.document.detection_source = DetectionSource::PlainFallback;
                self.clear_plain_state(line_count);
            }
        }
    }

    /// Apply one buffer edit and synchronously re-lex until the state stabilizes.
    pub(crate) fn apply_edit(&mut self, buffer: &TextBuffer, edit: BufferEdit) {
        self.document.generation = self.document.generation.saturating_add(1);
        self.document.dirty_start_line = Some(edit.start_line);
        self.splice_line_caches(edit);
        match self.active_profile_definition() {
            Some(profile) => self.relex_from(buffer, profile, edit),
            None => self.clear_plain_state(buffer.lines_count().max(1)),
        }
    }

    /// Return the active language identifier, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn active_profile(&self) -> Option<LanguageId> {
        self.document.active_profile
    }

    /// Borrow ordered highlight spans for one line.
    pub(crate) fn spans_for_line(&self, line_index: usize) -> &[HighlightSpan] {
        self.document
            .spans_by_line
            .get(line_index)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return the current syntax-generation number.
    pub(crate) fn generation(&self) -> u64 {
        self.document.generation
    }

    /// Return whether the current document state is fully lexed and stable.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn is_fully_lexed(&self) -> bool {
        self.document.fully_lexed
    }

    /// Return a shared reference to the full document state.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn document_state(&self) -> &DocumentHighlightState {
        &self.document
    }

    /// Replace the document with plain fallback state sized to `line_count`.
    fn clear_plain_state(&mut self, line_count: usize) {
        self.document.line_states = (0..line_count)
            .map(|line_index| LineLexState {
                line_index,
                revision: self.document.generation,
                ..LineLexState::default()
            })
            .collect();
        self.document.spans_by_line = vec![Vec::new(); line_count];
        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Return the built-in definition for the active language id.
    fn active_profile_definition(&self) -> Option<&'static LanguageProfile> {
        let active_id = self.document.active_profile?;
        crate::syntax::profiles::builtin_profiles()
            .iter()
            .find(|profile| profile.id == active_id)
    }

    /// Fully lex the current buffer from the first line to the last line.
    fn lex_all(&mut self, buffer: &TextBuffer, profile: &'static LanguageProfile) {
        let mut entry_mode = LineLexMode::Plain;
        let revision = self.document.generation;
        let line_count = buffer.lines_count().max(1);

        // Full-document lexing guarantees correct inherited state for multiline
        // constructs before the first frame is rendered.
        for line_index in 0..line_count {
            let line = buffer.line_for_display(line_index).unwrap_or_default();
            let mut parsed = (profile.lex_line)(&line, entry_mode);
            for span in &mut parsed.spans {
                span.line_index = line_index;
            }
            self.document.line_states[line_index] = LineLexState {
                line_index,
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision,
                stable: true,
            };
            self.document.spans_by_line[line_index] = parsed.spans;
            entry_mode = self.document.line_states[line_index].exit_mode;
        }

        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Re-lex from the first dirty line until the carried state stabilizes.
    fn relex_from(
        &mut self,
        buffer: &TextBuffer,
        profile: &'static LanguageProfile,
        edit: BufferEdit,
    ) {
        let line_count = buffer.lines_count().max(1);
        let start_line = edit.start_line.min(line_count.saturating_sub(1));
        let min_relex_line = edit.new_end_line.min(line_count.saturating_sub(1));
        let mut entry_mode = if start_line == 0 {
            LineLexMode::Plain
        } else {
            self.document.line_states[start_line - 1].exit_mode
        };
        let revision = self.document.generation;
        self.document.fully_lexed = false;

        // Continue until the edited region and any dependent multiline state have
        // both stabilized. Unchanged tail lines can keep their cached spans.
        for line_index in start_line..line_count {
            let line = buffer.line_for_display(line_index).unwrap_or_default();
            let previous_spans = self.document.spans_by_line[line_index].clone();
            let previous_exit = self.document.line_states[line_index].exit_mode;
            let mut parsed = (profile.lex_line)(&line, entry_mode);
            for span in &mut parsed.spans {
                span.line_index = line_index;
            }
            let unchanged = previous_spans == parsed.spans && previous_exit == parsed.exit_mode;

            self.document.line_states[line_index] = LineLexState {
                line_index,
                entry_mode,
                exit_mode: parsed.exit_mode,
                revision,
                stable: true,
            };
            self.document.spans_by_line[line_index] = parsed.spans;
            entry_mode = self.document.line_states[line_index].exit_mode;

            if line_index >= min_relex_line && unchanged {
                break;
            }
        }

        self.document.dirty_start_line = None;
        self.document.fully_lexed = true;
    }

    /// Splice cached line vectors to keep unchanged tail lines aligned after edits.
    fn splice_line_caches(&mut self, edit: BufferEdit) {
        let required_len = edit.old_end_line.saturating_add(1);
        if self.document.line_states.len() < required_len {
            self.document
                .line_states
                .resize(required_len, LineLexState::default());
        }
        if self.document.spans_by_line.len() < required_len {
            self.document.spans_by_line.resize(required_len, Vec::new());
        }
        let old_count = edit
            .old_end_line
            .saturating_sub(edit.start_line)
            .saturating_add(1);
        let new_count = edit
            .new_end_line
            .saturating_sub(edit.start_line)
            .saturating_add(1);
        match new_count.cmp(&old_count) {
            Ordering::Greater => {
                let diff = new_count - old_count;
                let insert_at = edit
                    .old_end_line
                    .saturating_add(1)
                    .min(self.document.line_states.len());
                for _ in 0..diff {
                    self.document
                        .line_states
                        .insert(insert_at, LineLexState::default());
                    self.document.spans_by_line.insert(insert_at, Vec::new());
                }
            }
            Ordering::Less => {
                let remove_start = edit.start_line.saturating_add(new_count);
                let remove_end = edit
                    .old_end_line
                    .saturating_add(1)
                    .min(self.document.line_states.len());
                if remove_start < remove_end {
                    self.document.line_states.drain(remove_start..remove_end);
                    self.document.spans_by_line.drain(remove_start..remove_end);
                }
            }
            Ordering::Equal => {}
        }

        for (line_index, state) in self.document.line_states.iter_mut().enumerate() {
            state.line_index = line_index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferEdit, LineLexMode, SyntaxEngine};
    use crate::text_buffer::TextBuffer;
    use std::path::Path;

    /// Verify that supported files are fully lexed on open.
    #[test]
    fn test_open_document_lexes_supported_file() {
        let buffer = TextBuffer::from_str("fn main() {\n    let x = 42;\n}\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        assert!(engine.is_fully_lexed());
        assert_eq!(
            engine.active_profile(),
            Some(crate::syntax::profile::LanguageId::Rust)
        );
        assert!(
            !engine.spans_for_line(0).is_empty(),
            "rust open should produce spans"
        );
    }

    /// Verify that unsupported files stay in plain fallback mode.
    #[test]
    fn test_open_document_falls_back_to_plain_text() {
        let buffer = TextBuffer::from_str("plain text only");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("notes.txt")), &buffer);
        assert_eq!(engine.active_profile(), None);
        assert!(engine.spans_for_line(0).is_empty());
    }

    /// Verify that forward relex stabilizes after a block comment closes.
    #[test]
    fn test_relex_stabilizes_after_multiline_comment_edit() {
        let mut buffer = TextBuffer::from_str("/* open\nstill open\n");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.rs")), &buffer);
        assert_eq!(
            engine.document_state().line_states[1].exit_mode,
            LineLexMode::RustBlockComment {
                depth: 1,
                doc: false
            }
        );

        buffer.insert(buffer.chars_count(), "*/\n");
        engine.apply_edit(
            &buffer,
            BufferEdit {
                start_line: 1,
                old_end_line: 1,
                new_end_line: 2,
            },
        );

        assert_eq!(
            engine.document_state().line_states[2].exit_mode,
            LineLexMode::Plain
        );
    }

    /// Verify that nested D block comments retain depth correctly.
    #[test]
    fn test_nested_d_comment_depth_is_preserved() {
        let buffer = TextBuffer::from_str("/+ outer\n/+ inner +/\nstill outer\n+/");
        let mut engine = SyntaxEngine::new();
        engine.open_document(Some(Path::new("sample.d")), &buffer);
        assert_eq!(
            engine.document_state().line_states[1].exit_mode,
            LineLexMode::DBlockComment {
                nested: true,
                depth: 1,
                doc: false
            }
        );
        assert_eq!(
            engine.document_state().line_states[2].exit_mode,
            LineLexMode::DBlockComment {
                nested: true,
                depth: 1,
                doc: false
            }
        );
    }
}
