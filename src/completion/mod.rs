//! Completion request modeling, source coordination, and popup helpers.

pub(crate) mod buffer_source;
pub(crate) mod file_path_source;

use crate::navigation::is_word_char;
use crate::text_buffer::TextBuffer;
use std::path::{Path, PathBuf};

/// Minimum visible candidate length for the buffer-text source.
pub(crate) const MIN_CANDIDATE_LENGTH: usize = 3;
/// Upper bound on visible buffer-word candidates in one popup.
pub(crate) const MAX_CANDIDATES: usize = 64;
/// Upper bound on visible file-path candidates in one popup.
pub(crate) const MAX_FILE_PATH_CANDIDATES: usize = 500;

/// Identify one completion provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CompletionSourceId {
    BufferText,
    FilePath,
    Lsp,
}

impl CompletionSourceId {
    /// Return the stable external identifier for one source.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BufferText => "buffer-text",
            Self::FilePath => "file-path",
            Self::Lsp => "lsp",
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
    /// Build the default registry for the current completion sources.
    pub(crate) fn new() -> Self {
        let mut registry = Self {
            sources: Vec::new(),
        };
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::FilePath,
            kind: CompletionSourceKind::Asynchronous,
            enabled: true,
            priority: 0,
        });
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::Lsp,
            kind: CompletionSourceKind::Asynchronous,
            enabled: true,
            priority: 1,
        });
        registry.register(CompletionSourceMeta {
            source_id: CompletionSourceId::BufferText,
            kind: CompletionSourceKind::Synchronous,
            enabled: true,
            priority: 2,
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
    #[cfg(test)]
    pub(crate) fn enabled_source_ids(&self) -> Vec<CompletionSourceId> {
        self.sources
            .iter()
            .filter(|source| source.enabled)
            .map(|source| source.source_id)
            .collect()
    }

    /// Return whether `source_id` is currently enabled.
    fn source_enabled(&self, source_id: CompletionSourceId) -> bool {
        self.sources
            .iter()
            .any(|source| source.enabled && source.source_id == source_id)
    }

    /// Return whether the buffer-text source is currently enabled.
    pub(crate) fn buffer_text_enabled(&self) -> bool {
        self.source_enabled(CompletionSourceId::BufferText)
    }

    /// Return whether the file-path source is currently enabled.
    pub(crate) fn file_path_enabled(&self) -> bool {
        self.source_enabled(CompletionSourceId::FilePath)
    }

    /// Return whether the LSP source is currently enabled.
    pub(crate) fn lsp_enabled(&self) -> bool {
        self.source_enabled(CompletionSourceId::Lsp)
    }

    /// Return the configured priority for `source_id`.
    pub(crate) fn source_priority(&self, source_id: CompletionSourceId) -> usize {
        self.sources
            .iter()
            .find(|source| source.source_id == source_id)
            .map_or(usize::MAX, |source| source.priority)
    }
}

impl Default for CompletionSourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsed file-path request metadata for one completion run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilePathCompletionRequest {
    display_prefix: String,
    segment_prefix: String,
    normalized_segment_prefix: String,
    resolved_directory: PathBuf,
}

impl FilePathCompletionRequest {
    /// Return the already-typed directory prefix preserved in inserted candidates.
    pub(crate) fn display_prefix(&self) -> &str {
        &self.display_prefix
    }

    /// Return the typed basename fragment matched against directory entries.
    pub(crate) fn segment_prefix(&self) -> &str {
        &self.segment_prefix
    }

    /// Return the normalized basename fragment used for case-insensitive matching.
    pub(crate) fn normalized_segment_prefix(&self) -> &str {
        &self.normalized_segment_prefix
    }

    /// Return the resolved directory scanned for this file-path request.
    pub(crate) fn resolved_directory(&self) -> &Path {
        &self.resolved_directory
    }
}

/// Parsed request context shared by all completion sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletionRequestContext {
    Word {
        prefix_text: String,
        normalized_prefix: String,
    },
    FilePath(FilePathCompletionRequest),
}

