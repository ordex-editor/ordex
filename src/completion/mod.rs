//! Completion-session state, request modeling, and popup helpers.

pub(crate) mod buffer_source;

use crate::dialogs::{PickerPopup, PickerPopupEntry};
use crate::navigation::is_word_char;
use crate::text_buffer::TextBuffer;

/// Minimum visible candidate length for the MVP buffer-text source.
pub(crate) const MIN_CANDIDATE_LENGTH: usize = 3;

/// Identify one completion provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CompletionSourceId {
    BufferText,
    FilePath,
}

impl CompletionSourceId {
    /// Return the stable external identifier for one source.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BufferText => "buffer-text",
            Self::FilePath => "file-path",
        }
    }
}

/// Describe whether one completion source runs inline or asynchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionSourceKind {
    Synchronous,
    Asynchronous,
}

/// Static metadata for one registered completion source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompletionSourceMeta {
    pub(crate) source_id: CompletionSourceId,
    pub(crate) kind: CompletionSourceKind,
    pub(crate) enabled: bool,
    pub(crate) priority: usize,
}

/// Track the known completion sources available to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionSourceRegistry {
    sources: Vec<CompletionSourceMeta>,
}

impl CompletionSourceRegistry {
    /// Build the default registry for the current MVP.
    pub(crate) fn new() -> Self {
        let mut registry = Self {
            sources: Vec::new(),
        };
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::BufferText,
            kind: CompletionSourceKind::Synchronous,
            enabled: true,
            priority: 0,
        });
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::FilePath,
            kind: CompletionSourceKind::Asynchronous,
            enabled: false,
            priority: 1,
        });
        registry
    }

    /// Register or replace one source definition by source id.
    pub(crate) fn register(&mut self, source: CompletionSourceMeta) {
        if let Some(index) = self
            .sources
            .iter()
            .position(|existing| existing.source_id == source.source_id)
        {
            self.sources[index] = source;
        } else {
            self.sources.push(source);
        }

        // Stable source priority keeps future multi-source ordering predictable.
        self.sources
            .sort_by_key(|entry| (entry.priority, entry.source_id.as_str()));
    }

    /// Return the enabled source ids in priority order.
    pub(crate) fn enabled_source_ids(&self) -> Vec<CompletionSourceId> {
        self.sources
            .iter()
            .filter(|source| source.enabled)
            .map(|source| source.source_id)
            .collect()
    }

    /// Return whether the buffer-text source is currently enabled.
    pub(crate) fn buffer_text_enabled(&self) -> bool {
        self.sources.iter().any(|source| {
            source.enabled && matches!(source.source_id, CompletionSourceId::BufferText)
        })
    }
}

impl Default for CompletionSourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Describe what triggered one completion refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionTriggerKind {
    Automatic,
}

/// Capture the current prefix under the insert cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionRequest {
    pub(crate) buffer_id: usize,
    pub(crate) request_generation: usize,
    pub(crate) trigger_kind: CompletionTriggerKind,
    pub(crate) prefix_start_char_idx: usize,
    pub(crate) cursor_char_idx: usize,
    pub(crate) prefix_text: String,
    pub(crate) normalized_prefix: String,
    pub(crate) min_candidate_length: usize,
}

/// Describe one candidate returned by a completion source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionCandidate {
    pub(crate) source_id: CompletionSourceId,
    pub(crate) insert_text: String,
    pub(crate) replace_start_char_idx: usize,
    pub(crate) replace_end_char_idx: usize,
    pub(crate) rank: usize,
}

/// Describe whether one session is still visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionState {
    Active,
}

/// Track the active completion popup and live preview state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionSession {
    pub(crate) buffer_id: usize,
    pub(crate) request_generation: usize,
    pub(crate) prefix_start_char_idx: usize,
    pub(crate) cursor_char_idx: usize,
    pub(crate) selected_index: Option<usize>,
    pub(crate) candidates: Vec<CompletionCandidate>,
    pub(crate) original_prefix_text: String,
    pub(crate) source_ids: Vec<CompletionSourceId>,
    pub(crate) state: CompletionState,
}

