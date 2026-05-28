//! Asynchronous file-fingerprint worker used by external-change and save-conflict checks.

use super::common::{FileFingerprint, read_fingerprint_from_disk};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

/// One completed fingerprint response drained by the UI thread.
#[derive(Debug)]
pub(crate) struct CompletedFingerprint {
    pub(crate) request_id: u64,
    pub(crate) path: PathBuf,
    pub(crate) result: io::Result<FileFingerprint>,
}

/// Session-wide fingerprint worker with restart and synchronous fallback behavior.
#[derive(Debug)]
pub(crate) struct FileFingerprintWorker {
    mode: WorkerMode,
    pending_paths: HashMap<u64, PathBuf>,
    next_request_id: u64,
    restart_attempt_used: bool,
    pending_warning: Option<String>,
}

/// Execution mode for fingerprint requests.
#[derive(Debug)]
enum WorkerMode {
    Async {
        request_sender: Sender<FingerprintRequest>,
        result_receiver: Receiver<FingerprintResponse>,
    },
    SyncFallback {
        completed: VecDeque<FingerprintResponse>,
    },
}

/// One queued background fingerprint request.
#[derive(Debug)]
struct FingerprintRequest {
    request_id: u64,
    path: PathBuf,
}

/// One finished fingerprint worker response.
#[derive(Debug)]
struct FingerprintResponse {
    request_id: u64,
    path: PathBuf,
    result: io::Result<FileFingerprint>,
}

impl Default for FileFingerprintWorker {
    /// Create one async-first fingerprint worker.
    fn default() -> Self {
        Self::new()
    }
}

impl FileFingerprintWorker {
    /// Create one async-first fingerprint worker.
    pub(crate) fn new() -> Self {
        let (request_sender, result_receiver) = spawn_worker_channels();
        Self {
            mode: WorkerMode::Async {
                request_sender,
                result_receiver,
            },
            pending_paths: HashMap::new(),
            next_request_id: 1,
            restart_attempt_used: false,
            pending_warning: None,
        }
    }

    /// Queue one fingerprint request and return its request id.
    pub(crate) fn queue_request(&mut self, path: &Path) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let path = path.to_path_buf();
        self.pending_paths.insert(request_id, path.clone());

        // Async mode stays nonblocking for the UI thread; fallback mode computes
        // immediately so behavior remains correct after repeated worker failures.
        match &self.mode {
            WorkerMode::Async { .. } => {
                if self.try_send_async_request(request_id, &path).is_err() {
                    self.handle_async_disconnect();
                }
            }
            WorkerMode::SyncFallback { .. } => {
                self.enqueue_sync_fallback_result(request_id, path);
            }
        }