impl CompletionRequestContext {
    /// Return the typed text matched against source candidates.
    pub(crate) fn match_prefix(&self) -> &str {
        match self {
            Self::Word { prefix_text, .. } => prefix_text,
            Self::FilePath(path) => path.segment_prefix(),
        }
    }

    /// Return the normalized prefix matched against source candidates.
    pub(crate) fn normalized_match_prefix(&self) -> &str {
        match self {
            Self::Word {
                normalized_prefix, ..
            } => normalized_prefix,
            Self::FilePath(path) => path.normalized_segment_prefix(),
        }
    }

    /// Return whether this request represents an explicit file-path fragment.
    pub(crate) fn is_file_path(&self) -> bool {
        matches!(self, Self::FilePath(_))
    }

    /// Return the file-path metadata when this request targets a path.
    pub(crate) fn file_path_request(&self) -> Option<&FilePathCompletionRequest> {
        match self {
            Self::FilePath(path) => Some(path),
            Self::Word { .. } => None,
        }
    }

    /// Build the inserted text for one matched segment.
    pub(crate) fn compose_insert_text(&self, matched_text: &str) -> String {
        match self {
            Self::Word { .. } => matched_text.to_string(),
            Self::FilePath(path) => format!("{}{}", path.display_prefix(), matched_text),
        }
    }
}

/// Stable request fields used to compare one logical completion query.
///
/// The request generation changes every refresh, but the identity stays the
/// same while the buffer range, typed text, and parsed completion context are
/// unchanged. That lets the editor keep visible popup state and reject stale
/// asynchronous results after the user has typed something different.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionRequestIdentity {
    pub(crate) replace_start_char_idx: usize,
    pub(crate) cursor_char_idx: usize,
    pub(crate) original_text: String,
    pub(crate) context: CompletionRequestContext,
}

/// Capture one completion request bound to an active buffer and generation token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionRequest {
    pub(crate) buffer_id: usize,
    pub(crate) request_generation: usize,
    identity: CompletionRequestIdentity,
    min_candidate_length: usize,
}

impl CompletionRequest {
    /// Bind `identity` to the current active buffer and generation token.
    pub(crate) fn new(
        buffer_id: usize,
        request_generation: usize,
        identity: CompletionRequestIdentity,
    ) -> Self {
        // Word completion keeps its original 3-character threshold, while
        // explicit path prefixes may list directory entries immediately.
        let min_candidate_length = if identity.context.is_file_path() {
            identity.context.match_prefix().chars().count()
        } else {
            MIN_CANDIDATE_LENGTH
        };
        Self {
            buffer_id,
            request_generation,
            identity,
            min_candidate_length,
        }
    }

    /// Return the typed replacement start for this request.
    pub(crate) fn replace_start_char_idx(&self) -> usize {
        self.identity.replace_start_char_idx
    }

    /// Return the cursor character index where the request was captured.
    pub(crate) fn cursor_char_idx(&self) -> usize {
        self.identity.cursor_char_idx
    }

    /// Return the original typed text preserved when no candidate is selected.
    pub(crate) fn original_text(&self) -> &str {
        &self.identity.original_text
    }

    /// Return the typed text matched against source candidates.
    pub(crate) fn match_prefix(&self) -> &str {
        self.identity.context.match_prefix()
    }

    /// Return the normalized typed prefix matched against source candidates.
    pub(crate) fn normalized_match_prefix(&self) -> &str {
        self.identity.context.normalized_match_prefix()
    }

    /// Return the minimum visible candidate length for this request.
    pub(crate) fn min_candidate_length(&self) -> usize {
        self.min_candidate_length
    }

    /// Return the maximum visible candidate count for this request.
    pub(crate) fn max_candidate_count(&self) -> usize {
        if self.is_file_path() {
            MAX_FILE_PATH_CANDIDATES
        } else {
            MAX_CANDIDATES
        }
    }

