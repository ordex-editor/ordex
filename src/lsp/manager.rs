//! App-owned orchestration for background LSP navigation lookups.

use super::progress::{LspProgressEvent, ProgressTracker};
use super::project::{WorkspaceError, detect_workspace_for_file};
use super::protocol::{LspPosition, LspProgressNotification, LspTextChange};
use super::session::{
    DocumentSyncRequest, LookupRequest, LspSession, SessionError, SessionNavigationTarget,
};
use crate::path_utils::current_dir_relative_path;
use ropey::Rope;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

/// One jump target shown to the editor and picker UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationTarget {
    /// Canonical filesystem path for the destination file.
    pub(crate) file_path: PathBuf,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
    /// User-facing label shown in the picker UI.
    pub(crate) display_label: String,
}

/// Stable navigation requests supported by the manager/editor flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigationKind {
    Definition,
    References,
}

impl NavigationKind {
    /// Return the progress message shown while this lookup is in flight.
    pub(crate) fn resolving_message(self) -> &'static str {
        match self {
            Self::Definition => "Resolving definition...",
            Self::References => "Resolving references...",
        }
    }

    /// Return the message shown when no file is available for lookup.
    pub(crate) fn unavailable_file_message(self) -> &'static str {
        match self {
            Self::Definition => "No file is open for go-to-definition",
            Self::References => "No file is open for go-to-references",
        }
    }

    /// Return the status message shown for an empty lookup result.
    pub(crate) fn not_found_message(self) -> &'static str {
        match self {
            Self::Definition => "No definition found",
            Self::References => "No references found",
        }
    }

    /// Return the title shown for one multi-target picker.
    pub(crate) fn picker_title(self) -> &'static str {
        match self {
            Self::Definition => "Definitions",
            Self::References => "References",
        }
    }

    /// Return the empty-state text shown while filtering one location picker.
    pub(crate) fn picker_empty_message(self) -> &'static str {
        match self {
            Self::Definition => "No matching definitions",
            Self::References => "No matching references",
        }
    }
}

/// Final outcome of one navigation lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NavigationLookupOutcome {
    Single(NavigationTarget),
    Multiple(Vec<NavigationTarget>),
    NotFound,
    UnsupportedFile(String),
    UnsupportedProject(String),
    Unavailable(String),
    Error(String),
}

/// Final outcome of one hover lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HoverLookupOutcome {
    Found(String),
    NotFound,
    UnsupportedFile(String),
    UnsupportedProject(String),
    Unavailable(String),
    Error(String),
}

/// One completed background lookup routed back to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationLookupResult {
    /// Lookup kind that produced this result.
    pub(crate) kind: NavigationKind,
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
    /// Monotonic lookup token used to reject stale responses.
    pub(crate) lookup_token: u64,
    /// Buffer version captured when the lookup was queued.
    pub(crate) document_version: i32,
    /// Final server outcome for this lookup.
    pub(crate) outcome: NavigationLookupOutcome,
}

/// One completed background hover lookup routed back to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HoverLookupResult {
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
    /// Monotonic lookup token used to reject stale responses.
    pub(crate) lookup_token: u64,
    /// Buffer version captured when the lookup was queued.
    pub(crate) document_version: i32,
    /// Final server outcome for this lookup.
    pub(crate) outcome: HoverLookupOutcome,
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
pub(crate) struct LookupRequestSnapshot {
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

/// Shared request snapshot type used by navigation and hover lookups.
pub(crate) type NavigationRequestSnapshot = LookupRequestSnapshot;

/// Shared request snapshot type used by navigation and hover lookups.
pub(crate) type HoverRequestSnapshot = LookupRequestSnapshot;

/// Internal channel payload for completed lookup work.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LookupWorkerResult {
    Navigation(NavigationLookupResult),
    Hover(HoverLookupResult),
}

/// Internal lookup dispatch used to share one worker pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookupDispatchKind {
    Navigation(NavigationKind),
    Hover,
}

