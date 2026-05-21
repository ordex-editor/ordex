//! Asynchronous content-search picker state and background worker helpers.

use super::picker::{PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState};
use crate::search::SearchQuery;
use crate::spinner::Spinner;
use crate::text_buffer::TextBuffer;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

const SEARCH_PICKER_BATCH_SIZE: usize = 64;
const SEARCH_PICKER_EVENTS_PER_POLL: usize = 4;
const SEARCH_PICKER_QUERY_DEBOUNCE_MS: u128 = 100;
const SEARCH_PICKER_DEBOUNCE_ITEM_THRESHOLD: usize = 10_000;
const SEARCH_PICKER_SPINNER_INTERVAL_MS: u128 = 100;
const SEARCH_PICKER_MAX_RESULTS: usize = 50_000;
const GREP_FILE_LIST_CHUNK_SIZE: usize = 256;

/// One navigable search-result location returned by the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchPickerTarget {
    /// Canonical filesystem path for the destination file.
    pub(crate) file_path: PathBuf,
    /// Zero-based line index of the selected match.
    pub(crate) line: usize,
    /// Zero-based character column of the selected match.
    pub(crate) column: usize,
}

/// One rendered search-result row tracked by the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchPickerItem {
    /// Canonical filesystem path for the destination file.
    pub(crate) file_path: PathBuf,
    /// Relative or absolute path shown to the user.
    pub(crate) display_path: String,
    /// Zero-based line index of the selected match.
    pub(crate) line: usize,
    /// Zero-based character column of the selected match.
    pub(crate) column: usize,
    /// One-line preview for the matched line.
    pub(crate) preview: String,
    /// Stable discovery order used as a tie-breaker.
    pub(crate) order: usize,
    /// Aggregated fuzzy-match text covering the path and preview content.
    match_label: String,
}

/// Mutable state for the asynchronous search-results picker.
#[derive(Debug)]
pub(crate) struct SearchPickerState {
    picker: PickerState<SearchPickerItem>,
    search: Option<SearchPickerSearch>,
    next_order: usize,
    applied_query: String,
    pending_query: Option<String>,
    query_updated_at: Option<Instant>,
    spinner: Spinner,
}

/// One background search worker plus its cancellation handle.
#[derive(Debug)]
struct SearchPickerSearch {
    receiver: Receiver<SearchPickerEvent>,
    cancel: Arc<AtomicBool>,
    started_at: Instant,
}

/// One queued search-picker update from the background worker.
#[derive(Debug)]
enum SearchPickerEvent {
    Batch(Vec<SearchPickerMatch>),
    Finished(Option<String>),
}

/// One raw search match emitted by the worker before picker ordering is assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchPickerMatch {
    file_path: PathBuf,
    display_path: String,
    line: usize,
    column: usize,
    preview: String,
}

/// Summary of one completed worker scan used for status messages.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SearchSummary {
    limit_reached: bool,
}

/// Result of draining search-worker updates into picker state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SearchPickerPollResult {
    /// Whether any visible picker state changed.
    pub(crate) changed: bool,
    /// Optional status message surfaced after the worker finishes.
    pub(crate) status_message: Option<String>,
}

/// Search backend used for one worker run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchBackend {
    Ripgrep,
    GrepGitFiles,
    GrepRecursive,
}

/// Mutable batching state shared while one child process streams matches.
struct SearchStreamState<'a> {
    sender: &'a mpsc::Sender<SearchPickerEvent>,
    cancel: &'a AtomicBool,
    batch: &'a mut Vec<SearchPickerMatch>,
    result_count: &'a mut usize,
    backend: SearchBackend,
}

impl SearchPickerState {
    const POPUP_SPEC: PickerPopupSpec = PickerPopupSpec {
        title: "Search Results",
        query_label: " Filter: ",
        empty_message: "No matching search results",
    };