    /// Return whether this request targets an explicit file-path fragment.
    pub(crate) fn is_file_path(&self) -> bool {
        self.identity.context.is_file_path()
    }

    /// Return the file-path metadata when this request targets a path.
    pub(crate) fn file_path_request(&self) -> Option<&FilePathCompletionRequest> {
        self.identity.context.file_path_request()
    }

    /// Return whether `identity` belongs to the same active request.
    pub(crate) fn matches_identity(
        &self,
        buffer_id: usize,
        identity: &CompletionRequestIdentity,
    ) -> bool {
        self.buffer_id == buffer_id && self.identity == *identity
    }

    /// Build the replacement text for one matched source segment.
    pub(crate) fn compose_insert_text(&self, matched_text: &str) -> String {
        self.identity.context.compose_insert_text(matched_text)
    }
}

/// Describe one candidate returned by a completion source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionCandidate {
    pub(crate) source_id: CompletionSourceId,
    pub(crate) insert_text: String,
    pub(crate) popup_label: String,
    pub(crate) popup_detail: Option<&'static str>,
    pub(crate) replace_start_char_idx: usize,
    pub(crate) replace_end_char_idx: usize,
    pub(crate) rank: usize,
}

/// One completion entry rendered in the inline popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionPopupEntry {
    pub(crate) label: String,
    pub(crate) detail: Option<&'static str>,
    pub(crate) selected: bool,
}

/// Entry-only popup model for cursor-anchored completion suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionPopup {
    pub(crate) anchor_char_idx: usize,
    pub(crate) entries: Vec<CompletionPopupEntry>,
    pub(crate) reserved_entry_count: usize,
    pub(crate) reserved_inner_width: usize,
}

/// Describe whether one session is still visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionState {
    Active,
}

/// Track the active completion popup and live preview state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionSession {
    request: CompletionRequest,
    pub(crate) popup_anchor_char_idx: usize,
    pub(crate) selected_index: Option<usize>,
    pub(crate) candidates: Vec<CompletionCandidate>,
    reserved_entry_count: usize,
    reserved_inner_width: usize,
    pub(crate) state: CompletionState,
}

impl CompletionSession {
    /// Build one active session from the current request and candidate set.
    pub(crate) fn new(
        request: CompletionRequest,
        candidates: Vec<CompletionCandidate>,
        popup_anchor_char_idx: usize,
    ) -> Self {
        let reserved_entry_count = candidates.len();
        let reserved_inner_width = completion_popup_inner_width_for_candidates(&candidates);
        Self {
            request,
            popup_anchor_char_idx,
            selected_index: None,
            candidates,
            reserved_entry_count,
            reserved_inner_width,
            state: CompletionState::Active,
        }
    }

    /// Borrow the request that produced this session.
    pub(crate) fn request(&self) -> &CompletionRequest {
        &self.request
    }

    /// Return the text that currently occupies the replacement span in the buffer.
    pub(crate) fn current_text(&self) -> &str {
        self.selected_index
            .and_then(|index| self.candidates.get(index))
            .map(|candidate| candidate.insert_text.as_str())
            .unwrap_or(self.request.original_text())
    }

    /// Return the active replacement start based on the visible preview text.
    pub(crate) fn current_replace_start_char_idx(&self) -> usize {
        self.selected_index
            .and_then(|index| self.candidates.get(index))
            .map_or(self.request.replace_start_char_idx(), |candidate| {
                candidate.replace_start_char_idx
            })
    }

