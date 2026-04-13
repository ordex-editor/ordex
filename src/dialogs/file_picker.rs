//! Asynchronous file-picker state and background scan helpers.

use super::picker::{
    MatchScore, PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState,
    fuzzy_match_score, query_excludes_candidate,
};
use crate::spinner::Spinner;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

const FILE_PICKER_BATCH_SIZE: usize = 64;
const FILE_PICKER_EVENTS_PER_POLL: usize = 4;
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

        if self.scan.is_some() {
            loop {
                // Yield after bounded work so a busy scanner cannot starve input handling.
                if processed_events >= FILE_PICKER_EVENTS_PER_POLL {
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
        if self.is_scanning() {
            popup.query_suffix = format!("{} ", self.spinner_glyph());
        }
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
    type Key = String;

    fn key(&self) -> Self::Key {
        self.path.clone()
    }

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
    match scan_git_tracked_and_untracked(root, max_files, sender, cancel) {
        Ok(Some(summary)) => return Ok(summary.status_message(max_files)),
        Ok(None) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    // Missing `git` or a non-worktree root should not disable the picker. Fall
    // back to the standard-library walk so file discovery still works anywhere.
    match scan_filesystem(root, max_files, sender, cancel) {
        Ok(summary) => Ok(summary.status_message(max_files)),
        Err(error) => Err(error),
    }
}

/// Try to stream unignored Git paths when `root` lives inside a Git work tree.
fn scan_git_tracked_and_untracked(
    root: &Path,
    max_files: usize,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<Option<ScanSummary>> {
    let mut child = match Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
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
    let mut batch = Vec::with_capacity(FILE_PICKER_BATCH_SIZE);
    let mut line = String::new();
    let mut discovered_files = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Some(ScanSummary::default()));
        }

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        let relative = line.trim_end_matches(['\n', '\r']);
        if relative.is_empty() {
            continue;
        }

        batch.push(relative.to_string());
        discovered_files += 1;
        if batch.len() >= FILE_PICKER_BATCH_SIZE {
            sender
                .send(FilePickerEvent::Batch(std::mem::take(&mut batch)))
                .ok();
        }
        if discovered_files >= max_files {
            if !batch.is_empty() {
                sender
                    .send(FilePickerEvent::Batch(std::mem::take(&mut batch)))
                    .ok();
            }
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Some(ScanSummary {
                limit_reached: true,
                skipped_entries: 0,
            }));
        }
    }

    if !batch.is_empty() {
        sender.send(FilePickerEvent::Batch(batch)).ok();
    }

    let status = child.wait()?;
    if status.success() {
        return Ok(Some(ScanSummary::default()));
    }
    Ok(None)
}

/// Recursively scan `root` with the standard library when Git metadata is unavailable.
fn scan_filesystem(
    root: &Path,
    max_files: usize,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
) -> io::Result<ScanSummary> {
    let mut batch = Vec::with_capacity(FILE_PICKER_BATCH_SIZE);
    let mut progress = FilesystemScanProgress {
        max_files,
        discovered_files: 0,
        summary: ScanSummary::default(),
    };
    walk_directory(
        root,
        Path::new(""),
        sender,
        cancel,
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
    relative_dir: &Path,
    sender: &mpsc::Sender<FilePickerEvent>,
    cancel: &AtomicBool,
    batch: &mut Vec<String>,
    progress: &mut FilesystemScanProgress,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) || progress.summary.limit_reached {
        return Ok(());
    }

    let directory_path = root.join(relative_dir);
    let read_dir = match fs::read_dir(&directory_path) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            progress.summary.skipped_entries += 1;
            if relative_dir.as_os_str().is_empty() {
                // An unreadable root leaves the picker with nowhere else to scan,
                // so the caller needs the original error instead of a silent skip.
                return Err(error);
            }
            return Ok(());
        }
    };
    let mut entries = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(_) => progress.summary.skipped_entries += 1,
        }
    }
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if cancel.load(Ordering::Relaxed) || progress.summary.limit_reached {
            return Ok(());
        }

        let file_name = entry.file_name();
        let relative_path = relative_dir.join(&file_name);
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
            walk_directory(root, &relative_path, sender, cancel, batch, progress)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempTree;

    #[test]
    /// Verify that one poll call yields even when more scan batches are already queued.
    fn test_file_picker_poll_yields_with_pending_batches() {
        let (sender, receiver) = mpsc::channel();
        for index in 0..(FILE_PICKER_EVENTS_PER_POLL + 4) {
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
        let remaining_events = picker
            .scan
            .as_ref()
            .expect("scan should remain active")
            .receiver
            .try_iter()
            .count();

        assert!(result.changed);
        assert_eq!(picker.picker.item_count(), FILE_PICKER_EVENTS_PER_POLL);
        assert_eq!(remaining_events, 4);
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
        let summary = scan_filesystem(tree.path(), 2, &sender, &AtomicBool::new(false))
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
        let summary = scan_filesystem(
            tree.path(),
            DEFAULT_FILE_PICKER_MAX_FILES,
            &sender,
            &AtomicBool::new(false),
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
        let summary =
            scan_git_tracked_and_untracked(tree.path(), 2, &sender, &AtomicBool::new(false))
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
        assert_eq!(popup.query_suffix, "⠋ ");
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
