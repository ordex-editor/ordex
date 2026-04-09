//! App-owned orchestration for background LSP definition lookups.

use super::progress::{LspProgressEvent, ProgressTracker};
use super::project::{WorkspaceError, detect_workspace_for_file};
use super::protocol::{LspPosition, LspTextChange};
use super::session::{
    DefinitionLookupRequest, DocumentSyncRequest, LspSession, SessionDefinitionTarget, SessionError,
};
use ropey::Rope;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

/// One jump target shown to the editor and picker UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionTarget {
    /// Canonical filesystem path for the destination file.
    pub(crate) file_path: PathBuf,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
    /// User-facing label shown in the picker UI.
    pub(crate) display_label: String,
}

/// Final outcome of one definition lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DefinitionLookupOutcome {
    Single(DefinitionTarget),
    Multiple(Vec<DefinitionTarget>),
    NotFound,
    UnsupportedFile(String),
    UnsupportedProject(String),
    Unavailable(String),
    Error(String),
}

/// One completed background lookup routed back to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionLookupResult {
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
    /// Monotonic lookup token used to reject stale responses.
    pub(crate) lookup_token: u64,
    /// Buffer version captured when the lookup was queued.
    pub(crate) document_version: i32,
    /// Final server outcome for this lookup.
    pub(crate) outcome: DefinitionLookupOutcome,
}

/// One completed background document-sync attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentSyncOutcome {
    /// The session accepted the current version and the editor can clear queued edits.
    Synced {
        buffer_id: usize,
        document_version: i32,
    },
    /// The active file is outside the supported LSP scope for this project.
    Unsupported {
        buffer_id: usize,
        document_version: i32,
    },
    /// The sync attempt failed and the editor should keep queued edits for later fallback.
    Failed {
        buffer_id: usize,
        document_version: i32,
    },
}

/// Immutable snapshot of one buffer used for background document sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentSyncSnapshot {
    /// Stable source-buffer id that owns this document version.
    pub(crate) buffer_id: usize,
    /// Monotonic document version captured when the snapshot was queued.
    pub(crate) document_version: i32,
    /// Canonical filesystem path for the source document.
    pub(crate) file_path: PathBuf,
    /// Cheaply cloned source snapshot stored as a rope.
    pub(crate) text: Rope,
    /// Ordered edits recorded since the previous successful sync.
    pub(crate) changes: Vec<LspTextChange>,
}

/// Immutable snapshot of the active buffer used for a background lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionRequestSnapshot {
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
    /// Monotonic lookup token used to reject stale responses.
    pub(crate) lookup_token: u64,
    /// Buffer version captured when the lookup was queued.
    pub(crate) document_version: i32,
    /// Canonical filesystem path for the source document.
    pub(crate) file_path: PathBuf,
    /// Cheaply cloned source snapshot stored as a rope.
    pub(crate) text: Rope,
    /// Whether the editor still has unsaved changes in this buffer.
    pub(crate) force_full_sync: bool,
    /// Ordered edits recorded since the previous successful sync.
    pub(crate) changes: Vec<LspTextChange>,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
}

/// One app-owned registry of reusable workspace-scoped language-server sessions.
pub(crate) struct LspManager {
    sessions: HashMap<PathBuf, Arc<Mutex<LspSession>>>,
    server_command: PathBuf,
    definition_sender: Sender<DefinitionLookupResult>,
    definition_receiver: Receiver<DefinitionLookupResult>,
    sync_sender: Sender<DocumentSyncOutcome>,
    sync_receiver: Receiver<DocumentSyncOutcome>,
    progress_tracker: ProgressTracker,
    progress_sender: Sender<LspProgressEvent>,
    progress_receiver: Receiver<LspProgressEvent>,
    pending_definition_requests: usize,
    pending_sync_requests: usize,
}

impl LspManager {
    /// Create one manager that spawns the default language-server executable.
    pub(crate) fn new() -> Self {
        let (definition_sender, definition_receiver) = mpsc::channel();
        let (sync_sender, sync_receiver) = mpsc::channel();
        let (progress_sender, progress_receiver) = mpsc::channel();
        Self {
            sessions: HashMap::new(),
            server_command: PathBuf::from("rust-analyzer"),
            definition_sender,
            definition_receiver,
            sync_sender,
            sync_receiver,
            progress_tracker: ProgressTracker::default(),
            progress_sender,
            progress_receiver,
            pending_definition_requests: 0,
            pending_sync_requests: 0,
        }
    }