    /// Return the current replacement end based on the visible preview text.
    pub(crate) fn replacement_end_char_idx(&self) -> usize {
        self.current_replace_start_char_idx() + self.current_text().chars().count()
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

    /// Return whether `identity` belongs to the request that produced this session.
    pub(crate) fn matches_identity(
        &self,
        buffer_id: usize,
        identity: &CompletionRequestIdentity,
    ) -> bool {
        self.request.matches_identity(buffer_id, identity)
    }

    /// Replace the visible candidate list while preserving the active selection when possible.
    ///
    /// Returns `true` when the selected preview text changed and the editor must
    /// rewrite the buffer preview, and `false` when the visible preview already
    /// matches the preserved selection state.
    pub(crate) fn replace_candidates(&mut self, candidates: Vec<CompletionCandidate>) -> bool {
        let previous_selected = self
            .selected_index
            .and_then(|index| self.candidates.get(index))
            .map(|candidate| (candidate.source_id, candidate.insert_text.clone()));
        let preview_before = self.current_text().to_string();

        // Preserve the selected item by source id plus inserted text because
        // sorting and deduplication can change ranks between refresh passes.
        self.candidates = candidates;
        self.selected_index = previous_selected.and_then(|(source_id, insert_text)| {
            self.candidates.iter().position(|candidate| {
                candidate.source_id == source_id && candidate.insert_text == insert_text
            })
        });
        self.reserved_entry_count = self.reserved_entry_count.max(self.candidates.len());
        self.reserved_inner_width =
            self.reserved_inner_width
                .max(completion_popup_inner_width_for_candidates(
                    &self.candidates,
                ));

        preview_before != self.current_text()
    }

    /// Preserve popup dimensions from one earlier session while async sources catch up.
    pub(crate) fn preserve_popup_metrics_from(&mut self, previous: &Self) {
        self.reserved_entry_count = self.reserved_entry_count.max(previous.reserved_entry_count);
        self.reserved_inner_width = self.reserved_inner_width.max(previous.reserved_inner_width);
    }

    /// Build the render-facing popup model for this session.
    pub(crate) fn popup(&self) -> CompletionPopup {
        let entries = self
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| CompletionPopupEntry {
                label: candidate.popup_label.clone(),
                detail: candidate.popup_detail,
                selected: self.selected_index == Some(index),
            })
            .collect();
        CompletionPopup {
            anchor_char_idx: self.popup_anchor_char_idx,
            entries,
            reserved_entry_count: self.reserved_entry_count,
            reserved_inner_width: self.reserved_inner_width,
        }
    }
}

/// One in-flight asynchronous completion request owned by the completion layer.
#[derive(Debug)]
pub(crate) struct PendingAsyncCompletion {
    request: CompletionRequest,
    popup_anchor_char_idx: usize,
    task: AsyncCompletionTask,
}

/// One asynchronous completion worker hidden behind a source-agnostic wrapper.
#[derive(Debug)]
enum AsyncCompletionTask {
    FilePath(file_path_source::FilePathCompletionScan),
}

impl PendingAsyncCompletion {
    /// Spawn one asynchronous completion worker for `request` when a source applies.
    pub(crate) fn spawn(
        registry: &CompletionSourceRegistry,
        request: CompletionRequest,
        popup_anchor_char_idx: usize,
    ) -> Option<Self> {
        if registry.file_path_enabled() && request.is_file_path() {
            let scan = file_path_source::FilePathCompletionScan::spawn(request.clone())?;
            return Some(Self {
                request,
                popup_anchor_char_idx,
                task: AsyncCompletionTask::FilePath(scan),
            });
        }
        None
    }

    /// Return the completion request that owns this asynchronous worker.
    pub(crate) fn request(&self) -> &CompletionRequest {
        &self.request
    }

    /// Return the popup anchor preserved for this asynchronous request.
    pub(crate) fn popup_anchor_char_idx(&self) -> usize {
        self.popup_anchor_char_idx
    }

    /// Return whether `identity` still matches this in-flight request.
    pub(crate) fn matches_identity(
        &self,
        buffer_id: usize,
        identity: &CompletionRequestIdentity,
    ) -> bool {
        self.request.matches_identity(buffer_id, identity)
    }

    /// Cancel the worker owned by this asynchronous completion request.
    pub(crate) fn cancel(&mut self) {
        match &mut self.task {
            AsyncCompletionTask::FilePath(scan) => scan.cancel(),
        }
    }

