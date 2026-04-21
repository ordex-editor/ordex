//! App-owned orchestration for background LSP navigation lookups.

use super::diagnostics::{LspFileDiagnostics, should_ignore_update};
use super::progress::{LspProgressEvent, ProgressTracker};
use super::project::{WorkspaceError, detect_workspace_for_server};
use super::protocol::{LspCompletionItem, LspPosition, LspTextChange, LspWorkspaceEdit};
use super::server::{
    LspRouteKind, LspServerDescriptor, LspServerId, language_for_path, route_servers,
    supported_project_description,
};
use super::session::{
    CompletionLookupRequest, DocumentSyncRequest, HoverLookupRequest, LspSession,
    NavigationLookupRequest, RenameLookupRequest, SessionError, SessionEvent,
    SessionNavigationTarget,
};
use crate::completion::CompletionRequest;
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

/// Final outcome of one completion lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletionLookupOutcome {
    Found(Vec<LspCompletionItem>),
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

/// One completed background completion lookup routed back to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionLookupResult {
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
    /// Buffer version captured when the lookup was queued.
    pub(crate) document_version: i32,
    /// Completion request that initiated this lookup.
    pub(crate) request: CompletionRequest,
    /// Popup anchor preserved while the completion query was in flight.
    pub(crate) popup_anchor_char_idx: usize,
    /// Final server outcome for this lookup.
    pub(crate) outcome: CompletionLookupOutcome,
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

/// Stable session key that scopes reuse by workspace root and server identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    root_path: PathBuf,
    server_id: LspServerId,
}

/// Resolved reusable session plus the server metadata that owns it.
#[derive(Debug, Clone)]
struct ResolvedSession {
    key: SessionKey,
    server: &'static LspServerDescriptor,
    session: Arc<Mutex<LspSession>>,
}

/// Diagnostics update tagged with the server that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerDiagnosticsEvent {
    server_id: LspServerId,
    update: LspFileDiagnostics,
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

/// Immutable snapshot of the active buffer used for a background completion request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionRequestSnapshot {
    /// Stable source-buffer id that initiated the lookup.
    pub(crate) buffer_id: usize,
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
    /// Completion request that should receive the result.
    pub(crate) request: CompletionRequest,
    /// Popup anchor preserved while the request is in flight.
    pub(crate) popup_anchor_char_idx: usize,
    /// Recently typed trigger text used to classify immediate trigger requests.
    pub(crate) trigger_text: Option<String>,
}

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
    sessions: HashMap<SessionKey, Arc<Mutex<LspSession>>>,
    navigation_sender: Sender<NavigationLookupResult>,
    navigation_receiver: Receiver<NavigationLookupResult>,
    hover_sender: Sender<HoverLookupResult>,
    hover_receiver: Receiver<HoverLookupResult>,
    rename_sender: Sender<RenameLookupResult>,
    rename_receiver: Receiver<RenameLookupResult>,
    completion_sender: Sender<CompletionLookupResult>,
    completion_receiver: Receiver<CompletionLookupResult>,
    sync_sender: Sender<DocumentSyncOutcome>,
    sync_receiver: Receiver<DocumentSyncOutcome>,
    progress_tracker: ProgressTracker,
    progress_sender: Sender<LspProgressEvent>,
    progress_receiver: Receiver<LspProgressEvent>,
    diagnostics_sender: Sender<ServerDiagnosticsEvent>,
    diagnostics_receiver: Receiver<ServerDiagnosticsEvent>,
    diagnostics_snapshots: HashMap<(LspServerId, PathBuf), LspFileDiagnostics>,
    pending_navigation_requests: usize,
    pending_hover_requests: usize,
    pending_rename_requests: usize,
    pending_completion_requests: usize,
    pending_sync_requests: usize,
}