/// One app-owned registry of reusable workspace-scoped language-server sessions.
pub(crate) struct LspManager {
    sessions: HashMap<PathBuf, Arc<Mutex<LspSession>>>,
    server_command: PathBuf,
    lookup_sender: Sender<LookupWorkerResult>,
    lookup_receiver: Receiver<LookupWorkerResult>,
    sync_sender: Sender<DocumentSyncOutcome>,
    sync_receiver: Receiver<DocumentSyncOutcome>,
    progress_tracker: ProgressTracker,
    progress_sender: Sender<LspProgressEvent>,
    progress_receiver: Receiver<LspProgressEvent>,
    pending_lookup_requests: usize,
    pending_sync_requests: usize,
}

impl LspManager {
    /// Create one manager that spawns the default language-server executable.
    pub(crate) fn new() -> Self {
        let (lookup_sender, lookup_receiver) = mpsc::channel();
        let (sync_sender, sync_receiver) = mpsc::channel();
        let (progress_sender, progress_receiver) = mpsc::channel();
        Self {
            sessions: HashMap::new(),
            server_command: PathBuf::from("rust-analyzer"),
            lookup_sender,
            lookup_receiver,
            sync_sender,
            sync_receiver,
            progress_tracker: ProgressTracker::default(),
            progress_sender,
            progress_receiver,
            pending_lookup_requests: 0,
            pending_sync_requests: 0,
        }
    }

    /// Start one background definition lookup from the supplied editor snapshot.
    pub(crate) fn request_definition(&mut self, snapshot: NavigationRequestSnapshot) {
        self.request_lookup(snapshot, LookupDispatchKind::Navigation(NavigationKind::Definition));
    }

    /// Start one background references lookup from the supplied editor snapshot.
    pub(crate) fn request_references(&mut self, snapshot: NavigationRequestSnapshot) {
        self.request_lookup(snapshot, LookupDispatchKind::Navigation(NavigationKind::References));
    }

    /// Start one background hover lookup from the supplied editor snapshot.
    pub(crate) fn request_hover(&mut self, snapshot: HoverRequestSnapshot) {
        self.request_lookup(snapshot, LookupDispatchKind::Hover);
    }