    /// Drain the worker when it has finished and return its final candidates.
    pub(crate) fn poll(&mut self) -> AsyncCompletionPollResult {
        match &mut self.task {
            AsyncCompletionTask::FilePath(scan) => {
                let result = scan.poll();
                AsyncCompletionPollResult {
                    finished: result.finished,
                    candidates: result.candidates,
                }
            }
        }
    }
}

/// Final poll state for one asynchronous completion worker.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AsyncCompletionPollResult {
    /// Whether the worker finished and no further polling is required.
    pub(crate) finished: bool,
    /// Completed candidates returned by the worker, when available.
    pub(crate) candidates: Option<Vec<CompletionCandidate>>,
}

/// Describe one keyboard direction for completion navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionDirection {
    Up,
    Down,
}

/// Build one request identity from the cursor position when a prefix is active.
pub(crate) fn build_request_identity(
    buffer: &TextBuffer,
    active_file_path: Option<&Path>,
    cursor_char_idx: usize,
) -> Option<CompletionRequestIdentity> {
    if let Some(identity) =
        build_file_path_request_identity(buffer, active_file_path, cursor_char_idx)
    {
        return Some(identity);
    }
    build_word_request_identity(buffer, cursor_char_idx)
}

/// Build one empty-prefix request identity used for LSP trigger completions.
pub(crate) fn build_lsp_trigger_request_identity(
    cursor_char_idx: usize,
) -> CompletionRequestIdentity {
    CompletionRequestIdentity {
        replace_start_char_idx: cursor_char_idx,
        cursor_char_idx,
        original_text: String::new(),
        context: CompletionRequestContext::Word {
            prefix_text: String::new(),
            normalized_prefix: String::new(),
        },
    }
}

/// Recompute the active session from synchronous sources plus `async_candidates`.
pub(crate) fn refresh_session(
    registry: &CompletionSourceRegistry,
    buffer: &TextBuffer,
    request: CompletionRequest,
    popup_anchor_char_idx: usize,
    async_candidates: &[CompletionCandidate],
) -> Option<CompletionSession> {
    let mut candidates = collect_sync_candidates(registry, buffer, &request);
    candidates.extend_from_slice(async_candidates);
    finalize_candidates(registry, &request, &mut candidates);
    if candidates.is_empty() {
        return None;
    }

    Some(CompletionSession::new(
        request,
        candidates,
        popup_anchor_char_idx,
    ))
}

/// Normalize one text value for case-insensitive comparisons.
pub(crate) fn normalize_text(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Collect all synchronous candidates enabled for `request`.
fn collect_sync_candidates(
    registry: &CompletionSourceRegistry,
    buffer: &TextBuffer,
    request: &CompletionRequest,
) -> Vec<CompletionCandidate> {
    let mut candidates = Vec::new();

    // Source dispatch stays centralized here so `EditorState` only manages
    // request lifecycles and popup state instead of source-specific logic.
    if registry.buffer_text_enabled() {
        candidates.extend(buffer_source::collect_buffer_candidates(request, buffer));
    }

    candidates
}

/// Sort, deduplicate, and cap the merged candidate list.
fn finalize_candidates(
    registry: &CompletionSourceRegistry,
    request: &CompletionRequest,
    candidates: &mut Vec<CompletionCandidate>,
) {
    // Source priority keeps path suggestions ahead of buffer words in explicit
    // path contexts while still leaving source-local ranking intact.
    candidates.sort_by_key(|candidate| {
        (
            registry.source_priority(candidate.source_id),
            candidate.rank,
            candidate.source_id.as_str(),
            normalize_text(&candidate.insert_text),
        )
    });

    // Cross-source deduplication keeps the popup readable when different
    // sources produce the same visible insertion text.
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| seen.insert(normalize_text(&candidate.insert_text)));
    candidates.truncate(request.max_candidate_count());
}