impl LspManager {
    /// Create one manager that spawns the default language-server executable.
    pub(crate) fn new() -> Self {
        let (navigation_sender, navigation_receiver) = mpsc::channel();
        let (hover_sender, hover_receiver) = mpsc::channel();
        let (rename_sender, rename_receiver) = mpsc::channel();
        let (completion_sender, completion_receiver) = mpsc::channel();
        let (sync_sender, sync_receiver) = mpsc::channel();
        let (progress_sender, progress_receiver) = mpsc::channel();
        let (diagnostics_sender, diagnostics_receiver) = mpsc::channel();
        Self {
            sessions: HashMap::new(),
            navigation_sender,
            navigation_receiver,
            hover_sender,
            hover_receiver,
            rename_sender,
            rename_receiver,
            completion_sender,
            completion_receiver,
            sync_sender,
            sync_receiver,
            progress_tracker: ProgressTracker::default(),
            progress_sender,
            progress_receiver,
            diagnostics_sender,
            diagnostics_receiver,
            diagnostics_snapshots: HashMap::new(),
            pending_navigation_requests: 0,
            pending_hover_requests: 0,
            pending_rename_requests: 0,
            pending_completion_requests: 0,
            pending_sync_requests: 0,
        }
    }

    /// Return the maximum cached trigger-text length for routed completion sessions.
    pub(crate) fn max_completion_trigger_chars(&self, file_path: &Path) -> usize {
        let mut max_trigger_chars = 0;
        for resolved in self.existing_completion_sessions_for_path(file_path) {
            let Ok(session) = resolved.session.try_lock() else {
                continue;
            };
            max_trigger_chars = max_trigger_chars.max(session.max_completion_trigger_chars());
        }
        max_trigger_chars
    }

