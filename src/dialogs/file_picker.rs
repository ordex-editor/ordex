//! Asynchronous file-picker state and background scan helpers.

use super::ignore_rules::{IgnoreEvaluationMode, IgnoreMatcher, IgnoreTraversalState};
use super::picker::{
    MatchScore, PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState,
    fuzzy_match_score, query_excludes_candidate,
};
use crate::spinner::Spinner;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

const FILE_PICKER_BATCH_SIZE: usize = 64;
const FILE_PICKER_MIN_EVENTS_PER_POLL: usize = 64;
const FILE_PICKER_MAX_EVENTS_PER_POLL: usize = 512;
const FILE_PICKER_POLL_BUDGET_MS: u128 = 4;
const FILE_PICKER_QUERY_DEBOUNCE_MS: u128 = 100;
const FILE_PICKER_DEBOUNCE_ITEM_THRESHOLD: usize = 10_000;
const FILE_PICKER_SPINNER_INTERVAL_MS: u128 = 100;
pub(crate) const DEFAULT_FILE_PICKER_MAX_FILES: usize = 1_000_000;

/// One discovered file listed by the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilePickerItem {
    /// Relative path shown in the popup and passed back on confirm.
    pub(crate) path: String,
    /// Basename used for higher-priority fuzzy matches.
    pub(crate) file_name: String,
    /// Stable discovery order used as a tie-breaker for equal matches.
    pub(crate) order: usize,
}

/// Mutable state for the asynchronous file picker.
#[derive(Debug)]
pub(crate) struct FilePickerState {
    picker: PickerState<FilePickerItem>,
    scan: Option<FilePickerScan>,
    next_order: usize,
    /// Last query text already applied to `picker`, used when appending scan batches.
    applied_query: String,
    /// Latest query text waiting for the debounce window before re-filtering a huge picker.
    pending_query: Option<String>,
    /// Time when `pending_query` last changed, used to decide when filtering may resume.
    query_updated_at: Option<Instant>,
    spinner: Spinner,
}

/// One background scan plus its cancellation handle.
#[derive(Debug)]
struct FilePickerScan {
    receiver: Receiver<FilePickerEvent>,
    cancel: Arc<AtomicBool>,
    started_at: Instant,
}

/// One batch of background scan updates.
#[derive(Debug)]
enum FilePickerEvent {
    Batch(Vec<String>),
    Finished(Option<String>),
}

/// One completed scan summary used to surface worker-side caveats.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ScanSummary {
    limit_reached: bool,
    skipped_entries: usize,
}

/// Mutable filesystem-scan bookkeeping shared across recursive calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FilesystemScanProgress {
    max_files: usize,
    discovered_files: usize,
    summary: ScanSummary,
}

/// Result of draining background scan updates into picker state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FilePickerPollResult {
    /// Whether any visible picker state changed.
    pub(crate) changed: bool,
    /// Optional status message surfaced after the worker finishes.
    pub(crate) status_message: Option<String>,
}

impl FilePickerState {
    const POPUP_SPEC: PickerPopupSpec = PickerPopupSpec {
        title: "Files",
        query_label: " Open: ",
        empty_message: "No matching files",
    };

    /// Start a new asynchronous scan rooted at `root`.
    pub(crate) fn new(root: PathBuf, max_files: usize) -> Self {
        Self {
            picker: PickerState::new(Vec::new()),
            scan: Some(FilePickerScan::spawn(root, max_files)),
            next_order: 0,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        }
    }

    /// Borrow the shared picker state mutably.
    pub(crate) fn picker_mut(&mut self) -> &mut PickerState<FilePickerItem> {
        &mut self.picker
    }

    /// Stop the background scan and release the picker worker handles.
    pub(crate) fn cancel(&mut self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        self.scan = None;
        self.pending_query = None;
        self.query_updated_at = None;
    }

    /// Return whether the file picker still has background scan or filter work in flight.
    pub(crate) fn is_scanning(&self) -> bool {
        self.scan.is_some() || self.pending_query.is_some()
    }