impl CompletionSession {
    /// Build one active session from the current request and candidate set.
    pub(crate) fn new(
        request: CompletionRequest,
        candidates: Vec<CompletionCandidate>,
        source_ids: Vec<CompletionSourceId>,
    ) -> Self {
        let _trigger_kind = request.trigger_kind;
        Self {
            buffer_id: request.buffer_id,
            request_generation: request.request_generation,
            prefix_start_char_idx: request.prefix_start_char_idx,
            cursor_char_idx: request.cursor_char_idx,
            selected_index: None,
            candidates,
            original_prefix_text: request.prefix_text,
            source_ids,
            state: CompletionState::Active,
        }
    }

    /// Return the text that currently occupies the replacement span in the buffer.
    pub(crate) fn current_text(&self) -> &str {
        self.selected_index
            .and_then(|index| self.candidates.get(index))
            .map(|candidate| candidate.insert_text.as_str())
            .unwrap_or(self.original_prefix_text.as_str())
    }

    /// Return the current replacement end based on the visible preview text.
    pub(crate) fn replacement_end_char_idx(&self) -> usize {
        self.prefix_start_char_idx + self.current_text().chars().count()
    }

    /// Move the completion selection and update the live-preview text metadata.
    pub(crate) fn move_selection(&mut self, direction: CompletionDirection) {
        if self.candidates.is_empty() {
            self.selected_index = None;
            return;
        }

        self.selected_index = match (direction, self.selected_index) {
            (CompletionDirection::Down, None) => Some(0),
            (CompletionDirection::Down, Some(index)) if index + 1 < self.candidates.len() => {
                Some(index + 1)
            }
            (CompletionDirection::Down, Some(_)) => None,
            (CompletionDirection::Up, None) => Some(self.candidates.len().saturating_sub(1)),
            (CompletionDirection::Up, Some(index)) if index > 0 => Some(index - 1),
            (CompletionDirection::Up, Some(_)) => None,
        };
    }

    /// Return the current request identity that must match to avoid a rescan.
    pub(crate) fn matches_request(&self, request: &CompletionRequest) -> bool {
        self.buffer_id == request.buffer_id
            && self.prefix_start_char_idx == request.prefix_start_char_idx
            && self.cursor_char_idx == request.cursor_char_idx
            && self.original_prefix_text == request.prefix_text
    }

    /// Build the shared picker-style popup view for this session.
    pub(crate) fn popup(&self, visible_entry_capacity: usize) -> PickerPopup {
        let entry_count = visible_entry_capacity.max(1);
        let suffix = self
            .source_ids
            .iter()
            .map(|source_id| source_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let entries = self
            .candidates
            .iter()
            .enumerate()
            .take(entry_count)
            .map(|(index, candidate)| PickerPopupEntry {
                label: candidate.insert_text.clone(),
                selected: self.selected_index == Some(index),
                active: false,
                modified: false,
            })
            .collect();
        PickerPopup {
            title: "Completion".to_string(),
            query_label: " Prefix: ".to_string(),
            query_suffix: suffix,
            empty_message: "No completions".to_string(),
            query: self.original_prefix_text.clone(),
            cursor_column: self.original_prefix_text.chars().count(),
            entries,
        }
    }
}

/// Describe one keyboard direction for completion navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionDirection {
    Up,
    Down,
}

/// Build one completion request from the cursor position when a prefix is active.
pub(crate) fn build_request(
    buffer: &TextBuffer,
    buffer_id: usize,
    cursor_char_idx: usize,
    request_generation: usize,
) -> Option<CompletionRequest> {
    if cursor_char_idx == 0 {
        return None;
    }

    let previous_idx = cursor_char_idx.saturating_sub(1);
    if !buffer.char_at(previous_idx).is_some_and(is_word_char) {
        return None;
    }

    let mut start = previous_idx;
    // Walk backward through the contiguous word run so the replace range matches the prefix.
    while start > 0 && buffer.char_at(start - 1).is_some_and(is_word_char) {
        start -= 1;
    }
    let prefix_text = buffer.slice_string(start, cursor_char_idx);
    if prefix_text.is_empty() {
        return None;
    }

    Some(CompletionRequest {
        buffer_id,
        request_generation,
        trigger_kind: CompletionTriggerKind::Automatic,
        prefix_start_char_idx: start,
        cursor_char_idx,
        normalized_prefix: normalize_text(&prefix_text),
        prefix_text,
        min_candidate_length: MIN_CANDIDATE_LENGTH,
    })
}

