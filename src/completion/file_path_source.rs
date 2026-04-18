//! Asynchronous file-path completion scanning.

use super::{CompletionCandidate, CompletionRequest, CompletionSourceId, normalize_text};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// One background file-path scan plus its cancellation handle.
#[derive(Debug)]
pub(crate) struct FilePathCompletionScan {
    receiver: Receiver<FilePathCompletionEvent>,
    cancel: Arc<AtomicBool>,
}

/// One completion result emitted by the background file-path worker.
#[derive(Debug)]
enum FilePathCompletionEvent {
    Finished(Vec<CompletionCandidate>),
}

/// Result of draining the background file-path worker.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FilePathPollResult {
    /// Whether the worker finished and no further polling is needed.
    pub(crate) finished: bool,
    /// Whether visible completion candidates changed on this poll.
    pub(crate) changed: bool,
    /// Completed path candidates, if the worker finished successfully.
    pub(crate) candidates: Option<Vec<CompletionCandidate>>,
}

impl FilePathCompletionScan {
    /// Spawn one background file-path scan for `request`.
    pub(crate) fn spawn(request: CompletionRequest) -> Option<Self> {
        if !request.is_file_path() {
            return None;
        }

        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            let candidates = collect_file_path_candidates(&request, &worker_cancel);
            let _ = sender.send(FilePathCompletionEvent::Finished(candidates));
        });
        Some(Self { receiver, cancel })
    }

    /// Cancel this background scan and release the worker handles.
    pub(crate) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Drain any completed path scan results.
    pub(crate) fn poll(&mut self) -> FilePathPollResult {
        match self.receiver.try_recv() {
            Ok(FilePathCompletionEvent::Finished(candidates)) => FilePathPollResult {
                finished: true,
                changed: true,
                candidates: Some(candidates),
            },
            Err(TryRecvError::Empty) => FilePathPollResult::default(),
            // A disconnected worker cannot produce more updates, so the editor
            // should stop polling even though the candidate set did not change.
            Err(TryRecvError::Disconnected) => FilePathPollResult {
                finished: true,
                changed: false,
                candidates: None,
            },
        }
    }
}

/// Collect file-path candidates for `request` on the worker thread.
fn collect_file_path_candidates(
    request: &CompletionRequest,
    cancel: &AtomicBool,
) -> Vec<CompletionCandidate> {
    let Some(path_request) = request.file_path_request() else {
        return Vec::new();
    };
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    // Collect and sort the directory entries first so ranking stays stable
    // across refreshes even when the filesystem order varies by platform.
    let Ok(read_dir) = fs::read_dir(path_request.resolved_directory()) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let file_name = match entry.file_name().into_string() {
            Ok(file_name) => file_name,
            Err(_) => continue,
        };
        entries.push(FilePathEntry {
            is_directory: file_type.is_dir(),
            normalized_name: normalize_text(&file_name),
            file_name,
        });
    }
    entries.sort_by_key(|entry| {
        (
            !entry.is_directory,
            entry.normalized_name.clone(),
            entry.file_name.clone(),
        )
    });

    let mut candidates = Vec::new();
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        if let Some(candidate) = build_candidate_for_entry(request, &entry, candidates.len()) {
            candidates.push(candidate);
        }
    }

    candidates
}

/// One filesystem entry considered for file-path completion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePathEntry {
    file_name: String,
    normalized_name: String,
    is_directory: bool,
}

/// Build one completion candidate from `entry` when it matches `request`.
fn build_candidate_for_entry(
    request: &CompletionRequest,
    entry: &FilePathEntry,
    rank: usize,
) -> Option<CompletionCandidate> {
    if !entry
        .normalized_name
        .starts_with(request.normalized_match_prefix())
    {
        return None;
    }

    let matched_text = entry.file_name.clone();
    let insert_text = request.compose_insert_text(&matched_text);
    if insert_text.chars().count() <= request.original_text().chars().count() {
        return None;
    }

    Some(CompletionCandidate {
        source_id: CompletionSourceId::FilePath,
        insert_text,
        popup_label: if entry.is_directory {
            format!("{}/", entry.file_name)
        } else {
            entry.file_name.clone()
        },
        popup_detail: Some(if entry.is_directory {
            "directory".to_string()
        } else {
            "file".to_string()
        }),
        replace_start_char_idx: request.replace_start_char_idx(),
        replace_end_char_idx: request.cursor_char_idx(),
        rank,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::{CompletionRequest, build_request_identity};
    use test_utils::TempTree;

    const TEST_BUFFER_ID: usize = 1;
    const TEST_REQUEST_GENERATION: usize = 1;

    /// Build one absolute file-path request for `text`.
    fn path_request_for(text: &str) -> CompletionRequest {
        let buffer = crate::text_buffer::TextBuffer::from_str(text);
        let identity = build_request_identity(&buffer, None, text.chars().count())
            .expect("request should exist");
        CompletionRequest::new(TEST_BUFFER_ID, TEST_REQUEST_GENERATION, identity)
    }

    #[test]
    /// Confirm file-path candidates list matching files and append `/` to directories.
    fn test_collect_file_path_candidates_lists_matching_entries() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/lib.rs", "pub fn main() {}\n")
            .expect("write file");
        tree.write_file("state/file.txt", "x\n")
            .expect("write file");
        let request = path_request_for(&format!("{}/s", tree.path().display()));
        let cancel = AtomicBool::new(false);
        let candidates = collect_file_path_candidates(&request, &cancel);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.popup_label.clone())
                .collect::<Vec<_>>(),
            vec!["src/".to_string(), "state/".to_string(),]
        );
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.insert_text.clone())
                .collect::<Vec<_>>(),
            vec![
                format!("{}/src", tree.path().display()),
                format!("{}/state", tree.path().display()),
            ]
        );
    }

    #[test]
    /// Confirm exact file matches do not repeat the already typed text.
    fn test_collect_file_path_candidates_skips_non_extending_files() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src", "x\n").expect("write file");
        let request = path_request_for(&format!("{}/src", tree.path().display()));
        let cancel = AtomicBool::new(false);
        let candidates = collect_file_path_candidates(&request, &cancel);

        assert!(candidates.is_empty());
    }
}