    /// Drain any pending background scan updates into the picker state.
    pub(crate) fn poll(&mut self, query: &str) -> FilePickerPollResult {
        if self.scan.is_none() && self.pending_query.is_none() {
            return FilePickerPollResult::default();
        }
        let mut result = FilePickerPollResult::default();
        let mut finished = false;
        let mut processed_events = 0usize;
        let poll_started_at = Instant::now();

        if self.scan.is_some() {
            loop {
                // Drain at least one substantial chunk every poll so large repositories
                // can appear quickly, but cap and budget the work to keep typing responsive.
                if processed_events >= FILE_PICKER_MAX_EVENTS_PER_POLL {
                    break;
                }
                if processed_events >= FILE_PICKER_MIN_EVENTS_PER_POLL
                    && poll_started_at.elapsed().as_millis() >= FILE_PICKER_POLL_BUDGET_MS
                {
                    break;
                }
                let event = match self.scan.as_ref() {
                    Some(scan) => scan.receiver.try_recv(),
                    None => break,
                };
                match event {
                    Ok(FilePickerEvent::Batch(paths)) => {
                        processed_events += 1;
                        if !paths.is_empty() {
                            let mut items = Vec::with_capacity(paths.len());
                            for path in paths {
                                items.push(self.build_item(path));
                            }
                            self.picker.extend_items(items, &self.applied_query);
                            result.changed = true;
                        }
                    }
                    Ok(FilePickerEvent::Finished(message)) => {
                        finished = true;
                        result.changed = true;
                        result.status_message = message;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        result.changed = true;
                        result.status_message = Some("File scan stopped unexpectedly".to_string());
                        break;
                    }
                }
            }
        }

        if finished {
            self.scan = None;
        }
        self.maybe_apply_pending_query(query, &mut result);
        if let Some(started_at) = self.busy_started_at()
            && self
                .spinner
                .sync_to_elapsed(started_at, FILE_PICKER_SPINNER_INTERVAL_MS)
        {
            result.changed = true;
        }

        result
    }