/// Recompute the active session from the current request and source registry.
pub(crate) fn refresh_session(
    registry: &CompletionSourceRegistry,
    buffer: &TextBuffer,
    request: CompletionRequest,
) -> Option<CompletionSession> {
    let source_ids = registry.enabled_source_ids();
    if source_ids.is_empty() {
        return None;
    }

    let mut candidates = Vec::new();
    if registry.buffer_text_enabled() {
        candidates.extend(buffer_source::collect_buffer_candidates(&request, buffer));
    }
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by_key(|candidate| (candidate.rank, candidate.source_id.as_str()));
    Some(CompletionSession::new(request, candidates, source_ids))
}

/// Normalize one text value for case-insensitive comparisons.
pub(crate) fn normalize_text(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one test request from plain text and a character index.
    fn request_for(text: &str, cursor_char_idx: usize) -> CompletionRequest {
        let buffer = TextBuffer::from_str(text);
        build_request(&buffer, 7, cursor_char_idx, 3).expect("request should exist")
    }

    #[test]
    /// Confirm request building uses the contiguous word left of the cursor.
    fn test_build_request_uses_current_word_prefix() {
        let request = request_for("alpha beta", 2);

        assert_eq!(request.prefix_start_char_idx, 0);
        assert_eq!(request.cursor_char_idx, 2);
        assert_eq!(request.prefix_text, "al");
        assert_eq!(request.normalized_prefix, "al");
    }

    #[test]
    /// Confirm non-word positions do not create completion requests.
    fn test_build_request_requires_word_character_before_cursor() {
        let buffer = TextBuffer::from_str("alpha beta");

        assert!(build_request(&buffer, 1, 0, 1).is_none());
        assert!(build_request(&buffer, 1, 6, 1).is_none());
    }

    #[test]
    /// Confirm selection movement keeps the required no-selection restoration state.
    fn test_completion_selection_cycles_through_none_state() {
        let request = request_for("alpha", 2);
        let mut session = CompletionSession::new(
            request,
            vec![
                CompletionCandidate {
                    source_id: CompletionSourceId::BufferText,
                    insert_text: "alpha".to_string(),
                    replace_start_char_idx: 0,
                    replace_end_char_idx: 2,
                    rank: 0,
                },
                CompletionCandidate {
                    source_id: CompletionSourceId::BufferText,
                    insert_text: "alphabet".to_string(),
                    replace_start_char_idx: 0,
                    replace_end_char_idx: 2,
                    rank: 1,
                },
            ],
            vec![CompletionSourceId::BufferText],
        );

        session.move_selection(CompletionDirection::Down);
        assert_eq!(session.selected_index, Some(0));
        assert_eq!(session.current_text(), "alpha");

        session.move_selection(CompletionDirection::Down);
        assert_eq!(session.selected_index, Some(1));

        session.move_selection(CompletionDirection::Down);
        assert_eq!(session.selected_index, None);
        assert_eq!(session.current_text(), "al");

        session.move_selection(CompletionDirection::Up);
        assert_eq!(session.selected_index, Some(1));
    }

    #[test]
    /// Confirm source registration updates existing metadata in place.
    fn test_source_registry_replaces_existing_registration() {
        let mut registry = CompletionSourceRegistry::new();
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::BufferText,
            kind: CompletionSourceKind::Synchronous,
            enabled: false,
            priority: 2,
        });
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::FilePath,
            kind: CompletionSourceKind::Asynchronous,
            enabled: true,
            priority: 1,
        });

        assert_eq!(
            registry.enabled_source_ids(),
            vec![CompletionSourceId::FilePath]
        );
        assert!(!registry.buffer_text_enabled());
    }

    #[test]
    /// Confirm request matching depends only on the stable session identity inputs.
    fn test_matches_request_uses_request_identity_fields() {
        let request = request_for("alpha", 2);
        let session = CompletionSession::new(
            request.clone(),
            Vec::new(),
            vec![CompletionSourceId::BufferText],
        );

        assert!(session.matches_request(&request));
    }
}