/// Return the popup inner width needed for the supplied completion candidates.
fn completion_popup_inner_width_for_candidates(candidates: &[CompletionCandidate]) -> usize {
    let detail_column = candidates
        .iter()
        .map(|candidate| candidate.popup_label.chars().count())
        .max()
        .unwrap_or(0);
    candidates
        .iter()
        .map(|candidate| {
            if let Some(detail) = candidate.popup_detail {
                detail_column + detail.chars().count() + 4
            } else {
                candidate.popup_label.chars().count() + 2
            }
        })
        .max()
        .unwrap_or(1)
}

/// Build one word-completion identity from the contiguous word left of the cursor.
fn build_word_request_identity(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
) -> Option<CompletionRequestIdentity> {
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

    Some(CompletionRequestIdentity {
        replace_start_char_idx: start,
        cursor_char_idx,
        original_text: prefix_text.clone(),
        context: CompletionRequestContext::Word {
            normalized_prefix: normalize_text(&prefix_text),
            prefix_text,
        },
    })
}

/// Build one file-path completion identity when the cursor sits on an explicit path token.
fn build_file_path_request_identity(
    buffer: &TextBuffer,
    active_file_path: Option<&Path>,
    cursor_char_idx: usize,
) -> Option<CompletionRequestIdentity> {
    if cursor_char_idx == 0 {
        return None;
    }

    let previous_idx = cursor_char_idx.saturating_sub(1);
    if !buffer.char_at(previous_idx).is_some_and(is_path_token_char) {
        return None;
    }

    let mut start = previous_idx;
    // Expand to the full non-whitespace token so completion can replace `./foo`
    // as one unit instead of only the trailing word segment after `/`.
    while start > 0 && buffer.char_at(start - 1).is_some_and(is_path_token_char) {
        start -= 1;
    }
    let token_text = buffer.slice_string(start, cursor_char_idx);
    if !has_explicit_path_prefix(&token_text) {
        return None;
    }

    let (display_prefix, segment_prefix) = split_path_token(&token_text);
    let resolved_directory = resolve_completion_directory(active_file_path, &display_prefix)?;
    Some(CompletionRequestIdentity {
        replace_start_char_idx: start,
        cursor_char_idx,
        original_text: token_text,
        context: CompletionRequestContext::FilePath(FilePathCompletionRequest {
            normalized_segment_prefix: normalize_text(&segment_prefix),
            display_prefix,
            segment_prefix,
            resolved_directory,
        }),
    })
}

/// Return whether `c` belongs to one path token eligible for completion parsing.
///
/// Whitespace and surrounding punctuation terminate path-like tokens in source
/// text, shell commands, and quoted strings. Treating those characters as part
/// of a path token would make completion swallow delimiters and replace text
/// beyond the intended path fragment.
fn is_path_token_char(c: char) -> bool {
    !c.is_whitespace()
        && !matches!(
            c,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';'
        )
}

/// Return whether `token_text` starts with one explicit path prefix.
fn has_explicit_path_prefix(token_text: &str) -> bool {
    token_text.starts_with('/')
        || token_text.starts_with("./")
        || token_text.starts_with("../")
        || token_text.starts_with("~/")
}

/// Split one path token into its preserved directory prefix and basename fragment.
fn split_path_token(token_text: &str) -> (String, String) {
    let split_idx = token_text.rfind('/').map_or(0, |index| index + 1);
    (
        token_text[..split_idx].to_string(),
        token_text[split_idx..].to_string(),
    )
}

/// Resolve the directory scanned for `display_prefix`.
fn resolve_completion_directory(
    active_file_path: Option<&Path>,
    display_prefix: &str,
) -> Option<PathBuf> {
    // Absolute paths stay absolute, `~/` expands from the user's home
    // directory, and `./` / `../` resolve from the active buffer directory or
    // process cwd when the buffer is unnamed.
    if display_prefix.starts_with('/') {
        return Some(PathBuf::from(display_prefix));
    }
    if let Some(home_relative) = display_prefix.strip_prefix("~/") {
        let home = std::env::home_dir()?;
        return Some(home.join(home_relative));
    }

    let base_directory = resolve_buffer_base_directory(active_file_path)?;
    Some(base_directory.join(display_prefix))
}