    /// Start one background lookup from the supplied editor snapshot.
    fn request_lookup(&mut self, snapshot: LookupRequestSnapshot, kind: LookupDispatchKind) {
        self.pending_lookup_requests += 1;
        let sender = self.lookup_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let server_command = self.server_command.clone();
        let (workspace_root, session) =
            match self.session_for_path(&snapshot.file_path, &server_command) {
                Ok(session) => session,
                Err(error) => {
                    let result = workspace_error_lookup_result(&snapshot, kind, &error);
                    let _ = sender.send(stamp_lookup_result(&snapshot, result));
                    return;
                }
            };
        thread::spawn(move || {
            let request = lookup_request(&snapshot);
            let result = match session.lock() {
                Ok(mut session) => {
                    let mut emit_progress = move |notification| {
                        let _ = progress_sender.send(LspProgressEvent {
                            workspace_root: workspace_root.clone(),
                            notification,
                        });
                    };
                    lookup_session_result(&mut session, &request, kind, &mut emit_progress)
                }
                Err(_) => lookup_unavailable_result(&snapshot, kind),
            };
            let _ = sender.send(stamp_lookup_result(&snapshot, result));
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
            match self.lookup_receiver.try_recv() {
                Ok(result) => {
                    self.pending_lookup_requests = self.pending_lookup_requests.saturating_sub(1);
                    changed |= match result {
                        LookupWorkerResult::Navigation(result) => {
                            editor.apply_navigation_lookup_result(result)
                        }
                        LookupWorkerResult::Hover(result) => editor.apply_hover_lookup_result(result),
                    };
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_lookup_requests = 0;
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
        self.pending_lookup_requests > 0
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

/// Build the session lookup request shared by navigation and hover workers.
fn lookup_request(snapshot: &LookupRequestSnapshot) -> LookupRequest {
    // The worker thread needs an owned request because it outlives the editor
    // borrow that produced the snapshot.
    LookupRequest {
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
    }
}

/// Execute one background lookup against the session and normalize the result.
fn lookup_session_result(
    session: &mut LspSession,
    request: &LookupRequest,
    kind: LookupDispatchKind,
    progress_sink: &mut dyn FnMut(LspProgressNotification),
) -> LookupWorkerResult {
    match kind {
        // Navigation lookups normalize transport failures and multi-target results
        // into picker-friendly outcomes before returning to the editor thread.
        LookupDispatchKind::Navigation(navigation_kind) => {
            let outcome = match match navigation_kind {
                NavigationKind::Definition => session.lookup_definition(request, progress_sink),
                NavigationKind::References => session.lookup_references(request, progress_sink),
            } {
                Ok(targets) => targets_to_outcome(targets),
                Err(error) => session_error_navigation_outcome(error),
            };
            LookupWorkerResult::Navigation(NavigationLookupResult {
                kind: navigation_kind,
                buffer_id: 0,
                lookup_token: 0,
                document_version: 0,
                outcome,
            })
        }
        // Hover results keep their optional text so the editor can distinguish
        // between "no hover" and transport/capability failures.
        LookupDispatchKind::Hover => {
            let outcome = match session.lookup_hover(request, progress_sink) {
                Ok(Some(text)) => HoverLookupOutcome::Found(text),
                Ok(None) => HoverLookupOutcome::NotFound,
                Err(error) => session_error_hover_outcome(error),
            };
            LookupWorkerResult::Hover(HoverLookupResult {
                buffer_id: 0,
                lookup_token: 0,
                document_version: 0,
                outcome,
            })
        }
    }
}

/// Attach the snapshot identity to one worker result created by the shared pipeline.
fn stamp_lookup_result(
    snapshot: &LookupRequestSnapshot,
    result: LookupWorkerResult,
) -> LookupWorkerResult {
    match result {
        LookupWorkerResult::Navigation(mut result) => {
            // Navigation and hover share the same worker pipeline, so attach the
            // editor-owned identity fields right before crossing back the channel.
            result.buffer_id = snapshot.buffer_id;
            result.lookup_token = snapshot.lookup_token;
            result.document_version = snapshot.document_version;
            LookupWorkerResult::Navigation(result)
        }
        LookupWorkerResult::Hover(mut result) => {
            // Hover uses the same stale-result checks, so stamp the identical
            // metadata fields onto the worker result before dispatch.
            result.buffer_id = snapshot.buffer_id;
            result.lookup_token = snapshot.lookup_token;
            result.document_version = snapshot.document_version;
            LookupWorkerResult::Hover(result)
        }
    }
}

/// Convert a workspace discovery failure into the matching user-visible lookup result.
fn workspace_error_lookup_result(
    _snapshot: &LookupRequestSnapshot,
    kind: LookupDispatchKind,
    error: &WorkspaceError,
) -> LookupWorkerResult {
    match kind {
        LookupDispatchKind::Navigation(navigation_kind) => {
            // Workspace discovery failures stay distinct so the editor can show
            // the same user-facing guidance for navigation as before.
            let outcome = match error {
                WorkspaceError::UnsupportedFileType(_) => {
                    NavigationLookupOutcome::UnsupportedFile(error.to_string())
                }
                WorkspaceError::UnsupportedProject(_) => {
                    NavigationLookupOutcome::UnsupportedProject(error.to_string())
                }
                WorkspaceError::CurrentDirectory(_)
                | WorkspaceError::Canonicalize { .. }
                | WorkspaceError::CargoMetadata { .. } => {
                    NavigationLookupOutcome::Error(error.to_string())
                }
            };
            LookupWorkerResult::Navigation(NavigationLookupResult {
                kind: navigation_kind,
                buffer_id: 0,
                lookup_token: 0,
                document_version: 0,
                outcome,
            })
        }
        LookupDispatchKind::Hover => {
            // Hover mirrors the same classification but maps it to popup/status
            // outcomes instead of picker outcomes.
            let outcome = match error {
                WorkspaceError::UnsupportedFileType(_) => {
                    HoverLookupOutcome::UnsupportedFile(error.to_string())
                }
                WorkspaceError::UnsupportedProject(_) => {
                    HoverLookupOutcome::UnsupportedProject(error.to_string())
                }
                WorkspaceError::CurrentDirectory(_)
                | WorkspaceError::Canonicalize { .. }
                | WorkspaceError::CargoMetadata { .. } => HoverLookupOutcome::Error(error.to_string()),
            };
            LookupWorkerResult::Hover(HoverLookupResult {
                buffer_id: 0,
                lookup_token: 0,
                document_version: 0,
                outcome,
            })
        }
    }
}

/// Convert one poisoned or missing session into the matching user-visible lookup result.
fn lookup_unavailable_result(
    _snapshot: &LookupRequestSnapshot,
    kind: LookupDispatchKind,
) -> LookupWorkerResult {
    let message = "language server session became unavailable".to_string();
    match kind {
        LookupDispatchKind::Navigation(navigation_kind) => {
            LookupWorkerResult::Navigation(NavigationLookupResult {
                kind: navigation_kind,
                buffer_id: 0,
                lookup_token: 0,
                document_version: 0,
                outcome: NavigationLookupOutcome::Error(message),
            })
        }
        LookupDispatchKind::Hover => LookupWorkerResult::Hover(HoverLookupResult {
            buffer_id: 0,
            lookup_token: 0,
            document_version: 0,
            outcome: HoverLookupOutcome::Error(message),
        }),
    }
}

/// Convert one session error into a user-visible navigation outcome.
fn session_error_navigation_outcome(error: SessionError) -> NavigationLookupOutcome {
    match error {
        SessionError::Spawn(error) => NavigationLookupOutcome::Unavailable(error.to_string()),
        SessionError::MissingStdin | SessionError::MissingStdout => {
            NavigationLookupOutcome::Unavailable(
                "language server did not expose its stdio transport".to_string(),
            )
        }
        SessionError::Protocol(error) => NavigationLookupOutcome::Error(error.to_string()),
        SessionError::Server(error) => NavigationLookupOutcome::Error(error),
    }
}

/// Convert one session error into a user-visible hover outcome.
fn session_error_hover_outcome(error: SessionError) -> HoverLookupOutcome {
    match error {
        SessionError::Spawn(error) => HoverLookupOutcome::Unavailable(error.to_string()),
        SessionError::MissingStdin | SessionError::MissingStdout => HoverLookupOutcome::Unavailable(
            "language server did not expose its stdio transport".to_string(),
        ),
        SessionError::Protocol(error) => HoverLookupOutcome::Error(error.to_string()),
        SessionError::Server(error) => HoverLookupOutcome::Error(error),
    }
}

/// Convert one list of normalized session targets into a lookup outcome.
fn targets_to_outcome(targets: Vec<SessionNavigationTarget>) -> NavigationLookupOutcome {
    match targets.len() {
        0 => NavigationLookupOutcome::NotFound,
        1 => NavigationLookupOutcome::Single(map_navigation_target(
            targets.into_iter().next().expect("single target"),
        )),
        _ => NavigationLookupOutcome::Multiple(
            targets.into_iter().map(map_navigation_target).collect(),
        ),
    }
}

/// Convert one session-owned target into the editor-facing picker representation.
fn map_navigation_target(target: SessionNavigationTarget) -> NavigationTarget {
    NavigationTarget {
        display_label: format_navigation_label(&target.path, target.line, target.character),
        file_path: target.path,
        line: target.line,
        character: target.character,
    }
}

/// Format one navigation target label for picker display.
fn format_navigation_label(path: &Path, line: usize, character: usize) -> String {
    format!(
        "{}:{}:{}",
        current_dir_relative_path(path).display(),
        line + 1,
        character + 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::{CurrentDirectoryGuard, TempTree};

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
            SessionNavigationTarget {
                path: PathBuf::from("/tmp/a.rs"),
                line: 1,
                character: 2,
            },
            SessionNavigationTarget {
                path: PathBuf::from("/tmp/b.rs"),
                line: 3,
                character: 4,
            },
        ]);

        assert!(matches!(outcome, NavigationLookupOutcome::Multiple(_)));
    }

    /// Verify navigation labels prefer current-directory-relative paths when available.
    #[test]
    fn test_map_navigation_target_formats_relative_display_label_within_current_directory() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/app.rs", "fn main() {}\n")
            .expect("write app file");
        let _guard = CurrentDirectoryGuard::change_to(tree.path());

        let target = map_navigation_target(SessionNavigationTarget {
            path: tree.path().join("src/app.rs"),
            line: 3,
            character: 5,
        });

        assert_eq!(target.display_label, "src/app.rs:4:6");
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