    /// Start one background definition lookup from the supplied editor snapshot.
    pub(crate) fn request_definition(&mut self, snapshot: DefinitionRequestSnapshot) {
        self.pending_definition_requests += 1;
        let sender = self.definition_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let server_command = self.server_command.clone();
        let (workspace_root, session) =
            match self.session_for_path(&snapshot.file_path, &server_command) {
                Ok(session) => session,
                Err(error) => {
                    let outcome = workspace_error_outcome(&error);
                    let _ = sender.send(DefinitionLookupResult {
                        buffer_id: snapshot.buffer_id,
                        lookup_token: snapshot.lookup_token,
                        document_version: snapshot.document_version,
                        outcome,
                    });
                    return;
                }
            };
        thread::spawn(move || {
            let request = DefinitionLookupRequest {
                document: DocumentSyncRequest {
                    file_path: snapshot.file_path.clone(),
                    version: snapshot.document_version,
                    text: snapshot.text.clone(),
                    changes: snapshot.changes.clone(),
                },
                force_full_sync: snapshot.force_full_sync,
                position: LspPosition {
                    line: snapshot.line,
                    character: snapshot.character,
                },
            };
            let outcome = match session.lock() {
                Ok(mut session) => {
                    let mut emit_progress = move |notification| {
                        let _ = progress_sender.send(LspProgressEvent {
                            workspace_root: workspace_root.clone(),
                            notification,
                        });
                    };
                    match session.lookup_definition(&request, &mut emit_progress) {
                        Ok(targets) => targets_to_outcome(targets),
                        Err(SessionError::Spawn(error)) => {
                            DefinitionLookupOutcome::Unavailable(error.to_string())
                        }
                        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
                            DefinitionLookupOutcome::Unavailable(
                                "language server did not expose its stdio transport".to_string(),
                            )
                        }
                        Err(SessionError::Protocol(error)) => {
                            DefinitionLookupOutcome::Error(error.to_string())
                        }
                        Err(SessionError::Server(error)) => DefinitionLookupOutcome::Error(error),
                    }
                }
                Err(_) => DefinitionLookupOutcome::Error(
                    "language-service session became unavailable".to_string(),
                ),
            };
            let _ = sender.send(DefinitionLookupResult {
                buffer_id: snapshot.buffer_id,
                lookup_token: snapshot.lookup_token,
                document_version: snapshot.document_version,
                outcome,
            });
        });
    }

    /// Start one background document sync from the supplied editor snapshot.
    pub(crate) fn request_document_sync(&mut self, snapshot: DocumentSyncSnapshot) {
        self.pending_sync_requests += 1;
        let sender = self.sync_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let server_command = self.server_command.clone();
        let (workspace_root, session) = match self
            .session_for_path(&snapshot.file_path, &server_command)
        {
            Ok(session) => session,
            Err(WorkspaceError::UnsupportedFileType(_) | WorkspaceError::UnsupportedProject(_)) => {
                let _ = sender.send(DocumentSyncOutcome::Unsupported {
                    buffer_id: snapshot.buffer_id,
                    document_version: snapshot.document_version,
                });
                return;
            }
            Err(_) => {
                let _ = sender.send(DocumentSyncOutcome::Failed {
                    buffer_id: snapshot.buffer_id,
                    document_version: snapshot.document_version,
                });
                return;
            }
        };
        thread::spawn(move || {
            let request = DocumentSyncRequest {
                file_path: snapshot.file_path,
                version: snapshot.document_version,
                text: snapshot.text,
                changes: snapshot.changes,
            };
            let outcome = match session.lock() {
                Ok(mut session) => {
                    let mut emit_progress = move |notification| {
                        let _ = progress_sender.send(LspProgressEvent {
                            workspace_root: workspace_root.clone(),
                            notification,
                        });
                    };
                    match session.sync_document(&request, &mut emit_progress) {
                        Ok(()) => DocumentSyncOutcome::Synced {
                            buffer_id: snapshot.buffer_id,
                            document_version: snapshot.document_version,
                        },
                        Err(_) => DocumentSyncOutcome::Failed {
                            buffer_id: snapshot.buffer_id,
                            document_version: snapshot.document_version,
                        },
                    }
                }
                Err(_) => DocumentSyncOutcome::Failed {
                    buffer_id: snapshot.buffer_id,
                    document_version: snapshot.document_version,
                },
            };
            let _ = sender.send(outcome);
        });
    }

    /// Drain any completed background lookups and apply them to `editor`.
    ///
    /// Returns `true` when at least one result changed visible editor state, and
    /// `false` when polling drained nothing user-visible.
    pub(crate) fn poll(&mut self, editor: &mut crate::editor_state::EditorState) -> bool {
        let mut changed = false;
        let mut saw_progress_event = false;
        self.poll_idle_sessions();
        loop {
            match self.definition_receiver.try_recv() {
                Ok(result) => {
                    self.pending_definition_requests =
                        self.pending_definition_requests.saturating_sub(1);
                    changed |= editor.apply_definition_lookup_result(result);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_definition_requests = 0;
                    break;
                }
            }
        }
        loop {
            match self.sync_receiver.try_recv() {
                Ok(outcome) => {
                    self.pending_sync_requests = self.pending_sync_requests.saturating_sub(1);
                    editor.apply_document_sync_outcome(outcome);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_sync_requests = 0;
                    break;
                }
            }
        }
        loop {
            match self.progress_receiver.try_recv() {
                Ok(event) => {
                    saw_progress_event = true;
                    changed |= editor.set_lsp_progress_lines(self.progress_tracker.apply(event));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if !saw_progress_event && self.progress_tracker.has_visible_lines() {
            // Quiet polls keep the overlay moving forward even without fresh
            // events, which lets the spinner animate and stale lines expire.
            changed |= editor.set_lsp_progress_lines(self.progress_tracker.poll_visible_lines());
        }
        changed
    }

    /// Return whether any LSP work is still running in the background.
    pub(crate) fn has_pending_work(&self) -> bool {
        self.pending_definition_requests > 0
            || self.pending_sync_requests > 0
            || self.progress_tracker.has_visible_lines()
    }

    /// Return whether the app loop should keep polling idle sessions for notifications.
    pub(crate) fn should_background_poll(&self) -> bool {
        !self.sessions.is_empty() || self.has_pending_work()
    }

    /// Resolve or create the reusable session for one file path.
    fn session_for_path(
        &mut self,
        file_path: &Path,
        server_command: &Path,
    ) -> Result<(PathBuf, Arc<Mutex<LspSession>>), WorkspaceError> {
        let workspace = detect_workspace_for_file(file_path)?;
        if let Some(session) = self.sessions.get(&workspace.root_path) {
            return Ok((workspace.root_path, Arc::clone(session)));
        }
        let root_path = workspace.root_path.clone();
        let session = Arc::new(Mutex::new(LspSession::new(
            workspace,
            server_command.to_path_buf(),
        )));
        self.sessions
            .insert(root_path.clone(), Arc::clone(&session));
        Ok((root_path, session))
    }

    /// Drain unsolicited notifications from idle sessions into the progress channel.
    fn poll_idle_sessions(&self) {
        for (workspace_root, session) in &self.sessions {
            let Ok(mut session) = session.try_lock() else {
                continue;
            };
            let progress_sender = self.progress_sender.clone();
            let workspace_root = workspace_root.clone();
            let mut emit_progress = move |notification| {
                let _ = progress_sender.send(LspProgressEvent {
                    workspace_root: workspace_root.clone(),
                    notification,
                });
            };
            let _ = session.poll_notifications(&mut emit_progress);
        }
    }
}

/// Convert a workspace discovery failure into a user-visible lookup outcome.
fn workspace_error_outcome(error: &WorkspaceError) -> DefinitionLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            DefinitionLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject(_) => {
            DefinitionLookupOutcome::UnsupportedProject(error.to_string())
        }
        WorkspaceError::CurrentDirectory(_)
        | WorkspaceError::Canonicalize { .. }
        | WorkspaceError::CargoMetadata { .. } => DefinitionLookupOutcome::Error(error.to_string()),
    }
}