    /// Refresh matches for the latest query text.
    pub(crate) fn sync_query(&mut self, query: &str) {
        // Small pickers stay fully synchronous so short scans keep immediate feedback.
        if self.picker.item_count() < FILE_PICKER_DEBOUNCE_ITEM_THRESHOLD {
            self.pending_query = None;
            self.query_updated_at = None;
            self.picker.sync_query(query);
            self.applied_query = query.to_string();
            return;
        }
        // Repeating the same pending query only extends the debounce window so we
        // avoid re-filtering while the user is still typing.
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

    /// Return the selected path, if the current filter still has matches.
    pub(crate) fn selected_path(&self) -> Option<&str> {
        // Confirmation waits for the deferred filter to finish so Enter always opens
        // the row that matches the text currently visible in the query prompt.
        if self.pending_query.is_some() {
            return None;
        }
        self.picker.selected().map(|item| item.path.as_str())
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(
        &self,
        query: &str,
        cursor_column: usize,
        visible_entry_capacity: usize,
    ) -> PickerPopup {
        // The shared picker already limits entries to the visible window, so the
        // file picker only needs to add scan-specific status text around it.
        let mut popup = self.picker.popup(
            Self::POPUP_SPEC,
            query,
            cursor_column,
            visible_entry_capacity,
        );
        popup.query_suffix = self.query_suffix();
        if self.scan.is_some() && self.picker.item_count() == 0 && popup.entries.is_empty() {
            popup.empty_message = "Scanning files...".to_string();
        } else if self.pending_query.is_some() && popup.entries.is_empty() {
            popup.empty_message = "Filtering files...".to_string();
        }
        popup
    }

    /// Convert one discovered path into a picker item with stable tie-breaker order.
    fn build_item(&mut self, path: String) -> FilePickerItem {
        let item = FilePickerItem {
            file_name: file_name_from_path(&path),
            path,
            order: self.next_order,
        };
        self.next_order += 1;
        item
    }

    /// Return the spinner glyph shown while the asynchronous scan is active.
    fn spinner_glyph(&self) -> char {
        self.spinner.current_frame()
    }

    /// Return the query-row suffix showing scan status and filtered/total counts.
    fn query_suffix(&self) -> String {
        let counts = self.picker.fuzzy_match_counts();
        if self.is_scanning() {
            // While scan or deferred filtering is active, keep the spinner prefix
            // and report counts from the most recently applied fuzzy filter state.
            return format!(
                "{} {}/{} ",
                self.spinner_glyph(),
                counts.filtered,
                counts.total
            );
        }
        format!("{}/{} ", counts.filtered, counts.total)
    }

    /// Return when the current scan or deferred filter work started.
    fn busy_started_at(&self) -> Option<Instant> {
        self.query_updated_at
            .or_else(|| self.scan.as_ref().map(|scan| scan.started_at))
    }

    /// Apply one pending query once the user has paused long enough to avoid typing stalls.
    fn maybe_apply_pending_query(&mut self, query: &str, result: &mut FilePickerPollResult) {
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

    /// Return whether the current deferred query update should be applied now.
    fn should_apply_pending_query(&self, query: &str) -> bool {
        let Some(pending_query) = self.pending_query.as_deref() else {
            return false;
        };
        if pending_query != query {
            return false;
        }
        self.query_updated_at.is_some_and(|updated_at| {
            updated_at.elapsed().as_millis() >= FILE_PICKER_QUERY_DEBOUNCE_MS
        })
    }
}

impl FilePickerScan {
    /// Spawn the background worker that discovers files under `root`.
    fn spawn(root: PathBuf, max_files: usize) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let started_at = Instant::now();
        thread::spawn(move || {
            let status_message = match scan_files(&root, max_files, &sender, &worker_cancel) {
                Ok(Some(message)) => Some(message),
                Ok(None) => None,
                Err(error) => Some(format!("File scan failed: {error}")),
            };
            let _ = sender.send(FilePickerEvent::Finished(status_message));
        });
        Self {
            receiver,
            cancel,
            started_at,
        }
    }
}

impl PickerItem for FilePickerItem {
    fn label(&self) -> &str {
        &self.path
    }

    fn order(&self) -> usize {
        self.order
    }

    fn match_score(&self, query: &str) -> Option<MatchScore> {
        if query_excludes_candidate(&self.file_name, query)
            || query_excludes_candidate(&self.path, query)
        {
            return None;
        }

        match (
            fuzzy_match_score(&self.file_name, query),
            fuzzy_match_score(&self.path, query),
        ) {
            (Some(file_name), Some(path)) => Some(file_name.min(path)),
            (Some(file_name), None) => Some(file_name),
            (None, Some(path)) => Some(path),
            (None, None) => None,
        }
    }

    fn popup_entry(&self, selected: bool) -> PickerPopupEntry {
        PickerPopupEntry {
            label: self.path.clone(),
            search_result_parts: None,
            selected,
            primary_marker: false,
            secondary_marker: false,
        }
    }
}

/// Scan `root` with the best available strategy and stream relative paths in batches.
fn scan_files(
    root: &Path,
    max_files: usize,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<Option<String>> {
    let mut ignore_matcher = IgnoreMatcher::new(root.to_path_buf());
    // Keep ignore matching scoped to the picker root.
    ignore_matcher.set_rules_ceiling(Some(root.to_path_buf()));
    match scan_filesystem(root, max_files, sender, cancel, &mut ignore_matcher) {
        Ok(summary) => Ok(summary.status_message(max_files)),
        Err(error) => Err(error),
    }
}

/// Compatibility scan entry kept for existing tests and benches.
///
/// Returns `Some` with scan summary because traversal now always runs in-process.
#[cfg(test)]
fn scan_git_tracked_and_untracked(
    root: &Path,
    max_files: usize,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
    ignore_matcher: &mut IgnoreMatcher,
) -> io::Result<Option<ScanSummary>> {
    ignore_matcher.set_rules_ceiling(Some(root.to_path_buf()));
    let summary = scan_filesystem(root, max_files, sender, cancel, ignore_matcher)?;
    Ok(Some(summary))
}

/// Recursively scan `root` with the standard library traversal path.
fn scan_filesystem(
    root: &Path,
    max_files: usize,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
    ignore_matcher: &mut IgnoreMatcher,
) -> io::Result<ScanSummary> {
    let mut batch = Vec::with_capacity(FILE_PICKER_BATCH_SIZE);
    let mut progress = FilesystemScanProgress {
        max_files,
        discovered_files: 0,
        summary: ScanSummary::default(),
    };
    let mut traversal_state =
        ignore_matcher.begin_traversal(Path::new(""), false, IgnoreEvaluationMode::AllRules)?;
    walk_directory(
        root,
        sender,
        cancel,
        ignore_matcher,
        &mut traversal_state,
        &mut batch,
        &mut progress,
    )?;
    if !batch.is_empty() {
        sender.send(FilePickerEvent::Batch(batch)).ok();
    }
    Ok(progress.summary)
}

/// Recursively walk one directory and stream visible files into `batch`.
fn walk_directory(
    root: &Path,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
    ignore_matcher: &mut IgnoreMatcher,
    traversal_state: &mut IgnoreTraversalState,
    batch: &mut Vec<String>,
    progress: &mut FilesystemScanProgress,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) || progress.summary.limit_reached {
        return Ok(());
    }

    let directory_path = ignore_matcher
        .traversal_directory_absolute_path(traversal_state)
        .to_path_buf();
    let read_dir = match fs::read_dir(&directory_path) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            progress.summary.skipped_entries += 1;
            if ignore_matcher
                .traversal_directory_relative_path(traversal_state)
                .as_os_str()
                .is_empty()
            {
                // An unreadable root leaves the picker with nowhere else to scan,
                // so the caller needs the original error instead of a silent skip.
                return Err(error);
            }
            return Ok(());
        }
    };
    // Stream directory entries as provided by the filesystem to avoid per-directory
    // allocation and sorting costs in very large trees.
    for entry in read_dir {
        if cancel.load(Ordering::Relaxed) || progress.summary.limit_reached {
            return Ok(());
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                progress.summary.skipped_entries += 1;
                continue;
            }
        };
        let file_name = entry.file_name();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                progress.summary.skipped_entries += 1;
                continue;
            }
        };

        if file_type.is_dir() {
            // Skip Git metadata directories so nested repositories do not flood the picker.
            if file_name == ".git" {
                continue;
            }
            // Directory exclusions are checked before recursion so ignored trees
            // are skipped consistently with file-level filtering.
            let child_ignored =
                ignore_matcher.enter_traversal_directory(traversal_state, file_name.as_os_str())?;
            if child_ignored {
                ignore_matcher.leave_traversal_directory(traversal_state);
                continue;
            }
            // Always restore traversal cursor even when recursive descent errors.
            let recurse_result = walk_directory(
                root,
                sender,
                cancel,
                ignore_matcher,
                traversal_state,
                batch,
                progress,
            );
            ignore_matcher.leave_traversal_directory(traversal_state);
            recurse_result?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }
        if ignore_matcher.traversal_file_ignored(traversal_state, file_name.as_os_str())? {
            continue;
        }

        let relative_path = ignore_matcher
            .traversal_directory_relative_path(traversal_state)
            .join(&file_name);
        batch.push(display_picker_path(root, &relative_path));
        progress.discovered_files += 1;
        if batch.len() >= FILE_PICKER_BATCH_SIZE {
            sender
                .send(FilePickerEvent::Batch(std::mem::take(batch)))
                .ok();
        }
        if progress.discovered_files >= progress.max_files {
            progress.summary.limit_reached = true;
            return Ok(());
        }
    }

    Ok(())
}