    /// Start a new asynchronous search rooted at `root` for `pattern`.
    pub(crate) fn new(root: PathBuf, pattern: String, query: SearchQuery) -> Self {
        Self {
            picker: PickerState::new(Vec::new()),
            search: Some(SearchPickerSearch::spawn(root, pattern, query)),
            next_order: 0,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<SearchPickerItem> {
        &mut self.picker
    }

    /// Stop the background worker and release the search handles.
    pub(crate) fn cancel(&mut self) {
        if let Some(search) = &self.search {
            search.cancel.store(true, Ordering::Relaxed);
        }
        self.search = None;
        self.pending_query = None;
        self.query_updated_at = None;
    }

    /// Return whether the picker still has search or deferred filtering work in flight.
    pub(crate) fn is_searching(&self) -> bool {
        self.search.is_some() || self.pending_query.is_some()
    }

    /// Drain any queued worker updates into the picker state.
    pub(crate) fn poll(&mut self, query: &str) -> SearchPickerPollResult {
        if self.search.is_none() && self.pending_query.is_none() {
            return SearchPickerPollResult::default();
        }
        let mut result = SearchPickerPollResult::default();
        let mut finished = false;
        let mut processed_events = 0usize;

        if self.search.is_some() {
            loop {
                // Yield after bounded work so a busy search cannot starve input handling.
                if processed_events >= SEARCH_PICKER_EVENTS_PER_POLL {
                    break;
                }
                let event = match self.search.as_ref() {
                    Some(search) => search.receiver.try_recv(),
                    None => break,
                };
                match event {
                    Ok(SearchPickerEvent::Batch(matches)) => {
                        processed_events += 1;
                        if !matches.is_empty() {
                            let items = matches
                                .into_iter()
                                .map(|search_match| self.build_item(search_match))
                                .collect::<Vec<_>>();
                            self.picker.extend_items(items, &self.applied_query);
                            result.changed = true;
                        }
                    }
                    Ok(SearchPickerEvent::Finished(message)) => {
                        finished = true;
                        result.changed = true;
                        result.status_message = message;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        result.changed = true;
                        result.status_message = Some("Search stopped unexpectedly".to_string());
                        break;
                    }
                }
            }
        }

        if finished {
            self.search = None;
        }
        self.maybe_apply_pending_query(query, &mut result);
        if let Some(started_at) = self.busy_started_at()
            && self
                .spinner
                .sync_to_elapsed(started_at, SEARCH_PICKER_SPINNER_INTERVAL_MS)
        {
            result.changed = true;
        }

        result
    }

    /// Refresh matches for the latest picker-side fuzzy-filter query.
    pub(crate) fn sync_query(&mut self, query: &str) {
        // Small result sets stay fully synchronous so filtering remains immediate.
        if self.picker.item_count() < SEARCH_PICKER_DEBOUNCE_ITEM_THRESHOLD {
            self.pending_query = None;
            self.query_updated_at = None;
            self.picker.sync_query(query);
            self.applied_query = query.to_string();
            return;
        }
        // Repeating the same pending query only extends the debounce window while typing continues.
        if self.pending_query.as_deref() == Some(query) {
            self.query_updated_at = Some(Instant::now());
            return;
        }
        // Once the visible picker already reflects this query, there is no extra work.
        if self.pending_query.is_none() && self.applied_query == query {
            return;
        }
        self.pending_query = Some(query.to_string());
        self.query_updated_at = Some(Instant::now());
    }

    /// Return the selected search target, if the current filter still has matches.
    pub(crate) fn selected_target(&self) -> Option<SearchPickerTarget> {
        // Confirmation waits for the deferred filter to finish so Enter always
        // opens the row that matches the text currently visible in the query prompt.
        if self.pending_query.is_some() {
            return None;
        }
        self.picker.selected().map(|item| SearchPickerTarget {
            file_path: item.file_path.clone(),
            line: item.line,
            column: item.column,
        })
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(
        &self,
        query: &str,
        cursor_column: usize,
        visible_entry_capacity: usize,
    ) -> PickerPopup {
        // The shared picker already limits visible rows, so this picker only layers on search status.
        let mut popup = self.picker.popup(
            Self::POPUP_SPEC,
            query,
            cursor_column,
            visible_entry_capacity,
        );
        popup.query_suffix = self.query_suffix();
        if self.search.is_some() && self.picker.item_count() == 0 && popup.entries.is_empty() {
            popup.empty_message = "Searching...".to_string();
        } else if self.pending_query.is_some() && popup.entries.is_empty() {
            popup.empty_message = "Filtering results...".to_string();
        } else if self.search.is_none() && self.picker.item_count() == 0 {
            popup.empty_message = "No search results".to_string();
        }
        popup
    }

    /// Convert one worker match into a picker item with stable ordering.
    fn build_item(&mut self, search_match: SearchPickerMatch) -> SearchPickerItem {
        let label = format!(
            "{}:{}:{}: {}",
            search_match.display_path,
            search_match.line.saturating_add(1),
            search_match.column.saturating_add(1),
            search_match.preview
        );
        let item = SearchPickerItem {
            file_path: search_match.file_path,
            display_path: search_match.display_path,
            line: search_match.line,
            column: search_match.column,
            preview: search_match.preview,
            order: self.next_order,
            match_label: label,
        };
        self.next_order += 1;
        item
    }

    /// Return the query-row suffix showing the worker spinner and result count.
    fn query_suffix(&self) -> String {
        match (self.is_searching(), self.picker.item_count()) {
            (true, count) => format!("{} {} ", self.spinner.current_frame(), count),
            (false, 0) => String::new(),
            (false, count) => format!("{count} "),
        }
    }

    /// Return when the current search or deferred filter work started.
    fn busy_started_at(&self) -> Option<Instant> {
        self.query_updated_at
            .or_else(|| self.search.as_ref().map(|search| search.started_at))
    }

    /// Apply one pending query once the user has paused long enough to resume filtering.
    fn maybe_apply_pending_query(&mut self, query: &str, result: &mut SearchPickerPollResult) {
        if !self.should_apply_pending_query(query) {
            return;
        }
        let pending_query = self
            .pending_query
            .take()
            .expect("pending query should exist when applying");
        self.picker.sync_query(&pending_query);
        self.applied_query = pending_query;
        self.query_updated_at = None;
        result.changed = true;
    }

    /// Return whether the deferred query update should be applied immediately.
    fn should_apply_pending_query(&self, query: &str) -> bool {
        let Some(pending_query) = self.pending_query.as_deref() else {
            return false;
        };
        if pending_query != query {
            return false;
        }
        self.query_updated_at.is_some_and(|updated_at| {
            updated_at.elapsed().as_millis() >= SEARCH_PICKER_QUERY_DEBOUNCE_MS
        })
    }
}

impl SearchPickerSearch {
    /// Spawn the background worker that searches `root` for `pattern`.
    fn spawn(root: PathBuf, pattern: String, query: SearchQuery) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let started_at = Instant::now();
        thread::spawn(move || {
            let status_message =
                match search_matches(&root, &pattern, &query, &sender, &worker_cancel) {
                    Ok(Some(message)) => Some(message),
                    Ok(None) => None,
                    Err(error) => Some(format!("Search failed: {error}")),
                };
            let _ = sender.send(SearchPickerEvent::Finished(status_message));
        });
        Self {
            receiver,
            cancel,
            started_at,
        }
    }
}

impl PickerItem for SearchPickerItem {
    type Key = usize;

    fn key(&self) -> Self::Key {
        self.order
    }

    fn label(&self) -> &str {
        &self.match_label
    }

    fn order(&self) -> usize {
        self.order
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.match_label.clone(),
            selected,
            primary_marker: false,
            secondary_marker: false,
        }
    }
}

/// Search `root` with the best available backend and stream matches in batches.
fn search_matches(
    root: &Path,
    pattern: &str,
    query: &SearchQuery,
    sender: &mpsc::Sender<SearchPickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<Option<String>> {
    match search_with_ripgrep(root, pattern, query, sender, cancel) {
        Ok(summary) => return Ok(summary.status_message()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    match list_git_search_files(root, cancel) {
        Ok(Some(files)) => {
            return search_with_grep_file_list(root, pattern, query, files, sender, cancel)
                .map(|summary| summary.status_message());
        }
        Ok(None) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    search_with_recursive_grep(root, pattern, query, sender, cancel)
        .map(|summary| summary.status_message())
}

/// Search with ripgrep using its default ignore and hidden-file behavior.
fn search_with_ripgrep(
    root: &Path,
    pattern: &str,
    query: &SearchQuery,
    sender: &mpsc::Sender<SearchPickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<SearchSummary> {
    let child = Command::new("rg")
        .current_dir(root)
        .args([
            "--line-number",
            "--no-heading",
            "--color=never",
            "--null",
            "--",
            pattern,
            ".",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    stream_grep_style_matches(root, child, query, sender, cancel, SearchBackend::Ripgrep)
}

/// Search with grep over git-provided non-ignored file paths.
fn search_with_grep_file_list(
    root: &Path,
    pattern: &str,
    query: &SearchQuery,
    files: Vec<String>,
    sender: &mpsc::Sender<SearchPickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<SearchSummary> {
    let mut batch = Vec::with_capacity(SEARCH_PICKER_BATCH_SIZE);
    let mut result_count = 0usize;
    let mut stream_state = SearchStreamState {
        sender,
        cancel,
        batch: &mut batch,
        result_count: &mut result_count,
        backend: SearchBackend::GrepGitFiles,
    };

    // Chunk the file list so very large repositories do not exceed the OS argument limit.
    for chunk in files.chunks(GREP_FILE_LIST_CHUNK_SIZE) {
        if stream_state.cancel.load(Ordering::Relaxed)
            || *stream_state.result_count >= SEARCH_PICKER_MAX_RESULTS
        {
            break;
        }
        let mut child = Command::new("grep");
        child
            .current_dir(root)
            .args(["-n", "-H", "-I", "-E", "-Z", "--", pattern])
            .args(chunk)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = child.spawn()?;
        let chunk_count = stream_child_matches(root, child, query, &mut stream_state)?;
        if chunk_count == 0 && *stream_state.result_count == 0 {
            continue;
        }
    }

    if !stream_state.batch.is_empty() {
        stream_state
            .sender
            .send(SearchPickerEvent::Batch(std::mem::take(stream_state.batch)))
            .ok();
    }
    Ok(SearchSummary {
        limit_reached: *stream_state.result_count >= SEARCH_PICKER_MAX_RESULTS,
    })
}

/// Search recursively with grep while skipping hidden files and directories.
fn search_with_recursive_grep(
    root: &Path,
    pattern: &str,
    query: &SearchQuery,
    sender: &mpsc::Sender<SearchPickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<SearchSummary> {
    let child = Command::new("grep")
        .current_dir(root)
        .args([
            "-R",
            "-n",
            "-H",
            "-I",
            "-E",
            "-Z",
            "--exclude-dir=.*",
            "--exclude=.*",
            "--",
            pattern,
            ".",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    stream_grep_style_matches(
        root,
        child,
        query,
        sender,
        cancel,
        SearchBackend::GrepRecursive,
    )
}

/// Stream grep-style null-delimited output into batched picker events.
fn stream_grep_style_matches(
    root: &Path,
    child: std::process::Child,
    query: &SearchQuery,
    sender: &mpsc::Sender<SearchPickerEvent>,
    cancel: &AtomicBool,
    backend: SearchBackend,
) -> io::Result<SearchSummary> {
    let mut batch = Vec::with_capacity(SEARCH_PICKER_BATCH_SIZE);
    let mut result_count = 0usize;
    let mut stream_state = SearchStreamState {
        sender,
        cancel,
        batch: &mut batch,
        result_count: &mut result_count,
        backend,
    };
    stream_child_matches(root, child, query, &mut stream_state)?;
    if !stream_state.batch.is_empty() {
        stream_state
            .sender
            .send(SearchPickerEvent::Batch(std::mem::take(stream_state.batch)))
            .ok();
    }
    Ok(SearchSummary {
        limit_reached: *stream_state.result_count >= SEARCH_PICKER_MAX_RESULTS,
    })
}

/// Read one child process, convert its matches, and append them into the shared batch state.
fn stream_child_matches(
    root: &Path,
    mut child: std::process::Child,
    query: &SearchQuery,
    stream_state: &mut SearchStreamState<'_>,
) -> io::Result<usize> {
    let Some(stdout) = child.stdout.take() else {
        let _ = child.wait();
        return Ok(0);
    };
    let mut reader = BufReader::new(stdout);
    let mut path_bytes = Vec::new();
    let mut payload_bytes = Vec::new();
    let mut emitted = 0usize;

    loop {
        // Cancellation is checked between records so the worker can stop after the current read.
        if stream_state.cancel.load(Ordering::Relaxed)
            || *stream_state.result_count >= SEARCH_PICKER_MAX_RESULTS
        {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(emitted);
        }

        path_bytes.clear();
        if reader.read_until(0, &mut path_bytes)? == 0 {
            break;
        }
        if path_bytes.last() == Some(&0) {
            path_bytes.pop();
        }

        payload_bytes.clear();
        if reader.read_until(b'\n', &mut payload_bytes)? == 0 {
            break;
        }

        let matches = parse_grep_style_record(root, &path_bytes, &payload_bytes, query);
        if matches.is_empty() {
            continue;
        }
        for search_match in matches {
            stream_state.batch.push(search_match);
            *stream_state.result_count += 1;
            emitted += 1;
            if stream_state.batch.len() >= SEARCH_PICKER_BATCH_SIZE {
                stream_state
                    .sender
                    .send(SearchPickerEvent::Batch(std::mem::take(stream_state.batch)))
                    .ok();
            }
            if *stream_state.result_count >= SEARCH_PICKER_MAX_RESULTS {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(emitted);
            }
        }
    }

    let status = child.wait()?;
    if command_status_is_success(status, stream_state.backend, emitted) {
        return Ok(emitted);
    }
    Err(io::Error::other(format!(
        "{:?} exited with status {status}",
        stream_state.backend
    )))
}

/// Return whether `status` is acceptable for the selected backend.
fn command_status_is_success(status: ExitStatus, backend: SearchBackend, emitted: usize) -> bool {
    if status.success() {
        return true;
    }
    // Grep-style tools return exit code 1 when no matches were found.
    matches!(
        (backend, status.code(), emitted),
        (SearchBackend::Ripgrep, Some(1), 0)
            | (SearchBackend::GrepGitFiles, Some(1), 0)
            | (SearchBackend::GrepRecursive, Some(1), 0)
    )
}

/// Parse one grep-style null-delimited record into zero or more picker matches.
fn parse_grep_style_record(
    root: &Path,
    path_bytes: &[u8],
    payload_bytes: &[u8],
    query: &SearchQuery,
) -> Vec<SearchPickerMatch> {
    // Normalize both the path and line text before deriving per-match rows from the line content.
    let raw_path = String::from_utf8_lossy(path_bytes);
    let payload = String::from_utf8_lossy(payload_bytes);
    let Some((line_number, preview)) = parse_grep_payload(&payload) else {
        return Vec::new();
    };
    let display_path = normalize_output_path(&raw_path);
    let file_path = resolve_output_path(root, &display_path);
    build_line_matches(&file_path, &display_path, line_number, &preview, query)
}

/// Parse one grep-style payload string into its line number and preview text.
fn parse_grep_payload(payload: &str) -> Option<(usize, String)> {
    let payload = payload.trim_end_matches(['\n', '\r']);
    let (line_number, preview) = payload.split_once(':')?;
    let line_number = line_number.parse::<usize>().ok()?.saturating_sub(1);
    Some((line_number, preview.to_string()))
}

/// Build one row per regex match inside a matched line.
fn build_line_matches(
    file_path: &Path,
    display_path: &str,
    line_number: usize,
    preview: &str,
    query: &SearchQuery,
) -> Vec<SearchPickerMatch> {
    let buffer = TextBuffer::from_reader(preview.as_bytes()).expect("read line preview");
    let matches = query.find_all_in_char_range(&buffer, 0, buffer.chars_count());

    // Each match occurrence gets its own picker row so Enter can jump to the exact location.
    matches
        .into_iter()
        .map(|search_match| SearchPickerMatch {
            file_path: file_path.to_path_buf(),
            display_path: display_path.to_string(),
            line: line_number,
            column: search_match.start,
            preview: preview.to_string(),
        })
        .collect()
}

/// Normalize one tool-reported path for display inside the picker.
fn normalize_output_path(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

/// Resolve one tool-reported path to the filesystem path Ordex should open.
fn resolve_output_path(root: &Path, display_path: &str) -> PathBuf {
    let path = Path::new(display_path);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    root.join(path)
}

/// Return visible git-tracked and untracked files when `root` is a git work tree.
fn list_git_search_files(root: &Path, cancel: &AtomicBool) -> io::Result<Option<Vec<String>>> {
    let mut child = match Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Err(error),
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.wait();
        return Ok(None);
    };
    let mut reader = BufReader::new(stdout);
    let mut entry = Vec::new();
    let mut files = Vec::new();

    loop {
        // Git file discovery only seeds grep fallback, so cancellation may stop before the list finishes.
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Some(Vec::new()));
        }

        entry.clear();
        if reader.read_until(0, &mut entry)? == 0 {
            break;
        }
        if entry.last() == Some(&0) {
            entry.pop();
        }
        let path = String::from_utf8_lossy(&entry).into_owned();
        if path.is_empty() || path_contains_hidden_component(&path) {
            continue;
        }
        files.push(path);
    }

    let status = child.wait()?;
    if status.success() {
        return Ok(Some(files));
    }
    Ok(None)
}

/// Return whether `path` contains any hidden path component.
fn path_contains_hidden_component(path: &str) -> bool {
    path.split('/').any(|component| {
        !component.is_empty() && component != "." && component != ".." && component.starts_with('.')
    })
}

impl SearchSummary {
    /// Convert search caveats into a user-facing status line, if needed.
    fn status_message(self) -> Option<String> {
        self.limit_reached
            .then(|| format!("Search limited to {SEARCH_PICKER_MAX_RESULTS} results"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use test_utils::TempTree;

    /// Build one test picker item with the requested label and order.
    fn item(label: &str, order: usize) -> SearchPickerItem {
        SearchPickerItem {
            file_path: PathBuf::from("src/main.rs"),
            display_path: "src/main.rs".to_string(),
            line: 0,
            column: order,
            preview: "target alpha beta".to_string(),
            order,
            match_label: label.to_string(),
        }
    }

    #[test]
    /// Line parsing should preserve the reported line number and line preview.
    fn test_parse_grep_payload_reads_line_number_and_preview() {
        assert_eq!(
            parse_grep_payload("7:target text\n"),
            Some((6, "target text".to_string()))
        );
    }

    #[test]
    /// Path normalization should drop the recursive-search `./` prefix for display.
    fn test_normalize_output_path_strips_leading_dot_slash() {
        assert_eq!(normalize_output_path("./src/main.rs"), "src/main.rs");
        assert_eq!(normalize_output_path("src/main.rs"), "src/main.rs");
    }

    #[test]
    /// Hidden path detection should reject files and directories whose names begin with `.`.
    fn test_path_contains_hidden_component_detects_hidden_segments() {
        assert!(path_contains_hidden_component(".env"));
        assert!(path_contains_hidden_component("src/.cache/item.txt"));
        assert!(!path_contains_hidden_component("src/cache/item.txt"));
    }

    #[test]
    /// One matched line should produce one picker row per regex match occurrence.
    fn test_build_line_matches_returns_each_match_location() {
        let query = SearchQuery::compile("ana").expect("compile regex");
        let matches =
            build_line_matches(Path::new("sample.txt"), "sample.txt", 3, "banana", &query);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].column, 1);
        assert_eq!(matches[1].column, 3);
        assert_eq!(matches[0].line, 3);
    }

    #[test]
    /// Picker query filtering should preserve the selected row when it still matches.
    fn test_search_picker_preserves_selected_row_across_query_updates() {
        let mut picker = SearchPickerState {
            picker: PickerState::new(vec![
                item("src/alpha.rs:1:1: alpha target", 0),
                item("src/beta.rs:2:5: beta target", 1),
                item("tests/beta.rs:4:3: beta helper", 2),
            ]),
            search: None,
            next_order: 3,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        };

        picker.picker_mut().move_down();
        picker.sync_query("beta");

        assert_eq!(picker.selected_target().expect("selected row").column, 1);
    }

    #[test]
    /// Git-backed grep fallback should ignore hidden tracked paths when building its file list.
    fn test_list_git_search_files_skips_hidden_paths() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write visible file");
        tree.write_file(".hidden/match.rs", "hidden\n")
            .expect("write hidden file");

        let init_status = Command::new("git")
            .current_dir(tree.path())
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(init_status.success());

        let files = list_git_search_files(tree.path(), &AtomicBool::new(false))
            .expect("list git files")
            .expect("git worktree");

        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(!files.iter().any(|path| path.contains(".hidden")));
    }

    #[test]
    /// Search completion should surface the configured result cap in the status message.
    fn test_search_summary_formats_limit_message() {
        assert_eq!(
            SearchSummary {
                limit_reached: true
            }
            .status_message()
            .as_deref(),
            Some("Search limited to 50000 results")
        );
    }

    #[test]
    /// Polling should yield after bounded queued work so input stays responsive.
    fn test_search_picker_poll_yields_with_pending_batches() {
        let (sender, receiver) = mpsc::channel();
        for index in 0..(SEARCH_PICKER_EVENTS_PER_POLL + 3) {
            sender
                .send(SearchPickerEvent::Batch(vec![SearchPickerMatch {
                    file_path: PathBuf::from(format!("src/file_{index}.rs")),
                    display_path: format!("src/file_{index}.rs"),
                    line: index,
                    column: 0,
                    preview: "target".to_string(),
                }]))
                .expect("queue batch");
        }

        let mut picker = SearchPickerState {
            picker: PickerState::new(Vec::new()),
            search: Some(SearchPickerSearch {
                receiver,
                cancel: Arc::new(AtomicBool::new(false)),
                started_at: Instant::now(),
            }),
            next_order: 0,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        };

        let result = picker.poll("");
        let remaining_events = picker
            .search
            .as_ref()
            .expect("search should remain active")
            .receiver
            .try_iter()
            .count();

        assert!(result.changed);
        assert_eq!(picker.picker.item_count(), SEARCH_PICKER_EVENTS_PER_POLL);
        assert_eq!(remaining_events, 3);
    }
}
