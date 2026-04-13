//! App-owned orchestration for background LSP navigation lookups.

use super::diagnostics::LspFileDiagnostics;
use super::progress::{LspProgressEvent, ProgressTracker};
use super::project::{WorkspaceError, detect_workspace_for_file};
use super::protocol::{LspPosition, LspTextChange, LspWorkspaceEdit};
use super::session::{
    DocumentSyncRequest, HoverLookupRequest, LspSession, NavigationLookupRequest,
    RenameLookupRequest, SessionError, SessionEvent, SessionNavigationTarget,
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

/// Final outcome of one rename lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RenameLookupOutcome {
    Applied(LspWorkspaceEdit),
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

/// One completed background rename lookup routed back to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenameLookupResult {
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
    /// Monotonic lookup token used to reject stale responses.
    pub(crate) lookup_token: u64,
    /// Buffer version captured when the lookup was queued.
    pub(crate) document_version: i32,
    /// Final server outcome for this lookup.
    pub(crate) outcome: RenameLookupOutcome,
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

/// Immutable snapshot of one saved buffer used for save-triggered LSP sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentSaveSnapshot {
    /// Stable source-buffer id that owns this document version.
    pub(crate) buffer_id: usize,
    /// Monotonic document version captured when the save completed.
    pub(crate) document_version: i32,
    /// Previously owned filesystem path, if the save changed the document URI.
    pub(crate) previous_file_path: Option<PathBuf>,
    /// Canonical filesystem path that now owns the saved document contents.
    pub(crate) file_path: PathBuf,
    /// Cheaply cloned saved snapshot stored as a rope.
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

/// Immutable snapshot of the active buffer used for a background rename request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenameRequestSnapshot {
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
    /// Replacement symbol name chosen by the user.
    pub(crate) new_name: String,
}

/// One app-owned registry of reusable workspace-scoped language-server sessions.
pub(crate) struct LspManager {
    sessions: HashMap<PathBuf, Arc<Mutex<LspSession>>>,
    server_command: PathBuf,
    navigation_sender: Sender<NavigationLookupResult>,
    navigation_receiver: Receiver<NavigationLookupResult>,
    hover_sender: Sender<HoverLookupResult>,
    hover_receiver: Receiver<HoverLookupResult>,
    rename_sender: Sender<RenameLookupResult>,
    rename_receiver: Receiver<RenameLookupResult>,
    sync_sender: Sender<DocumentSyncOutcome>,
    sync_receiver: Receiver<DocumentSyncOutcome>,
    progress_tracker: ProgressTracker,
    progress_sender: Sender<LspProgressEvent>,
    progress_receiver: Receiver<LspProgressEvent>,
    diagnostics_sender: Sender<LspFileDiagnostics>,
    diagnostics_receiver: Receiver<LspFileDiagnostics>,
    pending_navigation_requests: usize,
    pending_hover_requests: usize,
    pending_rename_requests: usize,
    pending_sync_requests: usize,
}

impl LspManager {
    /// Create one manager that spawns the default language-server executable.
    pub(crate) fn new() -> Self {
        let (navigation_sender, navigation_receiver) = mpsc::channel();
        let (hover_sender, hover_receiver) = mpsc::channel();
        let (rename_sender, rename_receiver) = mpsc::channel();
        let (sync_sender, sync_receiver) = mpsc::channel();
        let (progress_sender, progress_receiver) = mpsc::channel();
        let (diagnostics_sender, diagnostics_receiver) = mpsc::channel();
        Self {
            sessions: HashMap::new(),
            server_command: PathBuf::from("rust-analyzer"),
            navigation_sender,
            navigation_receiver,
            hover_sender,
            hover_receiver,
            rename_sender,
            rename_receiver,
            sync_sender,
            sync_receiver,
            progress_tracker: ProgressTracker::default(),
            progress_sender,
            progress_receiver,
            diagnostics_sender,
            diagnostics_receiver,
            pending_navigation_requests: 0,
            pending_hover_requests: 0,
            pending_rename_requests: 0,
            pending_sync_requests: 0,
        }
    }

    /// Start one background definition lookup from the supplied editor snapshot.
    pub(crate) fn request_definition(&mut self, snapshot: NavigationRequestSnapshot) {
        self.request_navigation(snapshot, NavigationKind::Definition);
    }

    /// Start one background references lookup from the supplied editor snapshot.
    pub(crate) fn request_references(&mut self, snapshot: NavigationRequestSnapshot) {
        self.request_navigation(snapshot, NavigationKind::References);
    }

