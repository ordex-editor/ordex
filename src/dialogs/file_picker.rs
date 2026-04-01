//! Asynchronous file-picker state and background scan helpers.

use super::picker::{
    MatchScore, PickerItem, PickerPopup, PickerPopupEntry, PickerPopupSpec, PickerState,
    fuzzy_match_score,
};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

const FILE_PICKER_BATCH_SIZE: usize = 64;
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
}

/// One background scan plus its cancellation handle.
#[derive(Debug)]
struct FilePickerScan {
    receiver: Receiver<FilePickerEvent>,
    cancel: Arc<AtomicBool>,
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
        }
    }

    /// Stop the background scan and release the picker worker handles.
    pub(crate) fn cancel(&mut self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        self.scan = None;
    }

    /// Return whether the file picker still has a background scan in flight.
    pub(crate) fn is_scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// Drain any pending background scan updates into the picker state.
    pub(crate) fn poll(&mut self, query: &str) -> FilePickerPollResult {
        if self.scan.is_none() {
            return FilePickerPollResult::default();
        }
        let mut result = FilePickerPollResult::default();
        let mut finished = false;

        loop {
            let event = {
                let receiver = &self
                    .scan
                    .as_ref()
                    .expect("scan should exist while polling")
                    .receiver;
                receiver.try_recv()
            };
            match event {
                Ok(FilePickerEvent::Batch(paths)) => {
                    if !paths.is_empty() {
                        let mut items = Vec::with_capacity(paths.len());
                        for path in paths {
                            items.push(self.build_item(path));
                        }
                        self.picker.extend_items(items, query);
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

        if finished {
            self.scan = None;
        }

        result
    }

    /// Refresh matches for the latest query text.
    pub(crate) fn sync_query(&mut self, query: &str) {
        self.picker.sync_query(query);
    }

    /// Move the picker selection one row up, stopping at the first row.
    pub(crate) fn move_up(&mut self) {
        self.picker.move_up();
    }

    /// Move the picker selection one row down, stopping at the last row.
    pub(crate) fn move_down(&mut self) {
        self.picker.move_down();
    }

    /// Move the picker selection one page up, stopping at the first row.
    pub(crate) fn move_page_up(&mut self, page_len: usize) {
        self.picker.move_page_up(page_len);
    }

    /// Move the picker selection one page down, stopping at the last row.
    pub(crate) fn move_page_down(&mut self, page_len: usize) {
        self.picker.move_page_down(page_len);
    }

    /// Return the selected path, if the current filter still has matches.
    pub(crate) fn selected_path(&self) -> Option<&str> {
        self.picker.selected().map(|item| item.path.as_str())
    }

    /// Build the render-facing popup snapshot for the current query and selection.
    pub(crate) fn popup(&self, query: &str, cursor_column: usize) -> PickerPopup {
        let mut popup = self.picker.popup(Self::POPUP_SPEC, query, cursor_column);
        if self.scan.is_some() && self.picker.item_count() == 0 && popup.entries.is_empty() {
            popup.empty_message = "Scanning files...".to_string();
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
}

impl FilePickerScan {
    /// Spawn the background worker that discovers files under `root`.
    fn spawn(root: PathBuf, max_files: usize) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            let status_message = match scan_files(&root, max_files, &sender, &worker_cancel) {
                Ok(Some(message)) => Some(message),
                Ok(None) => None,
                Err(error) => Some(format!("File scan failed: {error}")),
            };
            let _ = sender.send(FilePickerEvent::Finished(status_message));
        });
        Self { receiver, cancel }
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
            active: false,
            modified: false,
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
            walk_directory(root, &relative_path, sender, cancel, batch, progress)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        batch.push(relative_path.display().to_string());
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
}