/// Return the picker-facing path string for one file discovered under `root`.
fn display_picker_path(root: &Path, relative_path: &Path) -> String {
    if root == Path::new("/") {
        return root.join(relative_path).display().to_string();
    }
    relative_path.display().to_string()
}

/// Return the basename used for higher-priority fuzzy matching.
fn file_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string()
}

impl ScanSummary {
    /// Convert scan caveats into one user-facing status line, if needed.
    fn status_message(self, max_files: usize) -> Option<String> {
        match (self.limit_reached, self.skipped_entries) {
            (false, 0) => None,
            (true, 0) => Some(format!("File picker limited to {max_files} files")),
            (false, skipped) => Some(format!("File scan skipped {skipped} unreadable path(s)")),
            (true, skipped) => Some(format!(
                "File picker limited to {max_files} files; skipped {skipped} unreadable path(s)"
            )),
        }
    }
}

#[cfg(all(test, feature = "tiny-bench"))]
#[path = "file_picker_bench.rs"]
mod file_picker_bench;

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use test_utils::TempTree;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Initialize one Git repository at `path` for scan tests.
    fn init_git_repository(path: &Path) {
        let init_status = Command::new("git")
            .current_dir(path)
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(init_status.success());
    }

    /// One guard that restores directory permissions after one test finishes.
    #[cfg(unix)]
    struct PermissionResetGuard {
        path: PathBuf,
        mode: u32,
    }

    #[cfg(unix)]
    impl PermissionResetGuard {
        /// Build one guard that will reset `path` permissions to `mode` on drop.
        fn new(path: PathBuf, mode: u32) -> Self {
            Self { path, mode }
        }
    }

    #[cfg(unix)]
    impl Drop for PermissionResetGuard {
        /// Restore guarded directory permissions so temp-tree cleanup can succeed.
        fn drop(&mut self) {
            if let Ok(metadata) = fs::metadata(&self.path) {
                // Reset permissions only when the directory still exists.
                let mut permissions = metadata.permissions();
                permissions.set_mode(self.mode);
                let _ = fs::set_permissions(&self.path, permissions);
            }
        }
    }

    #[test]
    /// Verify that one poll call yields even when more scan batches are already queued.
    fn test_file_picker_poll_yields_with_pending_batches() {
        let (sender, receiver) = mpsc::channel();
        for index in 0..(FILE_PICKER_MIN_EVENTS_PER_POLL + 4) {
            // Queue more work than one UI poll is allowed to process.
            sender
                .send(FilePickerEvent::Batch(vec![format!(
                    "dir/file_{index:03}.txt"
                )]))
                .expect("queue batch");
        }

        let mut picker = FilePickerState {
            picker: PickerState::new(Vec::new()),
            scan: Some(FilePickerScan {
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
        let processed_items = picker.picker.item_count();
        let remaining_events = picker
            .scan
            .as_ref()
            .expect("scan should remain active")
            .receiver
            .try_iter()
            .count();

        assert!(result.changed);
        assert!(processed_items >= FILE_PICKER_MIN_EVENTS_PER_POLL);
        assert!(processed_items <= FILE_PICKER_MIN_EVENTS_PER_POLL + 4);
        assert_eq!(
            processed_items + remaining_events,
            FILE_PICKER_MIN_EVENTS_PER_POLL + 4
        );
    }

    #[test]
    /// Verify that Linux release builds keep Git-backed picker scans below one second median.
    #[cfg(target_os = "linux")]
    fn test_scan_git_perf_gate_median_under_one_second() {
        if std::env::var("ORDEX_ENABLE_PERF_GATES").is_err() {
            return;
        }
        if cfg!(debug_assertions) {
            return;
        }
        let tree = TempTree::new().expect("create temp tree");
        for directory_index in 0..200 {
            for file_index in 0..60 {
                // Build one deterministic fixture tree representative of large repositories.
                tree.write_file(
                    &format!("src/dir_{directory_index:03}/file_{file_index:03}.rs"),
                    "fn fixture() {}\n",
                )
                .expect("write perf fixture");
            }
        }
        init_git_repository(tree.path());

        let mut durations = Vec::new();
        for _ in 0..3 {
            let started_at = Instant::now();
            let (sender, receiver) = mpsc::channel();
            let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
            let summary = scan_git_tracked_and_untracked(
                tree.path(),
                DEFAULT_FILE_PICKER_MAX_FILES,
                &sender,
                &AtomicBool::new(false),
                &mut ignore_matcher,
            )
            .expect("scan git worktree")
            .expect("git scan summary");
            assert_eq!(summary, ScanSummary::default());

            let mut emitted_paths = 0usize;
            while let Ok(event) = receiver.try_recv() {
                // Consume every streamed batch so the run reflects complete listing work.
                if let FilePickerEvent::Batch(batch) = event {
                    emitted_paths += batch.len();
                }
            }
            assert_eq!(emitted_paths, 12_000);
            durations.push(started_at.elapsed());
        }
        durations.sort_unstable();
        assert!(durations[1] <= std::time::Duration::from_secs(1));
    }

    #[test]
    fn test_file_picker_prefers_basename_match_over_longer_path_match() {
        let item = FilePickerItem {
            path: "src/syntax/profiles/cpp.rs".to_string(),
            file_name: "cpp.rs".to_string(),
            order: 0,
        };
        let path_match = fuzzy_match_score(&item.path, "cpp").expect("path score");
        let picker_match = item.match_score("cpp").expect("picker score");
        assert!(picker_match <= path_match);
    }

    #[test]
    fn test_file_picker_negation_uses_literal_basename_or_path_substrings() {
        let item = FilePickerItem {
            path: "src/main.rs".to_string(),
            file_name: "main.rs".to_string(),
            order: 0,
        };

        assert!(item.match_score("!").is_some());
        assert!(item.match_score("!main.rs").is_none());
        assert!(item.match_score("!src/").is_none());
        assert!(item.match_score("!Main.rs").is_some());
    }

    #[test]
    fn test_scan_summary_formats_limit_and_skip_message() {
        let summary = ScanSummary {
            limit_reached: true,
            skipped_entries: 2,
        };
        assert_eq!(
            summary.status_message(32).as_deref(),
            Some("File picker limited to 32 files; skipped 2 unreadable path(s)")
        );
    }

    #[test]
    fn test_scan_filesystem_respects_max_file_limit() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("a.txt", "a\n").expect("write file");
        tree.write_file("b.txt", "b\n").expect("write file");
        tree.write_file("dir/c.txt", "c\n").expect("write file");

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_filesystem(
            tree.path(),
            2,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan filesystem");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(paths.len(), 2);
        assert!(summary.limit_reached);
    }

    #[test]
    /// Verify that the fallback filesystem scan skips nested Git metadata directories.
    fn test_scan_filesystem_skips_nested_git_directories() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write visible file");
        tree.write_file("vendor/.git/config", "[core]\n")
            .expect("write nested git metadata");
        tree.write_file("vendor/lib.rs", "pub fn helper() {}\n")
            .expect("write nested visible file");

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_filesystem(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan filesystem");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"vendor/lib.rs".to_string()));
        assert!(!paths.iter().any(|path| path.contains(".git/")));
    }

    #[test]
    /// Verify that fallback scans honor nested `.ignore` files in non-Git directories.
    fn test_scan_filesystem_honors_ignore_rules() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "*.tmp\n")
            .expect("write root ignore file");
        tree.write_file("nested/.ignore", "build/\n")
            .expect("write nested ignore file");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write visible file");
        tree.write_file("src/cache.tmp", "cached\n")
            .expect("write ignored tmp file");
        tree.write_file("nested/build/generated.rs", "pub fn generated() {}\n")
            .expect("write ignored nested file");
        tree.write_file("nested/keep.rs", "pub fn keep() {}\n")
            .expect("write visible nested file");

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let _summary = scan_filesystem(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan filesystem");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"nested/keep.rs".to_string()));
        assert!(!paths.contains(&"src/cache.tmp".to_string()));
        assert!(!paths.contains(&"nested/build/generated.rs".to_string()));
    }

    #[test]
    fn test_scan_git_respects_small_max_file_limit_with_partial_batch() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("a.txt", "a\n").expect("write file");
        tree.write_file("b.txt", "b\n").expect("write file");
        tree.write_file("dir/c.txt", "c\n").expect("write file");

        let init_status = Command::new("git")
            .current_dir(tree.path())
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(init_status.success());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            2,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(paths.len(), 2);
        assert!(summary.limit_reached);
    }

    #[test]
    /// Verify that Git scans keep submodule directories out of picker file rows.
    fn test_scan_git_skips_directory_entries() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write visible file");
        fs::create_dir_all(tree.path().join("vendor")).expect("create submodule directory");

        let init_status = Command::new("git")
            .current_dir(tree.path())
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(init_status.success());

        let gitlink_status = Command::new("git")
            .current_dir(tree.path())
            .args([
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,0123456789012345678901234567890123456789,vendor",
            ])
            .status()
            .expect("write gitlink entry");
        assert!(gitlink_status.success());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(!paths.contains(&"vendor".to_string()));
    }

    #[test]
    /// Verify that `.ignore` can re-include `.gitignore`-ignored files via negation.
    fn test_scan_git_applies_ignore_additively() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "git_ignored.txt\n")
            .expect("write gitignore file");
        tree.write_file(".ignore", "open_picker_ignored.txt\n!git_ignored.txt\n")
            .expect("write ignore file");
        tree.write_file("visible.txt", "visible\n")
            .expect("write visible file");
        tree.write_file("git_ignored.txt", "ignored by git\n")
            .expect("write gitignored file");
        tree.write_file("open_picker_ignored.txt", "ignored by picker\n")
            .expect("write picker ignored file");

        let init_status = Command::new("git")
            .current_dir(tree.path())
            .args(["init", "-q"])
            .status()
            .expect("run git init");
        assert!(init_status.success());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"visible.txt".to_string()));
        assert!(paths.contains(&"git_ignored.txt".to_string()));
        assert!(!paths.contains(&"open_picker_ignored.txt".to_string()));
    }

    #[test]
    /// Verify that `!/old` can un-ignore `old/plan.md` from Git ignored baseline.
    fn test_scan_git_unignores_descendant_through_ancestor_negation() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "old/\n")
            .expect("write gitignore file");
        tree.write_file(".ignore", "!/old\n")
            .expect("write ignore file");
        tree.write_file("old/plan.md", "plan\n")
            .expect("write ignored descendant");

        init_git_repository(tree.path());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"old/plan.md".to_string()));
    }

    #[test]
    /// Verify that Git scans stream early batches before finishing full directory recursion.
    #[cfg(unix)]
    fn test_scan_git_streams_before_full_directory_recursion_completes() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "stream-target\n")
            .expect("write gitignore file");
        tree.write_file(".ignore", "!/stream-target\n")
            .expect("write ignore file");

        for index in 0..FILE_PICKER_BATCH_SIZE {
            tree.write_file(&format!("stream-target/{index:03}.rs"), "fn main() {}\n")
                .expect("write batch-sized source file set");
        }

        let blocked_dir = tree.path().join("stream-target/zzz_blocked");
        fs::create_dir_all(&blocked_dir).expect("create blocked directory");
        let _permission_guard = PermissionResetGuard::new(blocked_dir.clone(), 0o755);

        let mut blocked_permissions = fs::metadata(&blocked_dir)
            .expect("read blocked directory metadata")
            .permissions();
        // Remove all permissions to trigger a traversal error if recursion reaches this path.
        blocked_permissions.set_mode(0o000);
        fs::set_permissions(&blocked_dir, blocked_permissions)
            .expect("set blocked directory permissions");

        init_git_repository(tree.path());

        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let scan_root = tree.path().to_path_buf();
        let worker_cancel = Arc::clone(&cancel);
        let handle = std::thread::spawn(move || {
            let mut ignore_matcher = IgnoreMatcher::new(scan_root.clone());
            scan_git_tracked_and_untracked(
                &scan_root,
                FILE_PICKER_BATCH_SIZE,
                &sender,
                &worker_cancel,
                &mut ignore_matcher,
            )
        });

        let first_event = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("receive one streamed batch before recursion finishes");
        match first_event {
            FilePickerEvent::Batch(batch) => {
                assert_eq!(batch.len(), FILE_PICKER_BATCH_SIZE);
            }
            FilePickerEvent::Finished(_) => {
                panic!("expected one streamed batch before completion event")
            }
        }

        // Stop background work once we observed the first streamed batch.
        cancel.store(true, Ordering::Relaxed);
        let scan_result = handle.join().expect("join scan thread");
        let summary = scan_result
            .expect("scan git worktree")
            .expect("git scan summary");
        assert!(summary.limit_reached);
    }

    #[test]
    /// Verify that `.ignore` can re-include descendants from a `.gitignore` directory exclusion.
    fn test_scan_git_reincludes_descendants_of_unignored_directory() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "test-backend\n")
            .expect("write gitignore file");
        tree.write_file(".ignore", "!/test-backend\n")
            .expect("write ignore file");
        tree.write_file("test-backend/src/main.rs", "fn main() {}\n")
            .expect("write reincluded source file");

        init_git_repository(tree.path());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"test-backend/src/main.rs".to_string()));
    }

    #[test]
    /// Verify that parent ignore files outside the Git worktree do not hide visible files.
    fn test_scan_git_ignores_parent_gitignore_outside_worktree() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "test-backend/\n")
            .expect("write parent gitignore file");
        tree.write_file(
            "workspace/project/test-backend/src/main.rs",
            "fn main() {}\n",
        )
        .expect("write visible source file");

        let project_root = tree.path().join("workspace/project");
        init_git_repository(&project_root);

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(project_root.clone());
        let summary = scan_git_tracked_and_untracked(
            &project_root,
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"test-backend/src/main.rs".to_string()));
    }

    #[test]
    /// Verify that one nested Git repository directory contributes its file contents.
    fn test_scan_git_expands_nested_repository_directory_entries() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("reproducer-memchr/src/main.rs", "fn main() {}\n")
            .expect("write nested source file");
        tree.write_file("test-backend/lib.rs", "pub fn backend() {}\n")
            .expect("write sibling source file");

        init_git_repository(tree.path());
        init_git_repository(&tree.path().join("reproducer-memchr"));

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"reproducer-memchr/src/main.rs".to_string()));
        assert!(paths.contains(&"test-backend/lib.rs".to_string()));
    }

    #[test]
    /// Verify that untracked directory files are included when no exclusions apply.
    fn test_scan_git_includes_untracked_directory_files() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("unstaged/src/main.rs", "fn main() {}\n")
            .expect("write unstaged source file");
        tree.write_file("visible.txt", "visible\n")
            .expect("write visible file");

        init_git_repository(tree.path());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"unstaged/src/main.rs".to_string()));
        assert!(paths.contains(&"visible.txt".to_string()));
    }

    #[test]
    /// Verify that Git scans keep reincluded descendants visible unless `.ignore` excludes them.
    fn test_scan_git_reincluded_directory_uses_picker_rules_for_descendants() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "ignored-by-gitignore/\ntarget\n")
            .expect("write gitignore file");
        tree.write_file(
            ".ignore",
            "!/ignored-by-gitignore/\n!/ignored-by-gitignore/reincluded/\n",
        )
        .expect("write ignore file");
        tree.write_file(
            "ignored-by-gitignore/reincluded/src/main.rs",
            "fn main() {}\n",
        )
        .expect("write reincluded source file");
        tree.write_file(
            "ignored-by-gitignore/reincluded/target/CACHEDIR.TAG",
            "signature\n",
        )
        .expect("write target marker file");

        init_git_repository(tree.path());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"ignored-by-gitignore/reincluded/src/main.rs".to_string()));
        assert!(
            !paths
                .iter()
                .any(|path| path.contains("ignored-by-gitignore/reincluded/target"))
        );
    }

    #[test]
    /// Verify that parent `target/` exclusions still apply inside `.ignore` reinclusions.
    fn test_scan_git_keeps_parent_target_exclusion_inside_reincluded_directory() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "ignored-by-gitignore/\n")
            .expect("write gitignore file");
        tree.write_file(
            ".ignore",
            "!/ignored-by-gitignore/\n!/ignored-by-gitignore/reincluded/\ntarget/\n",
        )
        .expect("write ignore file");
        tree.write_file(
            "ignored-by-gitignore/reincluded/src/main.rs",
            "fn main() {}\n",
        )
        .expect("write reincluded source file");
        tree.write_file(
            "ignored-by-gitignore/reincluded/target/output.o",
            "object\n",
        )
        .expect("write target artifact");

        init_git_repository(tree.path());

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_git_tracked_and_untracked(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan git worktree")
        .expect("git scan summary");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"ignored-by-gitignore/reincluded/src/main.rs".to_string()));
        assert!(
            !paths
                .iter()
                .any(|path| path.contains("ignored-by-gitignore/reincluded/target"))
        );
    }

    #[test]
    /// Verify that fallback filesystem scans only emit files, not directory names.
    fn test_scan_filesystem_only_emits_files() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write visible file");
        fs::create_dir_all(tree.path().join("empty_dir")).expect("create empty directory");

        let (sender, receiver) = mpsc::channel();
        let mut ignore_matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let summary = scan_filesystem(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
            &mut ignore_matcher,
        )
        .expect("scan filesystem");

        let mut paths = Vec::new();
        while let Ok(FilePickerEvent::Batch(batch)) = receiver.try_recv() {
            paths.extend(batch);
        }

        assert_eq!(summary, ScanSummary::default());
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(!paths.contains(&"empty_dir".to_string()));
    }

    #[test]
    fn test_display_picker_path_preserves_leading_slash_for_disk_root() {
        assert_eq!(
            display_picker_path(Path::new("/"), Path::new("tmp/example.txt")),
            "/tmp/example.txt"
        );
        assert_eq!(
            display_picker_path(Path::new("/tmp/project"), Path::new("src/main.rs")),
            "src/main.rs"
        );
    }

    #[test]
    fn test_file_picker_popup_title_shows_spinner_while_scanning() {
        let picker = FilePickerState {
            picker: PickerState::new(Vec::new()),
            scan: Some(FilePickerScan {
                receiver: mpsc::channel().1,
                cancel: Arc::new(AtomicBool::new(false)),
                started_at: Instant::now(),
            }),
            next_order: 0,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        };

        let popup = picker.popup("", 0, 10);

        assert_eq!(popup.title, "Files");
        assert_eq!(popup.query_suffix, "⠋ 0/0 ");
    }

    #[test]
    /// Idle file pickers should show filtered and total fuzzy-match counts.
    fn test_file_picker_popup_query_suffix_reports_filtered_and_total_counts() {
        let mut picker = FilePickerState {
            picker: PickerState::new(vec![
                FilePickerItem {
                    path: "src/main.rs".to_string(),
                    file_name: "main.rs".to_string(),
                    order: 0,
                },
                FilePickerItem {
                    path: "src/lib.rs".to_string(),
                    file_name: "lib.rs".to_string(),
                    order: 1,
                },
            ]),
            scan: None,
            next_order: 2,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        };

        // Applying a narrowed query should update the filtered numerator only.
        picker.sync_query("main");
        let popup = picker.popup("main", 4, 10);

        assert_eq!(popup.query_suffix, "1/2 ");
    }

    #[test]
    fn test_file_picker_defers_query_filtering_until_typing_pauses() {
        let mut items = (0..FILE_PICKER_DEBOUNCE_ITEM_THRESHOLD.saturating_sub(1))
            .map(|index| FilePickerItem {
                path: format!("fixture_{index:05}.txt"),
                file_name: format!("fixture_{index:05}.txt"),
                order: index,
            })
            .collect::<Vec<_>>();
        items.push(FilePickerItem {
            path: "cargo.toml".to_string(),
            file_name: "cargo.toml".to_string(),
            order: FILE_PICKER_DEBOUNCE_ITEM_THRESHOLD.saturating_sub(1),
        });

        let mut picker = FilePickerState {
            picker: PickerState::new(items),
            scan: None,
            next_order: FILE_PICKER_DEBOUNCE_ITEM_THRESHOLD,
            applied_query: String::new(),
            pending_query: None,
            query_updated_at: None,
            spinner: Spinner::new(),
        };

        picker.sync_query("car");

        assert!(picker.pending_query.is_some());
        assert_eq!(picker.selected_path(), None);

        std::thread::sleep(std::time::Duration::from_millis(110));
        let result = picker.poll("car");

        assert!(result.changed);
        assert_eq!(picker.pending_query, None);
        assert_eq!(picker.selected_path(), Some("cargo.toml"));
    }
}