        request_id
    }

    /// Drain every completed fingerprint response currently available.
    pub(crate) fn poll_completed(&mut self) -> Vec<CompletedFingerprint> {
        let mut completed = Vec::new();
        loop {
            if !self.poll_one_response(&mut completed) {
                break;
            }
        }
        completed
    }

    /// Take one pending worker warning, if any.
    pub(crate) fn take_warning(&mut self) -> Option<String> {
        self.pending_warning.take()
    }

    /// Simulate one async worker disconnect for unit tests.
    #[cfg(test)]
    pub(crate) fn simulate_disconnect_for_test(&mut self) {
        self.handle_async_disconnect();
    }

    /// Attempt to send one request to the async worker.
    fn try_send_async_request(&self, request_id: u64, path: &Path) -> io::Result<()> {
        let WorkerMode::Async { request_sender, .. } = &self.mode else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fingerprint worker is not in async mode",
            ));
        };
        request_sender
            .send(FingerprintRequest {
                request_id,
                path: path.to_path_buf(),
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "fingerprint worker stopped"))
    }

    /// Enqueue one synchronous fallback result for `request_id`.
    fn enqueue_sync_fallback_result(&mut self, request_id: u64, path: PathBuf) {
        let result = read_fingerprint_from_disk(&path);
        let WorkerMode::SyncFallback { completed } = &mut self.mode else {
            return;
        };
        completed.push_back(FingerprintResponse {
            request_id,
            path,
            result,
        });
    }

    /// Drain one response and return whether polling should continue.
    ///
    /// Returns `true` when additional responses may still be available now, and
    /// returns `false` when no immediate response remains or the worker disconnected.
    fn poll_one_response(&mut self, completed: &mut Vec<CompletedFingerprint>) -> bool {
        match &mut self.mode {
            WorkerMode::Async {
                result_receiver, ..
            } => {
                let received = result_receiver.try_recv();
                match received {
                    Ok(response) => {
                        self.pending_paths.remove(&response.request_id);
                        completed.push(CompletedFingerprint {
                            request_id: response.request_id,
                            path: response.path,
                            result: response.result,
                        });
                        true
                    }
                    Err(TryRecvError::Empty) => false,
                    Err(TryRecvError::Disconnected) => {
                        // A disconnected async worker is recovered once, then the
                        // worker permanently switches to synchronous fallback mode.
                        self.handle_async_disconnect();
                        false
                    }
                }
            }
            WorkerMode::SyncFallback { completed: queue } => {
                let Some(response) = queue.pop_front() else {
                    return false;
                };
                self.pending_paths.remove(&response.request_id);
                completed.push(CompletedFingerprint {
                    request_id: response.request_id,
                    path: response.path,
                    result: response.result,
                });
                true
            }
        }
    }

    /// Handle one async worker disconnect by restarting once, then falling back.
    fn handle_async_disconnect(&mut self) {
        const WORKER_UNAVAILABLE_MESSAGE: &str =
            "Fingerprint worker unavailable; using synchronous fingerprint checks";

        if self.restart_attempt_used {
            self.switch_to_sync_fallback(WORKER_UNAVAILABLE_MESSAGE);
            // Outstanding requests are replayed synchronously so callers still
            // receive every queued completion event.
            self.replay_pending_into_sync_fallback();
            return;
        }

        self.restart_attempt_used = true;
        self.restart_async_worker();
        if self.requeue_pending_async_requests().is_err() {
            self.switch_to_sync_fallback(WORKER_UNAVAILABLE_MESSAGE);
            self.replay_pending_into_sync_fallback();
        }
    }

    /// Replace the current mode with a fresh async worker instance.
    fn restart_async_worker(&mut self) {
        let (request_sender, result_receiver) = spawn_worker_channels();
        self.mode = WorkerMode::Async {
            request_sender,
            result_receiver,
        };
    }

    /// Requeue all pending requests into the restarted async worker.
    fn requeue_pending_async_requests(&self) -> io::Result<()> {
        let WorkerMode::Async { request_sender, .. } = &self.mode else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fingerprint worker is not in async mode",
            ));
        };

        for (request_id, path) in sorted_pending_requests(&self.pending_paths) {
            request_sender
                .send(FingerprintRequest {
                    request_id: *request_id,
                    path: path.clone(),
                })
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "fingerprint worker stopped")
                })?;
        }
        Ok(())
    }

    /// Switch to synchronous fallback mode and record one warning.
    fn switch_to_sync_fallback(&mut self, warning: &str) {
        if !matches!(self.mode, WorkerMode::SyncFallback { .. }) {
            self.mode = WorkerMode::SyncFallback {
                completed: VecDeque::new(),
            };
        }
        self.pending_warning = Some(warning.to_string());
    }

    /// Replay every pending request by computing it synchronously.
    fn replay_pending_into_sync_fallback(&mut self) {
        for (request_id, path) in pending_request_snapshot(&self.pending_paths) {
            self.enqueue_sync_fallback_result(request_id, path);
        }
    }
}

/// Return pending requests sorted by request id for deterministic processing.
fn sorted_pending_requests(pending_paths: &HashMap<u64, PathBuf>) -> Vec<(&u64, &PathBuf)> {
    let mut pending = pending_paths.iter().collect::<Vec<_>>();
    pending.sort_by_key(|(request_id, _)| *request_id);
    pending
}

/// Return a cloned snapshot of pending requests sorted by request id.
fn pending_request_snapshot(pending_paths: &HashMap<u64, PathBuf>) -> Vec<(u64, PathBuf)> {
    sorted_pending_requests(pending_paths)
        .into_iter()
        .map(|(request_id, path)| (*request_id, path.clone()))
        .collect()
}

/// Spawn one worker and return its request sender plus result receiver.
fn spawn_worker_channels() -> (Sender<FingerprintRequest>, Receiver<FingerprintResponse>) {
    let (request_sender, request_receiver) = mpsc::channel::<FingerprintRequest>();
    let (result_sender, result_receiver) = mpsc::channel::<FingerprintResponse>();
    thread::spawn(move || run_fingerprint_worker(request_receiver, result_sender));
    (request_sender, result_receiver)
}

/// Run the background worker loop until the request channel closes.
fn run_fingerprint_worker(
    request_receiver: Receiver<FingerprintRequest>,
    result_sender: Sender<FingerprintResponse>,
) {
    while let Ok(request) = request_receiver.recv() {
        let result = read_fingerprint_from_disk(&request.path);
        let _ = result_sender.send(FingerprintResponse {
            request_id: request.request_id,
            path: request.path,
            result,
        });
    }
}