    /// Return the known routed completion trigger that matches `recent_text`.
    ///
    /// Returns `Some(trigger)` when one already-initialized routed session has
    /// advertised a trigger text that matches the end of `recent_text`, and
    /// `None` when no cached provider metadata currently matches.
    pub(crate) fn matching_completion_trigger(
        &self,
        file_path: &Path,
        recent_text: &str,
    ) -> Option<String> {
        let mut best_match = None;
        let mut best_match_chars = 0;
        for resolved in self.existing_completion_sessions_for_path(file_path) {
            let Ok(session) = resolved.session.try_lock() else {
                continue;
            };
            let Some(trigger_text) = session.matching_completion_trigger(recent_text) else {
                continue;
            };
            let trigger_chars = trigger_text.chars().count();
            if trigger_chars > best_match_chars {
                best_match_chars = trigger_chars;
                best_match = Some(trigger_text);
            }
        }
        best_match
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
        let sessions = match self.route_sessions_for_path(&snapshot.file_path, LspRouteKind::Hover)
        {
            Ok(sessions) => sessions,
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
            let mut deferred_unavailable = None;
            for resolved in sessions {
                let outcome = match resolved.session.lock() {
                    Ok(mut session) => {
                        let emit_root_path = resolved.key.root_path.clone();
                        let server = resolved.server;
                        let progress_sender = progress_sender.clone();
                        let diagnostics_sender = diagnostics_sender.clone();
                        let mut emit_event = move |event| {
                            emit_session_event(
                                event,
                                &emit_root_path,
                                server,
                                &progress_sender,
                                &diagnostics_sender,
                            );
                        };
                        hover_outcome_from_result(session.lookup_hover(&request, &mut emit_event))
                    }
                    Err(_) => HoverLookupOutcome::Error(
                        "language server session became unavailable".to_string(),
                    ),
                };
                if let HoverLookupOutcome::Unavailable(message) = outcome {
                    deferred_unavailable = Some(message);
                    continue;
                }
                let _ = sender.send(HoverLookupResult {
                    buffer_id: snapshot.buffer_id,
                    lookup_token: snapshot.lookup_token,
                    document_version: snapshot.document_version,
                    outcome,
                });
                return;
            }
            let _ = sender.send(HoverLookupResult {
                buffer_id: snapshot.buffer_id,
                lookup_token: snapshot.lookup_token,
                document_version: snapshot.document_version,
                outcome: HoverLookupOutcome::Unavailable(
                    deferred_unavailable
                        .unwrap_or_else(|| "no available language server for hover".to_string()),
                ),
            });
        });
    }

    /// Start one background completion lookup from the supplied editor snapshot.
    pub(crate) fn request_completion(&mut self, snapshot: CompletionRequestSnapshot) {
        self.pending_completion_requests += 1;
        let sender = self.completion_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let CompletionRequestSnapshot {
            buffer_id,
            document_version,
            file_path,
            text,
            force_full_sync,
            changes,
            line,
            character,
            request: completion_request,
            popup_anchor_char_idx,
            trigger_text,
        } = snapshot;
        let sessions = match self.route_sessions_for_path(&file_path, LspRouteKind::Completion) {
            Ok(sessions) => sessions,
            Err(error) => {
                let _ = sender.send(CompletionLookupResult {
                    buffer_id,
                    document_version,
                    request: completion_request,
                    popup_anchor_char_idx,
                    outcome: workspace_error_completion_outcome(&error),
                });
                return;
            }
        };
        thread::spawn(move || {
            // Completion requests reuse the same background LSP flow as hover so
            // typing never blocks on transport startup or server indexing.
            let request = CompletionLookupRequest {
                document: DocumentSyncRequest {
                    file_path,
                    version: document_version,
                    text,
                    changes,
                },
                force_full_sync,
                position: LspPosition { line, character },
                trigger_text,
            };
            let mut deferred_unavailable = None;
            for resolved in sessions {
                let outcome = match resolved.session.lock() {
                    Ok(mut session) => {
                        let emit_root_path = resolved.key.root_path.clone();
                        let server = resolved.server;
                        let progress_sender = progress_sender.clone();
                        let diagnostics_sender = diagnostics_sender.clone();
                        let mut emit_event = move |event| {
                            emit_session_event(
                                event,
                                &emit_root_path,
                                server,
                                &progress_sender,
                                &diagnostics_sender,
                            );
                        };
                        completion_outcome_from_result(
                            session.lookup_completion(&request, &mut emit_event),
                        )
                    }
                    Err(_) => CompletionLookupOutcome::Error(
                        "language server session became unavailable".to_string(),
                    ),
                };
                if let CompletionLookupOutcome::Unavailable(message) = outcome {
                    deferred_unavailable = Some(message);
                    continue;
                }
                let _ = sender.send(CompletionLookupResult {
                    buffer_id,
                    document_version,
                    request: completion_request.clone(),
                    popup_anchor_char_idx,
                    outcome,
                });
                return;
            }
            let _ = sender.send(CompletionLookupResult {
                buffer_id,
                document_version,
                request: completion_request,
                popup_anchor_char_idx,
                outcome: CompletionLookupOutcome::Unavailable(
                    deferred_unavailable.unwrap_or_else(|| {
                        "no available language server for completion".to_string()
                    }),
                ),
            });
        });
    }

    /// Start one background rename lookup from the supplied editor snapshot.
    pub(crate) fn request_rename(&mut self, snapshot: RenameRequestSnapshot) {
        self.pending_rename_requests += 1;
        let sender = self.rename_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let sessions = match self.route_sessions_for_path(&snapshot.file_path, LspRouteKind::Rename)
        {
            Ok(sessions) => sessions,
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
            let mut deferred_unavailable = None;
            for resolved in sessions {
                let outcome = match resolved.session.lock() {
                    Ok(mut session) => {
                        let emit_root_path = resolved.key.root_path.clone();
                        let server = resolved.server;
                        let progress_sender = progress_sender.clone();
                        let diagnostics_sender = diagnostics_sender.clone();
                        let mut emit_event = move |event| {
                            emit_session_event(
                                event,
                                &emit_root_path,
                                server,
                                &progress_sender,
                                &diagnostics_sender,
                            );
                        };
                        rename_outcome_from_result(session.lookup_rename(&request, &mut emit_event))
                    }
                    Err(_) => RenameLookupOutcome::Error(
                        "language server session became unavailable".to_string(),
                    ),
                };
                if let RenameLookupOutcome::Unavailable(message) = outcome {
                    deferred_unavailable = Some(message);
                    continue;
                }
                let _ = sender.send(RenameLookupResult {
                    buffer_id: snapshot.buffer_id,
                    lookup_token: snapshot.lookup_token,
                    document_version: snapshot.document_version,
                    outcome,
                });
                return;
            }
            let _ = sender.send(RenameLookupResult {
                buffer_id: snapshot.buffer_id,
                lookup_token: snapshot.lookup_token,
                document_version: snapshot.document_version,
                outcome: RenameLookupOutcome::Unavailable(
                    deferred_unavailable
                        .unwrap_or_else(|| "no available language server for rename".to_string()),
                ),
            });
        });
    }

    /// Start one background navigation lookup from the supplied editor snapshot.
    fn request_navigation(&mut self, snapshot: NavigationRequestSnapshot, kind: NavigationKind) {
        self.pending_navigation_requests += 1;
        let sender = self.navigation_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let sessions =
            match self.route_sessions_for_path(&snapshot.file_path, LspRouteKind::Navigation) {
                Ok(sessions) => sessions,
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
            let mut deferred_unavailable = None;
            for resolved in sessions {
                let outcome = match resolved.session.lock() {
                    Ok(mut session) => {
                        let emit_root_path = resolved.key.root_path.clone();
                        let server = resolved.server;
                        let progress_sender = progress_sender.clone();
                        let diagnostics_sender = diagnostics_sender.clone();
                        let mut emit_event = move |event| {
                            emit_session_event(
                                event,
                                &emit_root_path,
                                server,
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
                        navigation_outcome_from_result(result)
                    }
                    Err(_) => NavigationLookupOutcome::Error(
                        "language server session became unavailable".to_string(),
                    ),
                };
                if let NavigationLookupOutcome::Unavailable(message) = outcome {
                    deferred_unavailable = Some(message);
                    continue;
                }
                let _ = sender.send(NavigationLookupResult {
                    kind,
                    buffer_id: snapshot.buffer_id,
                    lookup_token: snapshot.lookup_token,
                    document_version: snapshot.document_version,
                    outcome,
                });
                return;
            }
            let _ = sender.send(NavigationLookupResult {
                kind,
                buffer_id: snapshot.buffer_id,
                lookup_token: snapshot.lookup_token,
                document_version: snapshot.document_version,
                outcome: NavigationLookupOutcome::Unavailable(
                    deferred_unavailable.unwrap_or_else(|| {
                        "no available language server for navigation".to_string()
                    }),
                ),
            });
        });
    }

    /// Start one background document sync from the supplied editor snapshot.
    pub(crate) fn request_document_sync(&mut self, snapshot: DocumentSyncSnapshot) {
        self.pending_sync_requests += 1;
        let sender = self.sync_sender.clone();
        let progress_sender = self.progress_sender.clone();
        let diagnostics_sender = self.diagnostics_sender.clone();
        let sessions = match self.route_sessions_for_path(&snapshot.file_path, LspRouteKind::Sync) {
            Ok(sessions) => sessions,
            Err(
                WorkspaceError::UnsupportedFileType(_) | WorkspaceError::UnsupportedProject { .. },
            ) => {
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
            let mut synced_any = false;
            for resolved in sessions {
                let sync_result = match resolved.session.lock() {
                    Ok(mut session) => {
                        let emit_root_path = resolved.key.root_path.clone();
                        let server = resolved.server;
                        let progress_sender = progress_sender.clone();
                        let diagnostics_sender = diagnostics_sender.clone();
                        let mut emit_event = move |event| {
                            emit_session_event(
                                event,
                                &emit_root_path,
                                server,
                                &progress_sender,
                                &diagnostics_sender,
                            );
                        };
                        session.sync_document(&request, &mut emit_event)
                    }
                    Err(_) => Err(SessionError::Server(
                        "language server session became unavailable".to_string(),
                    )),
                };
                if sync_result.is_ok() {
                    synced_any = true;
                }
            }
            let outcome = if synced_any {
                DocumentSyncOutcome::Synced {
                    buffer_id: snapshot.buffer_id,
                    document_version: snapshot.document_version,
                }
            } else {
                DocumentSyncOutcome::Failed {
                    buffer_id: snapshot.buffer_id,
                    document_version: snapshot.document_version,
                }
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
        // Reuse the old session only when it already exists so save-as can close
        // the former URI without accidentally starting a second workspace session.
        let previous_sessions = snapshot
            .previous_file_path
            .as_ref()
            .map(|path| self.existing_sessions_for_path(path))
            .unwrap_or_default();
        let sessions = match self.route_sessions_for_path(&snapshot.file_path, LspRouteKind::Sync) {
            Ok(sessions) => sessions,
            Err(
                WorkspaceError::UnsupportedFileType(_) | WorkspaceError::UnsupportedProject { .. },
            ) => {
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
            let mut synced_any = false;
            for resolved in sessions {
                let save_result = match resolved.session.lock() {
                    Ok(mut session) => {
                        let emit_root_path = resolved.key.root_path.clone();
                        let server = resolved.server;
                        let progress_sender = progress_sender.clone();
                        let diagnostics_sender = diagnostics_sender.clone();
                        let mut emit_event = move |event| {
                            emit_session_event(
                                event,
                                &emit_root_path,
                                server,
                                &progress_sender,
                                &diagnostics_sender,
                            );
                        };
                        if let Some(previous_path) = previous_file_path.as_ref() {
                            // Save-as needs each server session to release the old URI
                            // before the replacement path becomes the authoritative one.
                            for previous in &previous_sessions {
                                if previous.key != resolved.key {
                                    continue;
                                }
                                if Arc::ptr_eq(&previous.session, &resolved.session) {
                                    let _ = session.close_document(previous_path);
                                } else if let Ok(mut previous_session) = previous.session.lock() {
                                    let _ = previous_session.close_document(previous_path);
                                }
                            }
                        }
                        session
                            .sync_document_for_save(&request, &mut emit_event)
                            .and_then(|()| session.save_document(&request.file_path, &request.text))
                            .and_then(|()| {
                                session.request_document_diagnostics(
                                    &request.file_path,
                                    document_version,
                                    &mut emit_event,
                                )
                            })
                    }
                    Err(_) => Err(SessionError::Server(
                        "language server session became unavailable".to_string(),
                    )),
                };
                if save_result.is_ok() {
                    synced_any = true;
                }
            }
            let outcome = if synced_any {
                DocumentSyncOutcome::Synced {
                    buffer_id,
                    document_version,
                }
            } else {
                DocumentSyncOutcome::Failed {
                    buffer_id,
                    document_version,
                }
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
            match self.completion_receiver.try_recv() {
                Ok(result) => {
                    self.pending_completion_requests =
                        self.pending_completion_requests.saturating_sub(1);
                    changed |= editor.apply_completion_lookup_result(result);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_completion_requests = 0;
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
                Ok(event) => {
                    changed |= self.apply_server_diagnostics(editor, event);
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
            || self.pending_completion_requests > 0
            || self.pending_sync_requests > 0
            || self.progress_tracker.has_visible_lines()
    }

    /// Return whether the app loop should keep polling idle sessions for notifications.
    pub(crate) fn should_background_poll(&self) -> bool {
        !self.sessions.is_empty() || self.has_pending_work()
    }

    /// Resolve or create the reusable sessions for one file path and route kind.
    fn route_sessions_for_path(
        &mut self,
        file_path: &Path,
        route: LspRouteKind,
    ) -> Result<Vec<ResolvedSession>, WorkspaceError> {
        let language = language_for_path(file_path)
            .ok_or_else(|| WorkspaceError::UnsupportedFileType(file_path.to_path_buf()))?;
        let servers = route_servers(language, route);
        if servers.is_empty() {
            return Err(WorkspaceError::UnsupportedFileType(file_path.to_path_buf()));
        }

        // Each route can resolve through multiple cooperating built-ins. We keep
        // the successful sessions and only surface an unsupported-project error
        // when none of the candidate servers can claim the file's workspace.
        let mut resolved = Vec::new();
        let mut fatal_error = None;
        for server in servers {
            match self.session_for_server_path(file_path, server) {
                Ok(session) => resolved.push(session),
                Err(WorkspaceError::UnsupportedProject { .. }) => {}
                Err(error) => {
                    fatal_error = Some(error);
                    break;
                }
            }
        }
        if !resolved.is_empty() {
            return Ok(resolved);
        }
        if let Some(error) = fatal_error {
            return Err(error);
        }
        Err(WorkspaceError::unsupported_project(
            file_path.to_path_buf(),
            supported_project_description(language),
        ))
    }

    /// Resolve or create the reusable session for one file path and server.
    fn session_for_server_path(
        &mut self,
        file_path: &Path,
        server: &'static LspServerDescriptor,
    ) -> Result<ResolvedSession, WorkspaceError> {
        let workspace = detect_workspace_for_server(file_path, server)?;
        let key = SessionKey {
            root_path: workspace.root_path.clone(),
            server_id: server.id,
        };
        let session = if let Some(session) = self.sessions.get(&key) {
            Arc::clone(session)
        } else {
            let session = Arc::new(Mutex::new(LspSession::new(workspace, server)));
            self.sessions.insert(key.clone(), Arc::clone(&session));
            session
        };
        Ok(ResolvedSession {
            key,
            server,
            session,
        })
    }

    /// Return the existing reusable sessions for one file path without creating them.
    fn existing_sessions_for_path(&self, file_path: &Path) -> Vec<ResolvedSession> {
        let Some(language) = language_for_path(file_path) else {
            return Vec::new();
        };
        let mut resolved = Vec::new();
        // Existing-session lookup should stay non-destructive, so it reuses only
        // already-created sessions whose server-specific project root still matches.
        for server in route_servers(language, LspRouteKind::Sync) {
            let Ok(workspace) = detect_workspace_for_server(file_path, server) else {
                continue;
            };
            let key = SessionKey {
                root_path: workspace.root_path,
                server_id: server.id,
            };
            let Some(session) = self.sessions.get(&key) else {
                continue;
            };
            resolved.push(ResolvedSession {
                key,
                server,
                session: Arc::clone(session),
            });
        }
        resolved
    }

    /// Return the existing reusable completion sessions for one file path without creating them.
    fn existing_completion_sessions_for_path(&self, file_path: &Path) -> Vec<ResolvedSession> {
        let Some(language) = language_for_path(file_path) else {
            return Vec::new();
        };
        let mut resolved = Vec::new();
        // Trigger metadata should be read from already-started sessions only so
        // main-loop polling never creates new sessions or repeats initialization.
        for server in route_servers(language, LspRouteKind::Completion) {
            let Ok(workspace) = detect_workspace_for_server(file_path, server) else {
                continue;
            };
            let key = SessionKey {
                root_path: workspace.root_path,
                server_id: server.id,
            };
            let Some(session) = self.sessions.get(&key) else {
                continue;
            };
            resolved.push(ResolvedSession {
                key,
                server,
                session: Arc::clone(session),
            });
        }
        resolved
    }

    /// Merge one per-server diagnostics snapshot and apply the combined file view.
    ///
    /// Returns `true` when the editor-visible diagnostics changed after applying
    /// the update, and `false` when the update was stale or produced no visible
    /// change.
    fn apply_server_diagnostics(
        &mut self,
        editor: &mut crate::editor_state::EditorState,
        event: ServerDiagnosticsEvent,
    ) -> bool {
        let key = (event.server_id, event.update.file_path.clone());
        // Ignore stale snapshots before touching the merged per-file cache.
        if let Some(existing) = self.diagnostics_snapshots.get(&key)
            && should_ignore_update(existing, &event.update)
        {
            return false;
        }
        if event.update.is_empty() {
            self.diagnostics_snapshots.remove(&key);
        } else {
            self.diagnostics_snapshots.insert(key, event.update.clone());
        }

        let Some(merged) = self.merged_diagnostics_for_file(&event.update.file_path) else {
            return editor.apply_lsp_file_diagnostics(LspFileDiagnostics::new(
                event.update.file_path,
                event.update.version,
                Vec::new(),
            ));
        };
        editor.apply_lsp_file_diagnostics(merged)
    }

    /// Combine all per-server diagnostics snapshots for one file into one view.
    fn merged_diagnostics_for_file(&self, file_path: &Path) -> Option<LspFileDiagnostics> {
        // Multiple LSP servers may contribute diagnostics for the same file, so
        // the editor receives one merged snapshot rather than one server clobbering
        // another server's results in the active-file cache.
        let mut saw_snapshot = false;
        let mut version = None;
        let mut diagnostics = Vec::new();
        for update in self
            .diagnostics_snapshots
            .iter()
            .filter_map(|((_, path), update)| (path == file_path).then_some(update))
        {
            saw_snapshot = true;
            version = version.max(update.version);
            diagnostics.extend(update.diagnostics.iter().cloned());
        }
        if !saw_snapshot {
            return None;
        }
        Some(LspFileDiagnostics::new(
            file_path.to_path_buf(),
            version,
            diagnostics,
        ))
    }

    /// Drain unsolicited notifications from idle sessions into the progress channel.
    fn poll_idle_sessions(&self) {
        for (key, session) in &self.sessions {
            let Ok(mut session) = session.try_lock() else {
                continue;
            };
            let progress_sender = self.progress_sender.clone();
            let diagnostics_sender = self.diagnostics_sender.clone();
            let workspace_root = key.root_path.clone();
            let server = session.server_descriptor();
            let mut emit_event = move |event| {
                emit_session_event(
                    event,
                    &workspace_root,
                    server,
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
    server: &'static LspServerDescriptor,
    progress_sender: &Sender<LspProgressEvent>,
    diagnostics_sender: &Sender<ServerDiagnosticsEvent>,
) {
    match event {
        SessionEvent::Progress(notification) => {
            let _ = progress_sender.send(LspProgressEvent {
                workspace_root: workspace_root.to_path_buf(),
                server_name: server.display_name.to_string(),
                notification,
            });
        }
        SessionEvent::Diagnostics(update) => {
            let _ = diagnostics_sender.send(ServerDiagnosticsEvent {
                server_id: server.id,
                update,
            });
        }
    }
}

/// Convert one session hover result into one manager-level outcome.
fn hover_outcome_from_result(result: Result<Option<String>, SessionError>) -> HoverLookupOutcome {
    match result {
        Ok(Some(text)) => HoverLookupOutcome::Found(text),
        Ok(None) => HoverLookupOutcome::NotFound,
        Err(SessionError::Spawn(error)) => HoverLookupOutcome::Unavailable(error.to_string()),
        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
            HoverLookupOutcome::Unavailable(
                "language server did not expose its stdio transport".to_string(),
            )
        }
        Err(SessionError::Protocol(error)) => HoverLookupOutcome::Error(error.to_string()),
        Err(SessionError::Server(error))
        | Err(SessionError::RequestCancelled(error))
        | Err(SessionError::ContentModified(error)) => HoverLookupOutcome::Error(error),
    }
}

/// Convert one session completion result into one manager-level outcome.
fn completion_outcome_from_result(
    result: Result<Vec<LspCompletionItem>, SessionError>,
) -> CompletionLookupOutcome {
    match result {
        Ok(items) if !items.is_empty() => CompletionLookupOutcome::Found(items),
        Ok(_) => CompletionLookupOutcome::NotFound,
        Err(SessionError::Spawn(error)) => CompletionLookupOutcome::Unavailable(error.to_string()),
        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
            CompletionLookupOutcome::Unavailable(
                "language server did not expose its stdio transport".to_string(),
            )
        }
        Err(SessionError::Protocol(error)) => CompletionLookupOutcome::Error(error.to_string()),
        Err(SessionError::Server(error))
        | Err(SessionError::RequestCancelled(error))
        | Err(SessionError::ContentModified(error)) => CompletionLookupOutcome::Error(error),
    }
}

/// Convert one session rename result into one manager-level outcome.
fn rename_outcome_from_result(
    result: Result<Option<LspWorkspaceEdit>, SessionError>,
) -> RenameLookupOutcome {
    match result {
        Ok(Some(edit)) => RenameLookupOutcome::Applied(edit),
        Ok(None) => RenameLookupOutcome::NotFound,
        Err(SessionError::Spawn(error)) => RenameLookupOutcome::Unavailable(error.to_string()),
        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
            RenameLookupOutcome::Unavailable(
                "language server did not expose its stdio transport".to_string(),
            )
        }
        Err(SessionError::Protocol(error)) => RenameLookupOutcome::Error(error.to_string()),
        Err(SessionError::Server(error))
        | Err(SessionError::RequestCancelled(error))
        | Err(SessionError::ContentModified(error)) => RenameLookupOutcome::Error(error),
    }
}

/// Convert one session navigation result into one manager-level outcome.
fn navigation_outcome_from_result(
    result: Result<Vec<SessionNavigationTarget>, SessionError>,
) -> NavigationLookupOutcome {
    match result {
        Ok(targets) => targets_to_outcome(targets),
        Err(SessionError::Spawn(error)) => NavigationLookupOutcome::Unavailable(error.to_string()),
        Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
            NavigationLookupOutcome::Unavailable(
                "language server did not expose its stdio transport".to_string(),
            )
        }
        Err(SessionError::Protocol(error)) => NavigationLookupOutcome::Error(error.to_string()),
        Err(SessionError::Server(error))
        | Err(SessionError::RequestCancelled(error))
        | Err(SessionError::ContentModified(error)) => NavigationLookupOutcome::Error(error),
    }
}

/// Convert a workspace discovery failure into a user-visible navigation outcome.
fn workspace_error_outcome(error: &WorkspaceError) -> NavigationLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            NavigationLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject { .. } => {
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
        WorkspaceError::UnsupportedProject { .. } => {
            HoverLookupOutcome::UnsupportedProject(error.to_string())
        }
        WorkspaceError::CurrentDirectory(_)
        | WorkspaceError::Canonicalize { .. }
        | WorkspaceError::CargoMetadata { .. } => HoverLookupOutcome::Error(error.to_string()),
    }
}

/// Convert a workspace discovery failure into a manager-level completion outcome.
fn workspace_error_completion_outcome(error: &WorkspaceError) -> CompletionLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            CompletionLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject { .. } => {
            CompletionLookupOutcome::UnsupportedProject(error.to_string())
        }
        WorkspaceError::CurrentDirectory(_)
        | WorkspaceError::Canonicalize { .. }
        | WorkspaceError::CargoMetadata { .. } => CompletionLookupOutcome::Error(error.to_string()),
    }
}

/// Convert a workspace discovery failure into a user-visible rename outcome.
fn workspace_error_rename_outcome(error: &WorkspaceError) -> RenameLookupOutcome {
    match error {
        WorkspaceError::UnsupportedFileType(_) => {
            RenameLookupOutcome::UnsupportedFile(error.to_string())
        }
        WorkspaceError::UnsupportedProject { .. } => {
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
    use crate::lsp::server::{RUFF, RUST_ANALYZER, TY};
    use test_utils::{CurrentDirectoryGuard, TempTree};

    /// Return one repository fixture path for manager tests.
    fn fixture_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    /// Verify session reuse stays scoped to one workspace root.
    #[test]
    fn test_session_for_server_path_reuses_one_session_per_workspace() {
        let mut manager = LspManager::new();
        let workspace_one_main = fixture_path("tests/fixtures/lsp/workspace_one/src/main.rs");
        let workspace_one_lib = fixture_path("tests/fixtures/lsp/workspace_one/src/lib.rs");
        let workspace_two_main = fixture_path("tests/fixtures/lsp/workspace_two/src/main.rs");

        // Opening two files from the same workspace should reuse the exact same session.
        let first = manager
            .session_for_server_path(&workspace_one_main, &RUST_ANALYZER)
            .expect("create first workspace session");
        let second = manager
            .session_for_server_path(&workspace_one_lib, &RUST_ANALYZER)
            .expect("reuse first workspace session");
        let third = manager
            .session_for_server_path(&workspace_two_main, &RUST_ANALYZER)
            .expect("create second workspace session");

        assert!(Arc::ptr_eq(&first.session, &second.session));
        assert!(!Arc::ptr_eq(&first.session, &third.session));
        assert_eq!(manager.sessions.len(), 2);
    }

    /// Verify one workspace may keep separate sessions for different servers.
    #[test]
    fn test_session_key_includes_server_identity() {
        let mut manager = LspManager::new();
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("pyproject.toml", "[project]\nname = \"fixture\"\n")
            .expect("write pyproject");
        tree.write_file("pkg/main.py", "print('hi')\n")
            .expect("write python source");
        let path = tree.path().join("pkg/main.py");

        let ty_session = manager
            .session_for_server_path(&path, &TY)
            .expect("create ty session");
        let ruff_session = manager
            .session_for_server_path(&path, &RUFF)
            .expect("create ruff session");

        assert_eq!(manager.sessions.len(), 2);
        assert_ne!(ty_session.key.server_id, ruff_session.key.server_id);
        assert!(!Arc::ptr_eq(&ty_session.session, &ruff_session.session));
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
                server_name: "rust-analyzer".to_string(),
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
                server_name: "rust-analyzer".to_string(),
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