    /// Start one background hover lookup from the supplied editor snapshot.
    pub(crate) fn request_hover(&mut self, snapshot: HoverRequestSnapshot) {
        self.pending_hover_requests += 1;
        let sender = self.hover_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let server_command = self.server_command.clone();
        let (workspace_root, session) =
            match self.session_for_path(&snapshot.file_path, &server_command) {
                Ok(session) => session,
                Err(error) => {
                    let _ = sender.send(HoverLookupResult {
                        buffer_id: snapshot.buffer_id,
                        lookup_token: snapshot.lookup_token,
                        document_version: snapshot.document_version,
                        outcome: workspace_error_hover_outcome(&error),
                    });
                    return;
                }
            };
        thread::spawn(move || {
            let request = HoverLookupRequest {
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
                    let emit_workspace_root = workspace_root.clone();
                    let mut emit_event = move |event| {
                        emit_session_event(
                            event,
                            &emit_workspace_root,
                            &progress_sender,
                            &diagnostics_sender,
                        );
                    };
                    match session.lookup_hover(&request, &mut emit_event) {
                        Ok(Some(text)) => HoverLookupOutcome::Found(text),
                        Ok(None) => HoverLookupOutcome::NotFound,
                        Err(SessionError::Spawn(error)) => {
                            HoverLookupOutcome::Unavailable(error.to_string())
                        }
                        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
                            HoverLookupOutcome::Unavailable(
                                "language server did not expose its stdio transport".to_string(),
                            )
                        }
                        Err(SessionError::Protocol(error)) => {
                            HoverLookupOutcome::Error(error.to_string())
                        }
                        Err(SessionError::Server(error))
                        | Err(SessionError::RequestCancelled(error))
                        | Err(SessionError::ContentModified(error)) => {
                            HoverLookupOutcome::Error(error)
                        }
                    }
                }
                Err(_) => HoverLookupOutcome::Error(
                    "language server session became unavailable".to_string(),
                ),
            };
            let _ = sender.send(HoverLookupResult {
                buffer_id: snapshot.buffer_id,
                lookup_token: snapshot.lookup_token,
                document_version: snapshot.document_version,
                outcome,
            });
        });
    }

    /// Start one background rename lookup from the supplied editor snapshot.
    pub(crate) fn request_rename(&mut self, snapshot: RenameRequestSnapshot) {
        self.pending_rename_requests += 1;
        let sender = self.rename_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let server_command = self.server_command.clone();
        let (workspace_root, session) =
            match self.session_for_path(&snapshot.file_path, &server_command) {
                Ok(session) => session,
                Err(error) => {
                    let _ = sender.send(RenameLookupResult {
                        buffer_id: snapshot.buffer_id,
                        lookup_token: snapshot.lookup_token,
                        document_version: snapshot.document_version,
                        outcome: workspace_error_rename_outcome(&error),
                    });
                    return;
                }
            };
        thread::spawn(move || {
            let request = RenameLookupRequest {
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
                new_name: snapshot.new_name,
            };
            let outcome = match session.lock() {
                Ok(mut session) => {
                    let emit_workspace_root = workspace_root.clone();
                    let mut emit_event = move |event| {
                        emit_session_event(
                            event,
                            &emit_workspace_root,
                            &progress_sender,
                            &diagnostics_sender,
                        );
                    };
                    match session.lookup_rename(&request, &mut emit_event) {
                        Ok(Some(edit)) => RenameLookupOutcome::Applied(edit),
                        Ok(None) => RenameLookupOutcome::NotFound,
                        Err(SessionError::Spawn(error)) => {
                            RenameLookupOutcome::Unavailable(error.to_string())
                        }
                        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
                            RenameLookupOutcome::Unavailable(
                                "language server did not expose its stdio transport".to_string(),
                            )
                        }
                        Err(SessionError::Protocol(error)) => {
                            RenameLookupOutcome::Error(error.to_string())
                        }
                        Err(SessionError::Server(error))
                        | Err(SessionError::RequestCancelled(error))
                        | Err(SessionError::ContentModified(error)) => {
                            RenameLookupOutcome::Error(error)
                        }
                    }
                }
                Err(_) => RenameLookupOutcome::Error(
                    "language server session became unavailable".to_string(),
                ),
            };
            let _ = sender.send(RenameLookupResult {
                buffer_id: snapshot.buffer_id,
                lookup_token: snapshot.lookup_token,
                document_version: snapshot.document_version,
                outcome,
            });
        });
    }

    /// Start one background navigation lookup from the supplied editor snapshot.
    fn request_navigation(&mut self, snapshot: NavigationRequestSnapshot, kind: NavigationKind) {
        self.pending_navigation_requests += 1;
        let sender = self.navigation_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let server_command = self.server_command.clone();
        let (workspace_root, session) =
            match self.session_for_path(&snapshot.file_path, &server_command) {
                Ok(session) => session,
                Err(error) => {
                    let _ = sender.send(NavigationLookupResult {
                        kind,
                        buffer_id: snapshot.buffer_id,
                        lookup_token: snapshot.lookup_token,
                        document_version: snapshot.document_version,
                        outcome: workspace_error_outcome(&error),
                    });
                    return;
                }
            };
        thread::spawn(move || {
            let request = NavigationLookupRequest {
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
                    let emit_workspace_root = workspace_root.clone();
                    let mut emit_event = move |event| {
                        emit_session_event(
                            event,
                            &emit_workspace_root,
                            &progress_sender,
                            &diagnostics_sender,
                        );
                    };
                    let result = match kind {
                        NavigationKind::Definition => {
                            session.lookup_definition(&request, &mut emit_event)
                        }
                        NavigationKind::References => {
                            session.lookup_references(&request, &mut emit_event)
                        }
                    };
                    match result {
                        Ok(targets) => targets_to_outcome(targets),
                        Err(SessionError::Spawn(error)) => {
                            NavigationLookupOutcome::Unavailable(error.to_string())
                        }
                        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
                            NavigationLookupOutcome::Unavailable(
                                "language server did not expose its stdio transport".to_string(),
                            )
                        }
                        Err(SessionError::Protocol(error)) => {
                            NavigationLookupOutcome::Error(error.to_string())
                        }
                        Err(SessionError::Server(error))
                        | Err(SessionError::RequestCancelled(error))
                        | Err(SessionError::ContentModified(error)) => {
                            NavigationLookupOutcome::Error(error)
                        }
                    }
                }
                Err(_) => NavigationLookupOutcome::Error(
                    "language server session became unavailable".to_string(),
                ),
            };
            let _ = sender.send(NavigationLookupResult {
                kind,
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
        let diagnostics_sender = self.diagnostics_sender.clone();
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
                    let mut emit_event = move |event| {
                        emit_session_event(
                            event,
                            &workspace_root,
                            &progress_sender,
                            &diagnostics_sender,
                        );
                    };
                    match session.sync_document(&request, &mut emit_event) {
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

    /// Start one background save-triggered sync and `didSave` notification.
    pub(crate) fn request_document_save(&mut self, snapshot: DocumentSaveSnapshot) {
        self.pending_sync_requests += 1;
        let sender = self.sync_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let server_command = self.server_command.clone();
        // Reuse the old session only when it already exists so save-as can close
        // the former URI without accidentally starting a second workspace session.
        let previous_session = snapshot
            .previous_file_path
            .as_ref()
            .and_then(|path| self.existing_session_for_path(path));
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
            let DocumentSaveSnapshot {
                buffer_id,
                document_version,
                previous_file_path,
                file_path,
                text,
                changes,
            } = snapshot;
            let request = DocumentSyncRequest {
                file_path: file_path.clone(),
                version: document_version,
                text,
                changes,
            };
            let outcome = match session.lock() {
                Ok(mut session) => {
                    let emit_workspace_root = workspace_root.clone();
                    let mut emit_event = move |event| {
                        emit_session_event(
                            event,
                            &emit_workspace_root,
                            &progress_sender,
                            &diagnostics_sender,
                        );
                    };
                    if let Some((previous_workspace_root, previous_session)) = previous_session {
                        let previous_path = previous_file_path
                            .as_ref()
                            .expect("previous path should exist when previous session exists");
                        // Save-as changes ownership from the old URI to the new
                        // one before the fresh `didOpen` / `didSave` pair lands.
                        if previous_workspace_root == workspace_root {
                            let _ = session.close_document(previous_path);
                        } else if let Ok(mut previous_session) = previous_session.lock() {
                            let _ = previous_session.close_document(previous_path);
                        }
                    }
                    match session
                        .sync_document_for_save(&request, &mut emit_event)
                        .and_then(|()| session.save_document(&request.file_path, &request.text))
                        .and_then(|()| {
                            session.request_document_diagnostics(
                                &request.file_path,
                                document_version,
                                &mut emit_event,
                            )
                        }) {
                        Ok(()) => DocumentSyncOutcome::Synced {
                            buffer_id,
                            document_version,
                        },
                        Err(_) => DocumentSyncOutcome::Failed {
                            buffer_id,
                            document_version,
                        },
                    }
                }
                Err(_) => DocumentSyncOutcome::Failed {
                    buffer_id,
                    document_version,
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
            match self.navigation_receiver.try_recv() {
                Ok(result) => {
                    self.pending_navigation_requests =
                        self.pending_navigation_requests.saturating_sub(1);
                    changed |= editor.apply_navigation_lookup_result(result);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_navigation_requests = 0;
                    break;
                }
            }
        }
        loop {
            match self.hover_receiver.try_recv() {
                Ok(result) => {
                    self.pending_hover_requests = self.pending_hover_requests.saturating_sub(1);
                    changed |= editor.apply_hover_lookup_result(result);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_hover_requests = 0;
                    break;
                }
            }
        }
        loop {
            match self.rename_receiver.try_recv() {
                Ok(result) => {
                    self.pending_rename_requests = self.pending_rename_requests.saturating_sub(1);
                    changed |= editor.apply_rename_lookup_result(result);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_rename_requests = 0;
                    break;
                }
            }
        }
        loop {
            match self.sync_receiver.try_recv() {
                Ok(outcome) => {
                    self.pending_sync_requests = self.pending_sync_requests.saturating_sub(1);
                    changed |= editor.apply_document_sync_outcome(outcome);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_sync_requests = 0;
                    break;
                }
            }
        }
        loop {
            match self.diagnostics_receiver.try_recv() {
                Ok(update) => {
                    changed |= editor.apply_lsp_file_diagnostics(update);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
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
        self.pending_navigation_requests > 0
            || self.pending_hover_requests > 0
            || self.pending_rename_requests > 0
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

    /// Return the existing reusable session for one file path without creating it.
    fn existing_session_for_path(
        &self,
        file_path: &Path,
    ) -> Option<(PathBuf, Arc<Mutex<LspSession>>)> {
        let workspace = detect_workspace_for_file(file_path).ok()?;
        let session = self.sessions.get(&workspace.root_path)?;
        Some((workspace.root_path, Arc::clone(session)))
    }

    /// Drain unsolicited notifications from idle sessions into the progress channel.
    fn poll_idle_sessions(&self) {
        for (workspace_root, session) in &self.sessions {
            let Ok(mut session) = session.try_lock() else {
                continue;
            };
            let progress_sender = self.progress_sender.clone();
            let diagnostics_sender = self.diagnostics_sender.clone();
            let workspace_root = workspace_root.clone();
            let mut emit_event = move |event| {
                emit_session_event(
                    event,
                    &workspace_root,
                    &progress_sender,
                    &diagnostics_sender,
                );
            };
            let _ = session.poll_notifications(&mut emit_event);
        }
    }
}

/// Forward one session event into the manager's progress or diagnostics channels.
fn emit_session_event(
    event: SessionEvent,
    workspace_root: &Path,
    progress_sender: &Sender<LspProgressEvent>,
    diagnostics_sender: &Sender<LspFileDiagnostics>,
) {
    match event {
        SessionEvent::Progress(notification) => {
            let _ = progress_sender.send(LspProgressEvent {
                workspace_root: workspace_root.to_path_buf(),
                notification,
            });
        }
        SessionEvent::Diagnostics(update) => {
            let _ = diagnostics_sender.send(update);
        }
    }
}

/// Convert a workspace discovery failure into a user-visible navigation outcome.
fn workspace_error_outcome(error: &WorkspaceError) -> NavigationLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            NavigationLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject(_) => {
            NavigationLookupOutcome::UnsupportedProject(error.to_string())
        }
        WorkspaceError::CurrentDirectory(_)
        | WorkspaceError::Canonicalize { .. }
        | WorkspaceError::CargoMetadata { .. } => NavigationLookupOutcome::Error(error.to_string()),
    }
}

/// Convert a workspace discovery failure into a user-visible hover outcome.
fn workspace_error_hover_outcome(error: &WorkspaceError) -> HoverLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            HoverLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject(_) => {
            HoverLookupOutcome::UnsupportedProject(error.to_string())
        }
        WorkspaceError::CurrentDirectory(_)
        | WorkspaceError::Canonicalize { .. }
        | WorkspaceError::CargoMetadata { .. } => HoverLookupOutcome::Error(error.to_string()),
    }
}

/// Convert a workspace discovery failure into a user-visible rename outcome.
fn workspace_error_rename_outcome(error: &WorkspaceError) -> RenameLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            RenameLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject(_) => {
            RenameLookupOutcome::UnsupportedProject(error.to_string())
        }
        WorkspaceError::CurrentDirectory(_)
        | WorkspaceError::Canonicalize { .. }
        | WorkspaceError::CargoMetadata { .. } => RenameLookupOutcome::Error(error.to_string()),
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