/// Convert one list of normalized session targets into a lookup outcome.
fn targets_to_outcome(targets: Vec<SessionDefinitionTarget>) -> DefinitionLookupOutcome {
    match targets.len() {
        0 => DefinitionLookupOutcome::NotFound,
        1 => DefinitionLookupOutcome::Single(map_definition_target(
            targets.into_iter().next().expect("single target"),
        )),
        _ => DefinitionLookupOutcome::Multiple(
            targets.into_iter().map(map_definition_target).collect(),
        ),
    }
}

/// Convert one session-owned target into the editor-facing picker representation.
fn map_definition_target(target: SessionDefinitionTarget) -> DefinitionTarget {
    DefinitionTarget {
        display_label: format!(
            "{}:{}:{}",
            target.path.display(),
            target.line + 1,
            target.character + 1
        ),
        file_path: target.path,
        line: target.line,
        character: target.character,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Return one repository fixture path for manager tests.
    fn fixture_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    /// Verify session reuse stays scoped to one workspace root.
    #[test]
    fn test_session_for_path_reuses_one_session_per_workspace() {
        let mut manager = LspManager::new();
        let server_command = PathBuf::from("rust-analyzer");
        let workspace_one_main = fixture_path("tests/fixtures/lsp/workspace_one/src/main.rs");
        let workspace_one_lib = fixture_path("tests/fixtures/lsp/workspace_one/src/lib.rs");
        let workspace_two_main = fixture_path("tests/fixtures/lsp/workspace_two/src/main.rs");

        // Opening two files from the same workspace should reuse the exact same session.
        let (_, first) = manager
            .session_for_path(&workspace_one_main, &server_command)
            .expect("create first workspace session");
        let (_, second) = manager
            .session_for_path(&workspace_one_lib, &server_command)
            .expect("reuse first workspace session");
        let (_, third) = manager
            .session_for_path(&workspace_two_main, &server_command)
            .expect("create second workspace session");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(manager.sessions.len(), 2);
    }

    /// Confirm that the manager reports idle state before any lookups are queued.
    #[test]
    fn test_manager_starts_idle() {
        let manager = LspManager::new();

        assert!(!manager.has_pending_work());
    }

    #[test]
    fn test_targets_to_outcome_returns_multiple_when_needed() {
        let outcome = targets_to_outcome(vec![
            SessionDefinitionTarget {
                path: PathBuf::from("/tmp/a.rs"),
                line: 1,
                character: 2,
            },
            SessionDefinitionTarget {
                path: PathBuf::from("/tmp/b.rs"),
                line: 3,
                character: 4,
            },
        ]);

        assert!(matches!(outcome, DefinitionLookupOutcome::Multiple(_)));
    }

    /// Ensure fast begin/end progress bursts still leave one visible overlay frame.
    #[test]
    fn test_poll_keeps_recent_progress_visible_after_begin_end_same_cycle() {
        let mut manager = LspManager::new();
        let mut editor = crate::editor_state::EditorState::new(24);

        manager
            .progress_sender
            .send(LspProgressEvent {
                workspace_root: PathBuf::from("/tmp/workspace"),
                notification: crate::lsp::protocol::LspProgressNotification::Begin {
                    token: "cargo-index".to_string(),
                    title: "Indexing".to_string(),
                    message: Some("crate graph".to_string()),
                    percentage: Some(5),
                },
            })
            .expect("send begin progress");
        manager
            .progress_sender
            .send(LspProgressEvent {
                workspace_root: PathBuf::from("/tmp/workspace"),
                notification: crate::lsp::protocol::LspProgressNotification::End {
                    token: "cargo-index".to_string(),
                    message: Some("done".to_string()),
                },
            })
            .expect("send end progress");

        manager.poll(&mut editor);

        assert_eq!(editor.lsp_progress_lines()[0], "Indexing: crate graph (5%)");
        assert!(editor.lsp_progress_lines()[1].contains("rust-analyzer"));
        assert!(manager.has_pending_work());

        manager.poll(&mut editor);
        assert_eq!(editor.lsp_progress_lines()[0], "Indexing: crate graph (5%)");

        for _ in 0..9 {
            manager.poll(&mut editor);
        }
        assert!(editor.lsp_progress_lines().is_empty());
        assert!(!manager.has_pending_work());
    }
}