/// Resolve the base directory used for relative file-path completion.
fn resolve_buffer_base_directory(active_file_path: Option<&Path>) -> Option<PathBuf> {
    // Relative buffer paths are interpreted from the current process directory
    // so unsaved-but-named buffers and startup relative paths stay coherent.
    let current_dir = std::env::current_dir().ok()?;
    let Some(active_file_path) = active_file_path else {
        return Some(current_dir);
    };

    let absolute_path = if active_file_path.is_absolute() {
        active_file_path.to_path_buf()
    } else {
        current_dir.join(active_file_path)
    };
    absolute_path
        .parent()
        .map(Path::to_path_buf)
        .or(Some(current_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use test_utils::TempTree;

    const TEST_BUFFER_ID: usize = 7;
    const TEST_REQUEST_GENERATION: usize = 3;

    /// Build one word request identity for unit tests.
    fn word_identity_for(text: &str, cursor_char_idx: usize) -> CompletionRequestIdentity {
        let buffer = TextBuffer::from_str(text);
        build_request_identity(&buffer, None, cursor_char_idx).expect("request should exist")
    }

    /// Build one request for unit tests with a fixed buffer id and generation.
    fn request_for(text: &str, cursor_char_idx: usize) -> CompletionRequest {
        CompletionRequest::new(
            TEST_BUFFER_ID,
            TEST_REQUEST_GENERATION,
            word_identity_for(text, cursor_char_idx),
        )
    }

    #[test]
    /// Confirm request building uses the contiguous word left of the cursor.
    fn test_build_request_uses_current_word_prefix() {
        let request = request_for("alpha beta", 2);

        assert_eq!(request.replace_start_char_idx(), 0);
        assert_eq!(request.cursor_char_idx(), 2);
        assert_eq!(request.original_text(), "al");
        assert_eq!(request.normalized_match_prefix(), "al");
    }

    #[test]
    /// Confirm non-word positions do not create completion requests.
    fn test_build_request_requires_word_character_before_cursor() {
        let buffer = TextBuffer::from_str("alpha beta");

        assert!(build_request_identity(&buffer, None, 0).is_none());
        assert!(build_request_identity(&buffer, None, 6).is_none());
    }

    #[test]
    /// Confirm explicit path prefixes take precedence over word-only parsing.
    fn test_build_request_prefers_path_context_for_explicit_prefix() {
        let buffer = TextBuffer::from_str("./src/li");
        let request =
            build_request_identity(&buffer, Some(Path::new("/tmp/project/src/main.rs")), 8)
                .expect("request should exist");

        match request.context {
            CompletionRequestContext::FilePath(path) => {
                assert_eq!(request.replace_start_char_idx, 0);
                assert_eq!(request.original_text, "./src/li");
                assert_eq!(path.display_prefix(), "./src/");
                assert_eq!(path.segment_prefix(), "li");
            }
            CompletionRequestContext::Word { .. } => {
                panic!("request should use file path context")
            }
        }
    }

    #[test]
    /// Confirm `./` path completion resolves relative to the active buffer directory.
    fn test_build_request_resolves_dot_paths_from_active_buffer_directory() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("workspace/src/main.rs", "fn main() {}\n")
            .expect("write file");
        let buffer = TextBuffer::from_str("./mod");
        let request =
            build_request_identity(&buffer, Some(&tree.path().join("workspace/src/main.rs")), 5)
                .expect("request should exist");

        let CompletionRequestContext::FilePath(path) = request.context else {
            panic!("request should use file path context");
        };
        assert_eq!(path.resolved_directory(), tree.path().join("workspace/src"));
    }

    #[test]
    /// Confirm unnamed buffers fall back to the process current directory for `./`.
    fn test_build_request_resolves_dot_paths_from_current_directory_for_unnamed_buffer() {
        let buffer = TextBuffer::from_str("./mod");
        let request = build_request_identity(&buffer, None, 5).expect("request should exist");

        let CompletionRequestContext::FilePath(path) = request.context else {
            panic!("request should use file path context");
        };
        assert_eq!(
            path.resolved_directory(),
            std::env::current_dir()
                .expect("current directory")
                .join("./")
        );
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
                    popup_label: "alpha".to_string(),
                    popup_detail: None,
                    replace_start_char_idx: 0,
                    replace_end_char_idx: 2,
                    rank: 0,
                },
                CompletionCandidate {
                    source_id: CompletionSourceId::BufferText,
                    insert_text: "alphabet".to_string(),
                    popup_label: "alphabet".to_string(),
                    popup_detail: None,
                    replace_start_char_idx: 0,
                    replace_end_char_idx: 2,
                    rank: 1,
                },
            ],
            2,
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
            vec![CompletionSourceId::FilePath, CompletionSourceId::Lsp]
        );
        assert!(!registry.buffer_text_enabled());
    }

    #[test]
    /// Confirm identity matching ignores only the generation token.
    fn test_matches_identity_uses_request_identity_fields() {
        let request = request_for("alpha", 2);

        assert!(request.matches_identity(TEST_BUFFER_ID, &word_identity_for("alpha", 2)));
        assert!(!request.matches_identity(9, &word_identity_for("alpha", 2)));
    }

    #[test]
    /// Confirm the visible session candidate list respects the popup-size cap.
    fn test_refresh_session_limits_visible_candidates() {
        let text = (0..(MAX_CANDIDATES + 10))
            .map(|index| format!("alpha_{index} "))
            .collect::<String>();
        let buffer = TextBuffer::from_str(&text);
        let identity = build_request_identity(&buffer, None, 2).expect("request should exist");
        let request = CompletionRequest::new(TEST_BUFFER_ID, TEST_REQUEST_GENERATION, identity);

        let session = refresh_session(&CompletionSourceRegistry::new(), &buffer, request, 2, &[])
            .expect("session should exist");

        assert_eq!(session.candidates.len(), MAX_CANDIDATES);
    }

    #[test]
    /// Confirm explicit file-path requests use the larger path-specific popup cap.
    fn test_file_path_requests_use_larger_candidate_cap() {
        let buffer = TextBuffer::from_str("./alpha");
        let identity = build_request_identity(&buffer, None, 7).expect("request should exist");
        let request = CompletionRequest::new(TEST_BUFFER_ID, TEST_REQUEST_GENERATION, identity);

        assert_eq!(request.max_candidate_count(), MAX_FILE_PATH_CANDIDATES);
    }

    #[test]
    /// Confirm candidate replacement preserves the selected item when async results merge in.
    fn test_replace_candidates_preserves_selected_item() {
        let request = request_for("alpha", 2);
        let mut session = CompletionSession::new(
            request,
            vec![CompletionCandidate {
                source_id: CompletionSourceId::BufferText,
                insert_text: "alphabet".to_string(),
                popup_label: "alphabet".to_string(),
                popup_detail: None,
                replace_start_char_idx: 0,
                replace_end_char_idx: 2,
                rank: 0,
            }],
            2,
        );
        session.move_selection(CompletionDirection::Down);

        let preview_changed = session.replace_candidates(vec![
            CompletionCandidate {
                source_id: CompletionSourceId::FilePath,
                insert_text: "alpha-dir".to_string(),
                popup_label: "alpha-dir/".to_string(),
                popup_detail: Some("directory"),
                replace_start_char_idx: 0,
                replace_end_char_idx: 2,
                rank: 0,
            },
            CompletionCandidate {
                source_id: CompletionSourceId::BufferText,
                insert_text: "alphabet".to_string(),
                popup_label: "alphabet".to_string(),
                popup_detail: None,
                replace_start_char_idx: 0,
                replace_end_char_idx: 2,
                rank: 1,
            },
        ]);

        assert!(!preview_changed);
        assert_eq!(session.selected_index, Some(1));
        assert_eq!(session.current_text(), "alphabet");
    }
}
