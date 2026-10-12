//! Shared language-server process sessions reused across requests in one workspace.

use super::configuration::LspConfigurationStore;
use super::diagnostics::{LspDiagnostic, LspFileDiagnostics};
use super::progress::LspProgressEvent;
use super::project::ProjectWorkspace;
use super::protocol::{
    CompletionProvider, DocumentDiagnosticProvider, DocumentDiagnosticReport, LspCodeAction,
    LspCompletionItem, LspLocation, LspPosition, LspProgressNotification, LspRange,
    LspResponseError, LspServerStatus, LspSignatureHelp, LspTextChange, LspWatchedFileChange,
    LspWorkspaceEdit, ProtocolError, ServerMessage, SignatureHelpProvider, TextDocumentSyncKind,
    TextDocumentSyncOptions, cancel_request_notification, code_action_request, completion_request,
    definition_request, did_change_notification, did_change_watched_files_notification,
    did_close_notification, did_open_notification, did_save_notification,
    document_diagnostic_request, exit_notification, file_uri_to_path, hover_request,
    initialize_request, initialized_notification, parse_apply_edit_request,
    parse_code_action_result, parse_completion_provider, parse_completion_result,
    parse_document_diagnostic_provider, parse_document_diagnostic_report, parse_hover_result,
    parse_location_result, parse_progress_notification, parse_publish_diagnostics_notification,
    parse_server_status_notification, parse_signature_help_provider, parse_signature_help_result,
    parse_text_document_sync_options, parse_watched_files_registrations,
    parse_watched_files_unregistrations, parse_workspace_edit_result, read_message,
    references_request, rename_request, server_request_response, server_request_result,
    shutdown_request, signature_help_request, write_message,
};
use super::server::{LspServerDescriptor, LspServerId};
use super::watched_files::{LspFileChangeKind, LspFileSystemWatcher};
use ropey::Rope;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One event forwarded from the session transport into higher-level orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionEvent {
    Progress(LspProgressNotification),
    Diagnostics(LspFileDiagnostics),
    /// Trigger metadata the running server advertised, or none once it stopped.
    Capabilities(SessionCapabilities),
    /// The server asked for open documents' diagnostics to be pulled again.
    DiagnosticRefreshRequested,
}

/// Trigger metadata one server advertised in its `initialize` result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SessionCapabilities {
    /// Completion support, including the trigger texts the server reacts to.
    pub(crate) completion: Option<CompletionProvider>,
    /// Signature-help support, including the trigger texts the server reacts to.
    pub(crate) signature_help: Option<SignatureHelpProvider>,
}

impl SessionCapabilities {
    /// Return the longest advertised completion trigger that matches `recent_text`.
    pub(crate) fn matching_completion_trigger(&self, recent_text: &str) -> Option<String> {
        self.completion
            .as_ref()
            .and_then(|provider| provider.matching_trigger_text(recent_text))
            .map(str::to_string)
    }

    /// Return the longest advertised signature-help trigger that matches `recent_text`.
    pub(crate) fn matching_signature_help_trigger(&self, recent_text: &str) -> Option<String> {
        self.signature_help
            .as_ref()
            .and_then(|provider| provider.matching_trigger_text(recent_text))
            .map(str::to_string)
    }

    /// Return the longest completion trigger text the server advertised.
    pub(crate) fn max_completion_trigger_chars(&self) -> usize {
        self.completion
            .as_ref()
            .map_or(0, CompletionProvider::max_trigger_text_chars)
    }

    /// Return the longest signature-help trigger text the server advertised.
    pub(crate) fn max_signature_help_trigger_chars(&self) -> usize {
        self.signature_help
            .as_ref()
            .map_or(0, SignatureHelpProvider::max_trigger_text_chars)
    }
}

/// One session event the manager acts on itself, tagged with its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerSessionEvent {
    /// Workspace root of the session that sent the event.
    pub(crate) workspace_root: PathBuf,
    /// Server of the session that sent the event.
    pub(crate) server_id: LspServerId,
    /// What the session reported.
    pub(crate) event: SessionEvent,
}

/// Message every waiter receives when the server's stdio transport ends.
const TRANSPORT_CLOSED_MESSAGE: &str = "language server transport closed";
/// Synthetic cancellation message used when a newer completion supersedes an older one.
const COMPLETION_SUPERSEDED_MESSAGE: &str = "completion request superseded";
/// Synthetic cancellation message used when a newer signature-help request supersedes an older one.
const SIGNATURE_HELP_SUPERSEDED_MESSAGE: &str = "signature-help request superseded";
/// LSP error code for one cancelled client request.
const REQUEST_CANCELLED_ERROR_CODE: i32 = -32800;
/// LSP error code for one request invalidated by newer document contents.
const CONTENT_MODIFIED_ERROR_CODE: i32 = -32801;
/// LSP error code for one server-side cancellation.
const SERVER_CANCELLED_ERROR_CODE: i32 = -32802;

/// Diagnostics update tagged with the server that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerDiagnosticsEvent {
    /// Server whose snapshot this is, so several servers can share one file.
    pub(crate) server_id: LspServerId,
    /// Diagnostics the server reported for one file.
    pub(crate) update: LspFileDiagnostics,
}

/// Manager-owned channels a session reports server events on.
#[derive(Clone)]
pub(crate) struct SessionEventSenders {
    /// Channel that feeds the progress overlay.
    pub(crate) progress: Sender<LspProgressEvent>,
    /// Channel that feeds the merged per-file diagnostics.
    pub(crate) diagnostics: Sender<ServerDiagnosticsEvent>,
    /// Channel for capabilities and refresh requests the manager handles itself.
    pub(crate) session: Sender<ServerSessionEvent>,
}

impl SessionEventSenders {
    /// Build senders whose receivers are already gone, for sessions nobody observes.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (progress, _) = mpsc::channel();
        let (diagnostics, _) = mpsc::channel();
        let (session, _) = mpsc::channel();
        Self {
            progress,
            diagnostics,
            session,
        }
    }

    /// Tag one session event with its origin and forward it to the matching channel.
    fn emit(
        &self,
        event: SessionEvent,
        workspace_root: &Path,
        server: &'static LspServerDescriptor,
    ) {
        // A manager that is gone has nobody left to show events to.
        match event {
            SessionEvent::Progress(notification) => {
                let _ = self.progress.send(LspProgressEvent {
                    workspace_root: workspace_root.to_path_buf(),
                    server_name: server.display_name.to_string(),
                    notification,
                });
            }
            SessionEvent::Diagnostics(update) => {
                let _ = self.diagnostics.send(ServerDiagnosticsEvent {
                    server_id: server.id,
                    update,
                });
            }
            event @ (SessionEvent::Capabilities(_) | SessionEvent::DiagnosticRefreshRequested) => {
                let _ = self.session.send(ServerSessionEvent {
                    workspace_root: workspace_root.to_path_buf(),
                    server_id: server.id,
                    event,
                });
            }
        }
    }
}

/// One synced document tracked by a shared language-server session.
///
/// Ordex keeps the editor's document version separate from the LSP transport
/// version because stale editor work must be ignored, while retransmitting the
/// same editor snapshot still needs a fresh protocol version for the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDocumentState {
    /// Most recent editor-owned document version accepted by the session.
    ///
    /// Ordex uses this version to reject stale background sync work after the
    /// active buffer has already advanced to a newer snapshot.
    pub(crate) editor_version: i32,
    /// Most recent LSP protocol version sent to the server for this document.
    ///
    /// The protocol version still has to advance when Ordex resends the same
    /// editor snapshot, because the server expects every transport update
    /// for one open document to use a strictly increasing LSP version number.
    pub(crate) protocol_version: i32,
    /// Most recent pull-diagnostics result id accepted for this document.
    pub(crate) diagnostic_result_id: Option<String>,
}

/// Input needed to synchronize one document snapshot into the LSP session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentSyncRequest {
    /// Canonical filesystem path for the source document.
    pub(crate) file_path: PathBuf,
    /// Monotonic document version sent with this snapshot.
    pub(crate) version: i32,
    /// Cheaply cloned document snapshot stored as a rope.
    pub(crate) text: Rope,
    /// Ordered edits recorded since the previous successful sync.
    pub(crate) changes: Vec<LspTextChange>,
}

/// Input needed to execute one navigation lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationLookupRequest {
    /// Document snapshot that must be visible to the server before lookup.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based lookup position in LSP coordinates.
    pub(crate) position: LspPosition,
}

/// Input needed to execute one hover lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HoverLookupRequest {
    /// Document snapshot that must be visible to the server before lookup.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based lookup position in LSP coordinates.
    pub(crate) position: LspPosition,
}

/// Input needed to execute one signature-help lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SignatureHelpLookupRequest {
    /// Document snapshot that must be visible to the server before lookup.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based lookup position in LSP coordinates.
    pub(crate) position: LspPosition,
    /// Recently typed trigger text used to classify one immediate trigger request.
    pub(crate) trigger_text: Option<String>,
    /// Whether this request refreshes an already-visible signature-help popup.
    pub(crate) is_retrigger: bool,
}

/// Input needed to execute one completion lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionLookupRequest {
    /// Document snapshot that must be visible to the server before completion.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based completion position in LSP coordinates.
    pub(crate) position: LspPosition,
    /// Recently typed trigger text used to mark one immediate trigger request.
    pub(crate) trigger_text: Option<String>,
}

/// Input needed to execute one rename lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenameLookupRequest {
    /// Document snapshot that must be visible to the server before rename.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based lookup position in LSP coordinates.
    pub(crate) position: LspPosition,
    /// Replacement symbol name chosen by the user.
    pub(crate) new_name: String,
}

/// Input needed to execute one code-action lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeActionLookupRequest {
    /// Document snapshot that must be visible to the server before lookup.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based lookup range in LSP coordinates.
    pub(crate) range: LspRange,
    /// Diagnostics relevant to the requested cursor context.
    pub(crate) diagnostics: Vec<LspDiagnostic>,
}

/// One normalized navigation location returned from the language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionNavigationTarget {
    /// Canonical filesystem path for the resolved target.
    pub(crate) path: PathBuf,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
}

/// Failure returned while starting or querying one language-server session.
#[derive(Debug)]
pub(crate) enum SessionError {
    Spawn(SessionSpawnError),
    MissingStdin,
    MissingStdout,
    Protocol(ProtocolError),
    RequestCancelled(String),
    Superseded(SupersededRequestKind),
    ContentModified(String),
    Server(String),
}

/// One locally cancelled request kind that became stale before its response arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupersededRequestKind {
    Completion,
    SignatureHelp,
}

/// One startup failure enriched with the server identity that triggered it.
#[derive(Debug)]
pub(crate) struct SessionSpawnError {
    server_name: &'static str,
    command_program: &'static str,
    source: io::Error,
}

impl SessionSpawnError {
    /// Build one spawn failure tied to one concrete server command.
    fn new(server: &'static LspServerDescriptor, source: io::Error) -> Self {
        Self {
            server_name: server.display_name,
            command_program: server.command_program(),
            source,
        }
    }

    /// Return whether the operating system could not locate the server executable.
    ///
    /// Returns `true` when the configured command was missing from `PATH`, and
    /// `false` when startup failed for any other reason.
    pub(crate) fn is_missing_from_path(&self) -> bool {
        self.source.kind() == io::ErrorKind::NotFound
    }
}

impl fmt::Display for SessionSpawnError {
    /// Format one startup failure for status messages and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_missing_from_path() {
            return write!(
                f,
                "language server \"{}\" is not in PATH; install \"{}\" or add it to PATH",
                self.server_name, self.command_program
            );
        }
        write!(
            f,
            "failed to start language server \"{}\" with \"{}\": {}",
            self.server_name, self.command_program, self.source
        )
    }
}

impl fmt::Display for SessionError {
    /// Format one session failure for status messages and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "{error}"),
            Self::MissingStdin => write!(f, "language server did not expose stdin"),
            Self::MissingStdout => write!(f, "language server did not expose stdout"),
            Self::Protocol(error) => write!(f, "{error}"),
            Self::Superseded(SupersededRequestKind::Completion) => {
                write!(f, "{COMPLETION_SUPERSEDED_MESSAGE}")
            }
            Self::Superseded(SupersededRequestKind::SignatureHelp) => {
                write!(f, "{SIGNATURE_HELP_SUPERSEDED_MESSAGE}")
            }
            Self::RequestCancelled(error) | Self::ContentModified(error) | Self::Server(error) => {
                write!(f, "{error}")
            }
        }
    }
}

impl std::error::Error for SessionError {}

impl SessionError {
    /// Return whether the server may answer the same request once its analysis catches up.
    ///
    /// Returns `true` for requests the server cancelled or invalidated because
    /// the document moved on, and `false` for every failure a retry cannot fix.
    fn is_transient(&self) -> bool {
        matches!(self, Self::RequestCancelled(_) | Self::ContentModified(_))
    }
}

impl From<ProtocolError> for SessionError {
    /// Wrap one protocol failure as a session failure.
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

/// Mutable session state that must remain consistent across concurrent requests.
struct SessionState {
    /// Open-document snapshots keyed by canonical file path.
    documents: HashMap<PathBuf, SessionDocumentState>,
    /// Tokens for progress tasks that have begun and not yet ended, used to keep
    /// navigation retries alive while the language server still reports active work.
    active_progress_tokens: HashSet<String>,
    /// Deadline that keeps empty-navigation retries alive briefly after the most
    /// recent progress event so the index can become queryable after visible work ends.
    recent_progress_deadline: Option<Instant>,
    /// Most recent workspace edit requested through `workspace/applyEdit`.
    /// One deferred workspace edit captured from `workspace/applyEdit` until the
    /// originating request consumes it.
    pending_apply_edit: Option<LspWorkspaceEdit>,
    /// Negotiated text synchronization behavior from initialize.
    text_document_sync: TextDocumentSyncOptions,
    /// Advertised pull-diagnostics support from initialize, if any.
    document_diagnostic_provider: Option<DocumentDiagnosticProvider>,
    /// Advertised completion provider metadata from initialize, if any.
    completion_provider: Option<CompletionProvider>,
    /// Advertised signature-help provider metadata from initialize, if any.
    signature_help_provider: Option<SignatureHelpProvider>,
    /// Whether a `workspace/diagnostic/refresh` request arrived and still needs
    /// one follow-up pull pass for the currently tracked open documents.
    pending_diagnostic_refresh: bool,
    /// Whether the session has seen enough startup traffic to treat lookups as warm.
    startup_ready: bool,
    /// Instant since which the server reported a non-quiescent workspace, meaning
    /// requests can still answer emptily because analysis is not loaded yet.
    workspace_loading_since: Option<Instant>,
    /// Watchers the server registered for `workspace/didChangeWatchedFiles`,
    /// keyed by the registration id that can later withdraw them.
    watched_file_registrations: HashMap<String, Vec<LspFileSystemWatcher>>,
}

impl SessionState {
    /// Extra retry window after the latest progress event so lookups can bridge
    /// the short gap between the visible progress ending and definitions resolving.
    const RECENT_PROGRESS_RETRY_WINDOW: Duration = Duration::from_millis(500);

    /// Build the state of a session whose server has not started yet.
    fn new() -> Self {
        Self {
            documents: HashMap::new(),
            active_progress_tokens: HashSet::new(),
            recent_progress_deadline: None,
            pending_apply_edit: None,
            text_document_sync: TextDocumentSyncOptions::default(),
            document_diagnostic_provider: None,
            completion_provider: None,
            signature_help_provider: None,
            pending_diagnostic_refresh: false,
            startup_ready: false,
            workspace_loading_since: None,
            watched_file_registrations: HashMap::new(),
        }
    }

    /// Forget every piece of bookkeeping that belonged to one server process.
    ///
    /// Documents, capabilities, and in-flight request state describe the process
    /// that carried them, so a session only ever holds them for its current child.
    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Record the capabilities one `initialize` result advertises.
    fn apply_capabilities(
        &mut self,
        result: Option<&json::JsonValue>,
    ) -> Result<(), ProtocolError> {
        self.text_document_sync = parse_text_document_sync_options(result)?;
        self.document_diagnostic_provider = parse_document_diagnostic_provider(result)?;
        self.completion_provider = parse_completion_provider(result)?;
        self.signature_help_provider = parse_signature_help_provider(result)?;
        // The initialize response itself says nothing about the server's
        // indexing, so warm-up starts from its first later message.
        self.startup_ready = false;
        Ok(())
    }

    /// Return whether one queued sync request can no longer advance session state.
    ///
    /// Returns `true` when the tracked document already reached `request_version`
    /// or a newer version, and `false` when applying the request would move the
    /// session forward.
    fn should_skip_document_sync(&self, file_path: &Path, request_version: i32) -> bool {
        self.documents
            .get(file_path)
            .is_some_and(|previous| previous.editor_version >= request_version)
    }

    /// Return whether the server already holds a newer snapshot of one document.
    ///
    /// Returns `true` when the tracked document is past `request_version`, so
    /// sending that version would move the server's copy backward, and `false`
    /// when the document is untracked or at most at `request_version`.
    fn holds_newer_document(&self, file_path: &Path, request_version: i32) -> bool {
        self.documents
            .get(file_path)
            .is_some_and(|previous| previous.editor_version > request_version)
    }

    /// Allocate the next LSP protocol version for one document path.
    fn next_document_protocol_version(&self, file_path: &Path, request_version: i32) -> i32 {
        self.documents
            .get(file_path)
            .map(|previous| previous.protocol_version.saturating_add(1))
            // LSP document versions must stay positive, so the first sync uses
            // version 1 when the caller has not recorded any prior version yet.
            .unwrap_or(request_version.max(1))
    }

    /// Build one `didChange` payload using incremental sync when available.
    fn change_notification(
        &self,
        request: &DocumentSyncRequest,
        protocol_version: i32,
        text: &str,
    ) -> json::JsonValue {
        let changes = if self.text_document_sync.change == TextDocumentSyncKind::Incremental
            && !request.changes.is_empty()
        {
            // Incremental-sync servers can apply the exact queued ranges, so keep
            // the coalesced edit batch instead of rebuilding a whole-document diff.
            request.changes.clone()
        } else {
            // Full-sync servers, or snapshots without a usable ranged delta, only
            // have one correct fallback: resend the current document contents.
            vec![LspTextChange {
                range: None,
                text: text.to_string(),
            }]
        };
        did_change_notification(&request.file_path, protocol_version, &changes)
    }

    /// Record that the server now holds `editor_version` of one document.
    fn record_synced_document(
        &mut self,
        file_path: &Path,
        editor_version: i32,
        protocol_version: i32,
    ) {
        // The pull-diagnostics result id describes the server's view of the
        // file, which a new snapshot of the same file does not invalidate.
        let diagnostic_result_id = self
            .documents
            .get(file_path)
            .and_then(|document| document.diagnostic_result_id.clone());
        self.documents.insert(
            file_path.to_path_buf(),
            SessionDocumentState {
                editor_version,
                protocol_version,
                diagnostic_result_id,
            },
        );
    }

    /// Update workspace-readiness tracking from one server-status notification.
    fn apply_server_status(&mut self, status: &LspServerStatus) {
        if status.quiescent {
            self.workspace_loading_since = None;
        } else if self.workspace_loading_since.is_none() {
            self.workspace_loading_since = Some(Instant::now());
        }
    }

    /// Update in-flight token tracking from one typed progress notification.
    fn apply_progress_notification(&mut self, notification: &LspProgressNotification) {
        // Retry grace is refreshed by every progress event because the server
        // can finish the visible task shortly before navigation results become ready.
        self.recent_progress_deadline = Some(Instant::now() + Self::RECENT_PROGRESS_RETRY_WINDOW);
        match notification {
            LspProgressNotification::Begin { token, .. } => {
                self.active_progress_tokens.insert(token.clone());
            }
            LspProgressNotification::Report { .. } => {}
            LspProgressNotification::End { token, .. } => {
                self.active_progress_tokens.remove(token);
            }
        }
    }

    /// Snapshot what request threads need to pace their retries.
    fn readiness(&self) -> Readiness {
        Readiness {
            startup_ready: self.startup_ready,
            workspace_loading_since: self.workspace_loading_since,
            active_progress: !self.active_progress_tokens.is_empty(),
            recent_progress_deadline: self.recent_progress_deadline,
        }
    }

    /// Return whether the server reports neither a loading workspace nor running work.
    ///
    /// Returns `true` when no progress task is open and the workspace is not
    /// loading, and `false` while the server still announces work in flight.
    fn is_idle(&self) -> bool {
        self.workspace_loading_since.is_none() && self.active_progress_tokens.is_empty()
    }

    /// Build the request payload one lookup sends.
    fn lookup_payload(
        &self,
        request_id: u64,
        file_path: &Path,
        request: &LookupRequest,
    ) -> Result<json::JsonValue, SessionError> {
        Ok(match request {
            LookupRequest::Definition(position) => {
                definition_request(request_id, file_path, *position)
            }
            LookupRequest::References(position) => {
                references_request(request_id, file_path, *position)
            }
            LookupRequest::Hover(position) => hover_request(request_id, file_path, *position),
            LookupRequest::SignatureHelp {
                position,
                trigger_text,
                is_retrigger,
            } => {
                let Some(provider) = self.signature_help_provider.as_ref() else {
                    return Err(SessionError::Server(
                        "language server does not support signature help".to_string(),
                    ));
                };
                let trigger_text = trigger_text
                    .as_deref()
                    .filter(|typed_trigger| provider.supports_trigger_text(typed_trigger));
                signature_help_request(
                    request_id,
                    file_path,
                    *position,
                    trigger_text,
                    *is_retrigger,
                )
            }
            LookupRequest::Completion {
                position,
                trigger_text,
            } => {
                let Some(provider) = self.completion_provider.as_ref() else {
                    return Err(SessionError::Server(
                        "language server does not support completions".to_string(),
                    ));
                };
                let trigger_text = trigger_text
                    .as_deref()
                    .filter(|typed_trigger| provider.supports_trigger_text(typed_trigger));
                completion_request(request_id, file_path, *position, trigger_text)
            }
            LookupRequest::Rename { position, new_name } => {
                rename_request(request_id, file_path, *position, new_name)
            }
            LookupRequest::CodeAction { range, diagnostics } => {
                code_action_request(request_id, file_path, *range, diagnostics)
            }
        })
    }

    /// Rewrite one pushed diagnostic version to the matching editor version when known.
    fn normalize_published_diagnostics_version(
        &self,
        mut diagnostics: LspFileDiagnostics,
    ) -> LspFileDiagnostics {
        if let Some(document) = self.documents.get(&diagnostics.file_path)
            && diagnostics.version == Some(document.protocol_version)
        {
            diagnostics.version = Some(document.editor_version);
        }
        diagnostics
    }

    /// Store the latest reusable pull-diagnostics result id for `file_path`, if any.
    fn apply_document_diagnostic_report(
        &mut self,
        file_path: &Path,
        report: &DocumentDiagnosticReport,
    ) {
        if let Some(document) = self.documents.get_mut(file_path)
            && report.result_id.is_some()
        {
            document.diagnostic_result_id = report.result_id.clone();
        }
    }
}

/// Framed payloads handed to the writer thread, plus the backlog they add up to.
///
/// A pipe write blocks once a server stops reading, so only the writer thread
/// touches the pipe; the driver only counts what the writer has not confirmed.
struct OutboundChannel {
    /// Channel the writer thread drains into the child's stdin, in send order.
    writer: Sender<Vec<u8>>,
    /// Bytes handed to the writer thread and not yet reported written.
    queued_bytes: usize,
}

/// Reason one payload could not be handed to the writer thread.
#[derive(Debug, PartialEq, Eq)]
enum OutboundError {
    /// The writer thread is gone, so nothing will ever send the payload.
    Closed,
    /// The backlog ceiling was reached because the server stopped reading.
    Overflow,
}

impl OutboundChannel {
    /// Ceiling on the bytes one session may hold for a server that stops reading.
    ///
    /// A healthy server drains its stdin continuously, so even a burst of
    /// whole-file syncs stays orders of magnitude below this. Reaching it means
    /// the server is wedged, and the session is torn down rather than grown.
    const MAX_OUTBOUND_QUEUE_BYTES: usize = 4 * 1024 * 1024;

    /// Wrap the sending side of one writer thread's payload channel.
    fn new(writer: Sender<Vec<u8>>) -> Self {
        Self {
            writer,
            queued_bytes: 0,
        }
    }

    /// Hand one framed payload to the writer thread without blocking.
    fn push(&mut self, framed: Vec<u8>) -> Result<(), OutboundError> {
        let length = framed.len();
        if self.queued_bytes + length > Self::MAX_OUTBOUND_QUEUE_BYTES {
            return Err(OutboundError::Overflow);
        }
        self.writer
            .send(framed)
            .map_err(|_| OutboundError::Closed)?;
        self.queued_bytes += length;
        Ok(())
    }

    /// Record that the writer thread finished handing `length` bytes to the server.
    fn record_written(&mut self, length: usize) {
        self.queued_bytes = self.queued_bytes.saturating_sub(length);
    }

    /// Return whether everything pushed so far has reached the server.
    ///
    /// Returns `true` when the writer thread has confirmed every queued byte,
    /// and `false` while some payload is still waiting or mid-write.
    fn is_drained(&self) -> bool {
        self.queued_bytes == 0
    }
}

/// Server readiness as the driver last saw it.
///
/// Retry policies on request threads read this snapshot, never session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Readiness {
    /// Whether the server has sent traffic since `initialize`, so lookups count as warm.
    startup_ready: bool,
    /// Instant since which the server has reported a workspace that is still
    /// loading, so its empty answers are not final yet.
    workspace_loading_since: Option<Instant>,
    /// Whether some progress task has begun and not yet ended.
    active_progress: bool,
    /// End of the grace window that follows the latest progress event.
    recent_progress_deadline: Option<Instant>,
}

impl Readiness {
    /// Return whether recent progress should keep retryable requests alive.
    ///
    /// Returns `true` when a progress event was observed within the active retry
    /// window, and `false` when that window has already expired or was never set.
    fn has_recent_progress(&self) -> bool {
        self.recent_progress_deadline
            .is_some_and(|deadline| Instant::now() <= deadline)
    }

    /// Return whether the server reported neither a loading workspace nor running work.
    ///
    /// Returns `true` when a readiness wait may end, and `false` while the
    /// server still announces work in flight.
    fn is_idle(&self) -> bool {
        self.workspace_loading_since.is_none() && !self.active_progress
    }

    /// Return the end of the post-progress grace window a readiness wait still has to sit out.
    ///
    /// Startup waits stop only after the server is visibly idle and this window
    /// has expired. That avoids firing requests in the gap between progress
    /// ending and symbol data becoming queryable across the workspace.
    fn pending_grace_end(&self) -> Option<Instant> {
        self.recent_progress_deadline
            .filter(|_| self.startup_ready && self.has_recent_progress())
    }

    /// Return whether one lookup may no longer retry against this session.
    ///
    /// Returns `true` once the applicable budget is spent, and `false` while the
    /// ordinary deadline is open or while the longer workspace-loading budget
    /// still applies because the server cannot answer queries yet.
    fn retry_budget_exhausted(&self, deadline: Instant) -> bool {
        let now = Instant::now();
        match self.workspace_loading_since {
            Some(loading_since) => now >= loading_since + SessionClient::WORKSPACE_LOADING_TIMEOUT,
            None => now >= deadline,
        }
    }

    /// Return whether one empty lookup response should be retried.
    ///
    /// Returns `true` when startup timing may still hide a real result, and
    /// `false` when the empty result should be treated as final.
    fn should_retry_empty_lookup(
        &self,
        started: bool,
        synced_for_lookup: bool,
        startup_ready_before_request: bool,
        deadline: Instant,
    ) -> bool {
        // A running progress task means the server is still doing visible work
        // for this session, so an empty navigation response is not final yet.
        // Fresh sessions stay retryable inside the same deadline even before a
        // progress token arrives because startup indexing may begin slightly later.
        // A short post-progress grace window covers the gap between visible LSP
        // work ending and the server serving the finished symbol data. Lookups
        // that just resent document text also retry because the server may need
        // a brief analysis pass before that snapshot becomes queryable.
        !self.retry_budget_exhausted(deadline)
            && (self.workspace_loading_since.is_some()
                || synced_for_lookup
                || started
                || !startup_ready_before_request
                || self.active_progress
                || self.has_recent_progress())
    }
}

/// One request a lookup sends, with the parameters only that kind takes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LookupRequest {
    Definition(LspPosition),
    References(LspPosition),
    Hover(LspPosition),
    SignatureHelp {
        position: LspPosition,
        /// Recently typed trigger text, kept only when the server advertises it.
        trigger_text: Option<String>,
        /// Whether this request refreshes an already-visible popup.
        is_retrigger: bool,
    },
    Completion {
        position: LspPosition,
        /// Recently typed trigger text, kept only when the server advertises it.
        trigger_text: Option<String>,
    },
    Rename {
        position: LspPosition,
        new_name: String,
    },
    CodeAction {
        range: LspRange,
        diagnostics: Vec<LspDiagnostic>,
    },
}

/// One lookup request together with the document snapshot it answers for.
struct LookupCommand {
    /// Canonical path of the document the request is about.
    file_path: PathBuf,
    /// Snapshot to sync immediately before the request, if it has to reach the server.
    document: Option<DocumentSyncRequest>,
    /// Whether `document` is resent as whole text even when already current.
    force_full: bool,
    /// Request to send once the document is in place.
    request: LookupRequest,
    /// Channel that receives the server's answer.
    reply: SyncSender<Result<LookupReply, SessionError>>,
}

/// The driver's answer to one lookup request.
struct LookupReply {
    /// The server's result, or the error it answered with.
    response: Result<Option<json::JsonValue>, SessionError>,
    /// Edit the server pushed through `workspace/applyEdit` while answering a rename.
    applied_edit: Option<LspWorkspaceEdit>,
    /// What the retry policy needs to know about this request beyond its result.
    context: ReplyContext,
}

/// Facts about one lookup request that retry policies read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplyContext {
    /// Whether the request arrived before the server finished starting.
    started: bool,
    /// Whether the request sent its document to the server first.
    synced: bool,
    /// Whether the server had shown startup traffic before the request was sent.
    startup_ready_before_request: bool,
    /// Server readiness when the answer arrived.
    readiness: Readiness,
}

/// One request in flight on the server, with whoever waits for its answer.
enum PendingRequest {
    /// A lookup, answered with the raw result for its thread to interpret.
    Lookup {
        reply: SyncSender<Result<LookupReply, SessionError>>,
        /// Whether a `workspace/applyEdit` pushed meanwhile belongs to the answer.
        rename: bool,
        /// Whether the request arrived before the server finished starting.
        started: bool,
        /// Whether the request sent its document to the server first.
        synced: bool,
        /// Whether the server had shown startup traffic before the request was sent.
        startup_ready_before_request: bool,
    },
    /// A diagnostics pull, whose report the driver applies itself.
    Diagnostics {
        file_path: PathBuf,
        /// Editor version the report describes.
        version: i32,
        reply: SyncSender<Result<(), SessionError>>,
    },
    /// The `shutdown` request, whose answer only ends the wait for it.
    Shutdown { reply: SyncSender<()> },
}

/// One open document a diagnostics refresh pass pulls again.
struct RefreshDocument {
    /// Canonical path of the document.
    file_path: PathBuf,
    /// Editor version the server holds for it.
    version: i32,
}

/// One client waiting for the server to become idle.
struct ReadinessWaiter {
    /// Instant after which the client has stopped waiting.
    deadline: Instant,
    /// Channel that receives the readiness once the server is idle.
    reply: SyncSender<Readiness>,
}

/// One message in a session driver's mailbox.
enum DriverMessage {
    /// Sync one lookup's document when given, then send its request, as one step.
    Lookup(LookupCommand),
    /// Pull diagnostics for one open document when the server supports them.
    PullDiagnostics {
        file_path: PathBuf,
        /// Editor version the pulled report describes.
        version: i32,
        /// Whether to send the previous report's result id for an incremental report.
        reuse_result_id: bool,
        reply: SyncSender<Result<(), SessionError>>,
    },
    /// Answer once the server is idle, or with its busy readiness once `deadline` passes.
    AwaitReadiness {
        deadline: Instant,
        reply: SyncSender<Readiness>,
    },
    /// Supersede the in-flight completion, which a save must not wait behind.
    CancelCompletion,
    /// Remember that a cancelled diagnostics pull needs a later refresh pass.
    QueueDiagnosticRefresh,
    /// Answer with the documents a pending refresh covers, or `None` once none is pending.
    TakeDiagnosticRefresh {
        reply: SyncSender<Option<Vec<RefreshDocument>>>,
    },
    /// The refresh pass the manager started has ended.
    DiagnosticRefreshFinished,
    /// Run one closure against the session state, so tests can observe and arrange it.
    #[cfg(test)]
    Inspect(Box<dyn FnOnce(&mut SessionState) + Send>),
    /// Bring the server's copy of one document up to `request`, as `mode` asks.
    SyncDocument {
        request: DocumentSyncRequest,
        mode: SyncMode,
        reply: SyncSender<Result<SyncOutcome, SessionError>>,
    },
    /// Forget one document and tell the server it was closed.
    CloseDocument { file_path: PathBuf },
    /// Tell the server that one synchronized document was saved.
    SaveDocument { file_path: PathBuf, text: Rope },
    /// Tell the server about one filesystem change it may have registered a watcher for.
    WatchedFileChanged {
        path: PathBuf,
        kind: LspFileChangeKind,
    },
    /// One message the reader thread of transport `generation` decoded.
    Inbound {
        generation: u64,
        message: ServerMessage,
    },
    /// Bytes the writer thread of transport `generation` handed to the server.
    Written { generation: u64, length: usize },
    /// The reader or writer thread of transport `generation` hit end of stream.
    TransportClosed { generation: u64 },
    /// Shut the server down, stop the transport, and end the driver.
    Shutdown,
}

impl DriverMessage {
    /// Return whether this command is worth starting a server for.
    ///
    /// Returns `true` for commands that need an answer from a running server,
    /// and `false` for notifications that mean nothing to a server that has
    /// not started, because it holds no documents or watchers yet.
    fn starts_server(&self) -> bool {
        matches!(self, Self::Lookup(_) | Self::SyncDocument { .. })
    }
}

/// What preparing one lookup produced, ready to send.
struct PreparedLookup {
    /// JSON-RPC id the request goes out with.
    request_id: u64,
    /// Framed-ready request payload.
    payload: json::JsonValue,
    /// Whether the lookup's document was sent first.
    synced: bool,
}

/// How one document sync treats a server that already holds the snapshot.
///
/// No mode sends a snapshot older than the one the server holds, so the
/// server's copy of a document only ever moves forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    /// Resend the whole text even when the server already holds this version.
    Full,
    /// Send the whole text unless the server holds this version or a newer one.
    FullUnlessCurrent,
    /// Send the negotiated change unless the server holds this version or a newer one.
    IncrementalUnlessCurrent,
}

/// What one document sync did, as reported back to the thread that asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncOutcome {
    /// Whether the snapshot was sent, rather than skipped as already current.
    sent: bool,
    /// Whether the sync arrived before the server finished starting.
    started: bool,
}

/// Where one session's server process is in its life.
enum Lifecycle {
    /// No server was started yet, or it was shut down.
    Stopped,
    /// `initialize` is in flight; commands wait here in arrival order, because
    /// nothing else may be sent before its response.
    Initializing {
        request_id: u64,
        queued: Vec<DriverMessage>,
    },
    /// The handshake finished and commands are served as they arrive.
    Running,
    /// The last start or the last server process failed. Like `Stopped`, the
    /// next command that needs a server starts a fresh one.
    Failed,
}

/// Server process that is killed if its owner is dropped without stopping it.
///
/// That only happens when the driver panics; a server must not outlive it.
struct ServerProcess(Child);

impl Drop for ServerProcess {
    /// Kill and reap the process, which is a no-op once it was stopped.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One running server process with the threads that carry its traffic.
struct Transport {
    /// Number that tags this transport's messages apart from older transports'.
    generation: u64,
    /// Running language-server process.
    process: ServerProcess,
    /// Payload channel to the writer thread.
    outbound: OutboundChannel,
    /// Reader thread that owns the child's stdout.
    reader_thread: JoinHandle<()>,
    /// Writer thread that owns the child's stdin.
    writer_thread: JoinHandle<()>,
}

impl Transport {
    /// Stop the child process and both transport threads.
    ///
    /// `grace` is how long the child may exit on its own before it is killed. The
    /// child goes first either way: closing its end of the pipes is what releases
    /// a writer thread parked on a full stdin buffer, so joining before that could
    /// wait for exactly the server that already stopped responding.
    fn stop(self, grace: Duration) {
        let Self {
            mut process,
            outbound,
            reader_thread,
            writer_thread,
            ..
        } = self;
        {
            // Closing the payload channel ends the writer once its current write returns.
            let _closed = outbound;
        }
        if !wait_for_graceful_shutdown(&mut process.0, grace) {
            let _ = process.0.kill();
            let _ = process.0.wait();
        }
        for thread_handle in [reader_thread, writer_thread] {
            #[allow(
                clippy::disallowed_methods,
                reason = "the child is already reaped above, so both threads are at end of pipe"
            )]
            let _ = thread_handle.join();
        }
    }
}

/// Single owner of one session's server process and the traffic to and from it.
///
/// Inbound messages are applied here as they arrive, so no other thread ever
/// processes server traffic or waits on the child's pipes.
struct SessionDriver {
    /// Workspace root the server runs in, also used to tag events and traces.
    workspace_root: PathBuf,
    /// Built-in server descriptor that owns command resolution and naming.
    server: &'static LspServerDescriptor,
    /// Configuration snapshot used to initialize the server and answer it.
    configuration: Arc<LspConfigurationStore>,
    /// Documents, capabilities, and readiness of the current server process.
    state: SessionState,
    /// Manager channels that receive this session's events.
    events: SessionEventSenders,
    /// Sending side of this driver's own mailbox, handed to transport threads.
    mailbox: Sender<DriverMessage>,
    /// JSON-RPC id the next request goes out with.
    next_request_id: u64,
    /// Where the server process is in its life.
    lifecycle: Lifecycle,
    /// Running server process and its threads, while one is starting or running.
    transport: Option<Transport>,
    /// Generation the next transport receives.
    next_generation: u64,
    /// Requests in flight on the server, keyed by JSON-RPC request id.
    pending_requests: HashMap<u64, PendingRequest>,
    /// In-flight completion request, which a newer one supersedes.
    pending_completion: Option<u64>,
    /// In-flight signature-help request, which a newer one supersedes.
    pending_signature_help: Option<u64>,
    /// Clients waiting for the server to become idle.
    readiness_waiters: Vec<ReadinessWaiter>,
    /// Whether the manager was asked for a refresh pass that has not finished yet.
    diagnostic_refresh_claimed: bool,
}

/// Owner of one reusable language-server session keyed by workspace root and server.
///
/// The session has one owner, its driver thread, which holds the server
/// process and every piece of session state. Other threads only send it
/// messages, so there is no lock to order and nothing to block under. The
/// methods here only send, so the editor thread may call them; anything that
/// waits for an answer needs a [`SessionClient`] on a worker thread.
pub(crate) struct SessionHandle {
    /// Sending side of the driver's mailbox.
    mailbox: Sender<DriverMessage>,
    /// Driver thread, joined once the handle has told it to shut down.
    driver_thread: Option<JoinHandle<()>>,
}

/// Blocking access to one session, only available on worker threads.
///
/// The only way to obtain one is [`SessionHandle::spawn_worker`], which hands
/// it to a fresh thread, so the editor thread can never wait on a session.
pub(crate) struct SessionClient {
    /// Sending side of the driver's mailbox.
    mailbox: Sender<DriverMessage>,
}

/// Bookkeeping one lookup carries across its retries.
struct LookupAttempts<'a> {
    /// Document snapshot the lookup answers for.
    document: &'a DocumentSyncRequest,
    /// Whether the next request resends the whole text first.
    resend_full_text: bool,
    /// Whether a whole-text sync already happened or was asked for up front.
    forced_full_sync: bool,
    /// What the first reply said about startup and syncing.
    first_reply: Option<ReplyContext>,
    /// What the latest reply said, which paces the next retry.
    latest_reply: Option<ReplyContext>,
    /// Requests sent so far while answers kept coming back empty.
    empty_attempts: usize,
    /// End of the ordinary retry budget.
    deadline: Instant,
}

impl<'a> LookupAttempts<'a> {
    /// Start the bookkeeping for one lookup with `budget` to retry in.
    fn new(document: &'a DocumentSyncRequest, force_full_sync: bool, budget: Duration) -> Self {
        Self {
            document,
            resend_full_text: false,
            forced_full_sync: force_full_sync,
            first_reply: None,
            latest_reply: None,
            // One request has already been sent by the time the first retry is
            // considered, so the budget starts at one.
            empty_attempts: 1,
            deadline: Instant::now() + budget,
        }
    }

    /// Return whether the server had to start for this lookup.
    ///
    /// Returns `true` when the first request arrived before the server finished
    /// starting, and `false` when it found the server running or no reply came yet.
    fn started(&self) -> bool {
        self.first_reply.is_some_and(|context| context.started)
    }

    /// Return whether this lookup sent its document text to the server.
    ///
    /// Returns `true` when the first request carried the document, and `false`
    /// when the server already held that version or no reply came yet.
    fn synced_for_lookup(&self) -> bool {
        self.first_reply.is_some_and(|context| context.synced)
    }
}

impl SessionHandle {
    /// Create one lazily-started language-server session for `workspace`.
    #[cfg(test)]
    pub(crate) fn new(workspace: ProjectWorkspace, server: &'static LspServerDescriptor) -> Self {
        Self::new_with_configuration(
            workspace,
            server,
            Arc::new(LspConfigurationStore::default()),
            SessionEventSenders::detached(),
        )
    }

    /// Create one session with one explicit configuration snapshot.
    ///
    /// The server itself starts with the first command that needs it.
    pub(crate) fn new_with_configuration(
        workspace: ProjectWorkspace,
        server: &'static LspServerDescriptor,
        configuration: Arc<LspConfigurationStore>,
        events: SessionEventSenders,
    ) -> Self {
        let (mailbox, receiver) = mpsc::channel();
        let driver = SessionDriver::new(
            workspace.root_path,
            server,
            configuration,
            events,
            mailbox.clone(),
        );
        Self {
            mailbox,
            driver_thread: Some(thread::spawn(move || {
                append_lsp_trace_line("DEBUG", Path::new("-"), "DRIVER", "driver thread started");
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    driver.run(receiver)
                }));
                let ending = match &outcome {
                    Ok(()) => "driver thread returned".to_string(),
                    Err(payload) => format!(
                        "driver thread PANICKED: {:?}",
                        payload
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
                    ),
                };
                append_lsp_trace_line("DEBUG", Path::new("-"), "DRIVER", &ending);
                if let Err(payload) = outcome {
                    std::panic::resume_unwind(payload);
                }
            })),
        }
    }

    /// Run `work` on a new thread that may wait on the given sessions.
    ///
    /// This is the only way to obtain a [`SessionClient`], which keeps every
    /// blocking session call off the thread that creates the worker.
    pub(crate) fn spawn_worker<'a>(
        sessions: impl IntoIterator<Item = &'a SessionHandle>,
        work: impl FnOnce(Vec<SessionClient>) + Send + 'static,
    ) {
        let clients = sessions
            .into_iter()
            .map(|session| SessionClient {
                mailbox: session.mailbox.clone(),
            })
            .collect::<Vec<_>>();
        thread::spawn(move || work(clients));
    }

    /// Return whether the driver thread is still serving this session.
    ///
    /// Returns `true` while the driver runs, and `false` once it ended, which
    /// only happens before shutdown when it panicked.
    pub(crate) fn is_alive(&self) -> bool {
        self.driver_thread
            .as_ref()
            .is_some_and(|driver_thread| !driver_thread.is_finished())
    }

    /// Make the driver panic, the way a bug inside it would.
    #[cfg(test)]
    pub(crate) fn panic_driver(&self) {
        let _ = self.mailbox.send(DriverMessage::Inspect(Box::new(|_| {
            panic!("driver panic injected by a test")
        })));
    }

    /// Send `didClose` for one tracked document and forget its transport state.
    pub(crate) fn close_document(&self, file_path: &Path) {
        // A driver that already ended has no open document to close.
        let _ = self.mailbox.send(DriverMessage::CloseDocument {
            file_path: file_path.to_path_buf(),
        });
    }

    /// Report one filesystem change Ordex performed to the running server.
    ///
    /// The driver forwards it only when a registered watcher covers the path.
    pub(crate) fn notify_watched_file_change(&self, path: &Path, kind: LspFileChangeKind) {
        // A driver that already ended has no watcher to tell.
        let _ = self.mailbox.send(DriverMessage::WatchedFileChanged {
            path: path.to_path_buf(),
            kind,
        });
    }
}

impl Drop for SessionHandle {
    /// Ensure the server process does not outlive the session object.
    fn drop(&mut self) {
        // A driver that already ended has nothing left to shut down.
        let _ = self.mailbox.send(DriverMessage::Shutdown);
        if let Some(driver_thread) = self.driver_thread.take() {
            #[allow(
                clippy::disallowed_methods,
                reason = "the driver was told to shut down, and every step of that is bounded"
            )]
            let _ = driver_thread.join();
        }
    }
}

impl SessionClient {
    /// Maximum wait for one startup message before the session treats the server
    /// as ready enough to continue with the current request.
    const STARTUP_READY_TIMEOUT: Duration = Duration::from_secs(2);
    /// Delay between navigation retries while startup work is settling.
    const LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(150);
    /// Total retry budget for one navigation lookup that races startup indexing.
    const LOOKUP_RETRY_TIMEOUT: Duration = Duration::from_secs(10);
    /// Total retry budget for one references lookup while workspace indexing settles.
    ///
    /// References can lag behind definition and hover readiness because the
    /// workspace may still be populating cross-file use sites after the origin
    /// symbol is already queryable.
    const REFERENCES_LOOKUP_RETRY_TIMEOUT: Duration = Duration::from_secs(20);
    /// Total retry budget for one pull-diagnostics request cancelled during analysis.
    const DIAGNOSTIC_RETRY_TIMEOUT: Duration = Duration::from_secs(2);
    /// Total budget for one lookup issued while the server reports that its
    /// workspace is still loading.
    ///
    /// Loading a `rustc_private` Cargo workspace takes far longer than the
    /// ordinary lookup budget, so readiness rather than wall-clock decides when
    /// an empty answer becomes final.
    const WORKSPACE_LOADING_TIMEOUT: Duration = Duration::from_secs(120);
    /// Delay between lookup retries while the server workspace is still loading.
    ///
    /// Re-sending at the warm cadence would queue hundreds of requests against a
    /// server that cannot answer any of them yet.
    const WORKSPACE_LOADING_RETRY_DELAY: Duration = Duration::from_secs(1);
    /// Requests one lookup may send while its result keeps coming back empty.
    ///
    /// Reporting a quiescent workspace does not mean every cross-crate query can
    /// be answered yet, so re-asks have to span the whole retry window. They do
    /// it with a doubling delay, which covers that window with a handful of
    /// requests instead of one per retry delay.
    pub(crate) const MAX_EMPTY_LOOKUP_ATTEMPTS: usize = 8;
    /// Longest wait between two re-asks of one lookup that answered empty.
    const MAX_EMPTY_LOOKUP_BACKOFF: Duration = Duration::from_secs(2);
    /// Re-asks signature help may spend learning whether a fresh session is
    /// still loading its workspace.
    ///
    /// Parameter hints must disappear promptly, so this stays far below the
    /// budget the other lookups use.
    const MAX_SIGNATURE_HELP_READINESS_ATTEMPTS: usize = 4;
    /// Pulls one refresh pass may repeat while the server keeps asking for more.
    const MAX_DIAGNOSTIC_REFRESH_PASSES: usize = 4;

    /// Hand one message to the driver without waiting for it.
    fn send(&self, message: DriverMessage) -> Result<(), SessionError> {
        self.mailbox
            .send(message)
            .map_err(|_| transport_closed_error())
    }

    /// Send one command and wait for the driver's answer, for at most `window` when given.
    ///
    /// This is the one place a client waits on the driver. Returns `Ok(None)`
    /// when `window` passed before the answer arrived.
    fn call<T>(
        &self,
        command: impl FnOnce(SyncSender<T>) -> DriverMessage,
        window: Option<Duration>,
    ) -> Result<Option<T>, SessionError> {
        let (reply, answer) = mpsc::sync_channel(1);
        let message = command(reply);
        let description = match &message {
            DriverMessage::Lookup(lookup) => format!("Lookup {:?}", lookup.request),
            DriverMessage::PullDiagnostics { .. } => "PullDiagnostics".to_string(),
            DriverMessage::AwaitReadiness { .. } => "AwaitReadiness".to_string(),
            DriverMessage::TakeDiagnosticRefresh { .. } => "TakeDiagnosticRefresh".to_string(),
            DriverMessage::SyncDocument { .. } => "SyncDocument".to_string(),
            _ => "other".to_string(),
        };
        if let Err(error) = self.send(message) {
            append_lsp_trace_line("DEBUG", Path::new("-"), "CLIENT", &format!("send failed for {description}"));
            return Err(error);
        }
        let result = self.call_wait(answer, window);
        if let Err(error) = &result {
            append_lsp_trace_line("DEBUG", Path::new("-"), "CLIENT", &format!("call failed ({error}) for {description}"));
        }
        result
    }

    /// DEBUG: the wait half of `call`.
    fn call_wait<T>(
        &self,
        answer: Receiver<T>,
        window: Option<Duration>,
    ) -> Result<Option<T>, SessionError> {
        // A dropped reply means the driver ended, which no retry can outlast.
        let Some(window) = window else {
            #[allow(
                clippy::disallowed_methods,
                reason = "a worker's reply wait; the driver answers every command or drops its reply"
            )]
            let received = answer.recv();
            return received.map(Some).map_err(|_| transport_closed_error());
        };
        match answer.recv_timeout(window) {
            Ok(received) => Ok(Some(received)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(transport_closed_error()),
        }
    }

    /// Send one command and wait for the driver's answer for as long as the driver lives.
    fn ask<T>(
        &self,
        command: impl FnOnce(SyncSender<T>) -> DriverMessage,
    ) -> Result<T, SessionError> {
        self.call(command, None)?.ok_or_else(transport_closed_error)
    }

    /// Synchronize one document snapshot into the running language server.
    pub(crate) fn sync_document(&self, request: &DocumentSyncRequest) -> Result<(), SessionError> {
        if self.synchronize_document(request)? {
            self.request_document_diagnostics(&request.file_path, request.version)?;
        }
        Ok(())
    }

    /// Synchronize one document snapshot for a save lifecycle before `didSave`.
    pub(crate) fn sync_document_for_save(
        &self,
        request: &DocumentSyncRequest,
    ) -> Result<(), SessionError> {
        // Save must not wait behind stale completion work.
        self.send(DriverMessage::CancelCompletion)?;
        self.synchronize_document(request).map(|_| ())
    }

    /// Send `didSave` for one already-synchronized document when the server wants it.
    pub(crate) fn save_document(&self, file_path: &Path, text: &Rope) {
        // A driver that already ended has no open document to save.
        let _ = self.send(DriverMessage::SaveDocument {
            file_path: file_path.to_path_buf(),
            text: text.clone(),
        });
    }

    /// Execute one definition lookup against the running language server.
    pub(crate) fn lookup_definition(
        &self,
        request: &NavigationLookupRequest,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_navigation(request, LookupKind::Definition)
    }

    /// Execute one references lookup against the running language server.
    pub(crate) fn lookup_references(
        &self,
        request: &NavigationLookupRequest,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_navigation(request, LookupKind::References)
    }

    /// Execute one hover lookup against the running language server.
    pub(crate) fn lookup_hover(
        &self,
        request: &HoverLookupRequest,
    ) -> Result<Option<String>, SessionError> {
        self.lookup_with_empty_retry(
            &request.document,
            request.force_full_sync,
            Self::LOOKUP_RETRY_TIMEOUT,
            || LookupRequest::Hover(request.position),
            |result, _| Ok(parse_hover_result(result.as_ref())?.map(Cow::into_owned)),
            // Hover can return empty while startup indexing is still catching
            // up, so it keeps the retry window navigation lookups use.
            Option::is_none,
        )
    }

    /// Execute one rename lookup against the running language server.
    pub(crate) fn lookup_rename(
        &self,
        request: &RenameLookupRequest,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        self.lookup_with_empty_retry(
            &request.document,
            request.force_full_sync,
            Self::LOOKUP_RETRY_TIMEOUT,
            || LookupRequest::Rename {
                position: request.position,
                new_name: request.new_name.clone(),
            },
            |result, applied_edit| {
                Ok(parse_workspace_edit_result(result.as_ref())?.or(applied_edit))
            },
            Option::is_none,
        )
    }

    /// Execute one code-action lookup against the running language server.
    pub(crate) fn lookup_code_actions(
        &self,
        request: &CodeActionLookupRequest,
    ) -> Result<Vec<LspCodeAction>, SessionError> {
        self.lookup_with_empty_retry(
            &request.document,
            request.force_full_sync,
            Self::LOOKUP_RETRY_TIMEOUT,
            || LookupRequest::CodeAction {
                range: request.range,
                diagnostics: request.diagnostics.clone(),
            },
            |result, _| Ok(parse_code_action_result(result.as_ref())?),
            Vec::is_empty,
        )
    }

    /// Execute one navigation lookup with the retry policy the server needs.
    fn lookup_navigation(
        &self,
        request: &NavigationLookupRequest,
        kind: LookupKind,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_with_empty_retry(
            &request.document,
            request.force_full_sync,
            Self::navigation_lookup_retry_timeout(kind),
            || match kind {
                LookupKind::Definition => LookupRequest::Definition(request.position),
                LookupKind::References => LookupRequest::References(request.position),
            },
            |result, _| {
                parse_location_result(result.as_ref())?
                    .into_iter()
                    .map(normalize_location)
                    .collect()
            },
            // Some servers can report only the definition itself before
            // cross-file reference indexing settles, so that startup placeholder
            // counts as empty while the retry window is open.
            |targets: &Vec<SessionNavigationTarget>| {
                targets.is_empty()
                    || (kind == LookupKind::References
                        && Self::references_only_origin_result(request, targets))
            },
        )
    }

    /// Run one lookup, asking again while answers stay empty or fail transiently.
    ///
    /// `parse` turns the raw result into an answer, and `is_empty` decides which
    /// answers may still hide a real result while the server warms up.
    fn lookup_with_empty_retry<T>(
        &self,
        document: &DocumentSyncRequest,
        force_full_sync: bool,
        budget: Duration,
        request: impl Fn() -> LookupRequest,
        parse: impl Fn(Option<json::JsonValue>, Option<LspWorkspaceEdit>) -> Result<T, SessionError>,
        is_empty: impl Fn(&T) -> bool,
    ) -> Result<T, SessionError> {
        let mut attempts = LookupAttempts::new(document, force_full_sync, budget);
        loop {
            let reply = self.lookup(&mut attempts, request())?;
            match reply.response {
                Ok(result) => {
                    let answer = parse(result, reply.applied_edit)?;
                    let synced_for_lookup = attempts.synced_for_lookup();
                    if is_empty(&answer)
                        && self.retry_empty_lookup(&mut attempts, synced_for_lookup)?
                    {
                        continue;
                    }
                    return Ok(answer);
                }
                Err(error) if error.is_transient() => {
                    if self.retry_transient_lookup_failure(&mut attempts)? {
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one completion lookup against the running language server.
    pub(crate) fn lookup_completion(
        &self,
        request: &CompletionLookupRequest,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let mut attempts = LookupAttempts::new(
            &request.document,
            request.force_full_sync,
            Self::LOOKUP_RETRY_TIMEOUT,
        );
        let completion = |trigger_text| LookupRequest::Completion {
            position: request.position,
            trigger_text,
        };
        loop {
            let reply = match self.lookup(&mut attempts, completion(request.trigger_text.clone())) {
                // A newer completion request already replaced this one, so report
                // an empty batch and let the freshest request own the popup state.
                Err(SessionError::Superseded(_)) => return Ok(Vec::new()),
                reply => reply?,
            };
            match reply.response {
                Ok(result) => {
                    let items = parse_completion_result(result.as_ref())?;
                    if !items.is_empty() {
                        return Ok(items);
                    }
                    if request.trigger_text.is_some() {
                        // An empty trigger-driven completion is asked once more as
                        // an ordinary invoked request.
                        let invoked = match self.send_lookup(
                            &request.document.file_path,
                            None,
                            false,
                            completion(None),
                        ) {
                            Err(SessionError::Superseded(_)) => return Ok(Vec::new()),
                            invoked => invoked?,
                        };
                        let invoked_items = parse_completion_result(invoked.response?.as_ref())?;
                        if !invoked_items.is_empty() {
                            return Ok(invoked_items);
                        }
                    }
                    // Completion still retries during startup-progress warmup, but
                    // once the session is already warm an empty batch after a fresh
                    // sync should return promptly so later keystrokes can trigger
                    // the next request instead of waiting behind a long retry loop.
                    if self.retry_empty_lookup(&mut attempts, false)? {
                        continue;
                    }
                    return Ok(items);
                }
                Err(error) if error.is_transient() => {
                    if self.retry_transient_lookup_failure(&mut attempts)? {
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one signature-help lookup against the running language server.
    pub(crate) fn lookup_signature_help(
        &self,
        request: &SignatureHelpLookupRequest,
    ) -> Result<Option<LspSignatureHelp>, SessionError> {
        let mut attempts = LookupAttempts::new(
            &request.document,
            request.force_full_sync,
            Self::LOOKUP_RETRY_TIMEOUT,
        );
        loop {
            let signature_help = LookupRequest::SignatureHelp {
                position: request.position,
                trigger_text: request.trigger_text.clone(),
                is_retrigger: request.is_retrigger,
            };
            let reply = match self.lookup(&mut attempts, signature_help) {
                // Mature editors treat parameter hints as a latest-cursor-state
                // feature, so a superseded request simply shows nothing.
                Err(SessionError::Superseded(_)) => return Ok(None),
                reply => reply?,
            };
            match reply.response {
                Ok(result) => {
                    if let Some(help) = parse_signature_help_result(result.as_ref())? {
                        return Ok(Some(help));
                    }
                    // Signature help is driven by the current cursor context, so
                    // on a loaded workspace an empty response dismisses the popup
                    // immediately rather than retrying an older cursor state. A
                    // workspace that is still loading has not answered the
                    // question yet, so that wait happens before giving up.
                    if self.retry_signature_help_while_loading(&mut attempts)? {
                        continue;
                    }
                    return Ok(None);
                }
                Err(error) if error.is_transient() => {
                    if self.retry_transient_lookup_failure(&mut attempts)? {
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Send the next request of one lookup and wait for its answer.
    ///
    /// The first request carries the document, so the driver syncs it and asks
    /// in one step that no other operation can come between.
    fn lookup(
        &self,
        attempts: &mut LookupAttempts<'_>,
        request: LookupRequest,
    ) -> Result<LookupReply, SessionError> {
        let (document, force_full) = match attempts.latest_reply {
            None => (Some(attempts.document.clone()), attempts.forced_full_sync),
            Some(latest) => {
                if !latest.readiness.startup_ready {
                    self.await_startup_ready(Self::STARTUP_READY_TIMEOUT)?;
                }
                let document = attempts.resend_full_text.then(|| attempts.document.clone());
                (document, true)
            }
        };
        attempts.resend_full_text = false;
        let reply =
            self.send_lookup(&attempts.document.file_path, document, force_full, request)?;
        attempts.first_reply.get_or_insert(reply.context);
        attempts.latest_reply = Some(reply.context);
        Ok(reply)
    }

    /// Send one lookup request, syncing `document` first when given, and wait for its answer.
    fn send_lookup(
        &self,
        file_path: &Path,
        document: Option<DocumentSyncRequest>,
        force_full: bool,
        request: LookupRequest,
    ) -> Result<LookupReply, SessionError> {
        self.ask(|reply| {
            DriverMessage::Lookup(LookupCommand {
                file_path: file_path.to_path_buf(),
                document,
                force_full,
                request,
                reply,
            })
        })?
    }

    /// Have the driver bring the server's copy of one document up to `request`.
    ///
    /// The driver starts the server first when none is running.
    fn sync_through_driver(
        &self,
        request: &DocumentSyncRequest,
        mode: SyncMode,
    ) -> Result<SyncOutcome, SessionError> {
        self.ask(|reply| DriverMessage::SyncDocument {
            request: request.clone(),
            mode,
            reply,
        })?
    }

    /// Synchronize one document snapshot without issuing follow-up diagnostic requests.
    ///
    /// Returns `Ok(true)` when the snapshot advanced session state, and `Ok(false)`
    /// when a newer synced version already made this request stale.
    fn synchronize_document(&self, request: &DocumentSyncRequest) -> Result<bool, SessionError> {
        // Debounced background sync favors one coherent full-text snapshot over a
        // long queued edit batch so diagnostics always reflect the live buffer.
        let outcome = self.sync_through_driver(request, SyncMode::FullUnlessCurrent)?;
        if !outcome.sent {
            return Ok(false);
        }
        if outcome.started {
            // Startup progress often arrives immediately after `didOpen`, so the
            // first background sync waits briefly to surface launch-time feedback.
            self.await_startup_ready(Self::STARTUP_READY_TIMEOUT)?;
        }
        Ok(true)
    }

    /// Pull fresh diagnostics for one synchronized document when the server supports it.
    pub(crate) fn request_document_diagnostics(
        &self,
        file_path: &Path,
        version: i32,
    ) -> Result<(), SessionError> {
        let deadline = Instant::now() + Self::DIAGNOSTIC_RETRY_TIMEOUT;
        let mut reuse_result_id = true;
        loop {
            let pulled = self.ask(|reply| DriverMessage::PullDiagnostics {
                file_path: file_path.to_path_buf(),
                version,
                reuse_result_id,
                reply,
            })?;
            match pulled {
                Ok(()) => return Ok(()),
                Err(SessionError::RequestCancelled(_)) if Instant::now() < deadline => {
                    // Servers may cancel a diagnostic pull while analysis is still
                    // converging. Drop any previous result id before retrying so
                    // the follow-up request forces a fresh full report instead of
                    // repeating a cancelled incremental comparison.
                    reuse_result_id = false;
                    self.await_startup_ready(Self::LOOKUP_RETRY_DELAY)?;
                }
                Err(SessionError::RequestCancelled(_)) => {
                    // Once the bounded retry window expires, treat the cancelled pull
                    // as best-effort and let a queued refresh pass repull later.
                    self.send(DriverMessage::QueueDiagnosticRefresh)?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Wait for the server to go quiet before the next request, for at most `timeout`.
    ///
    /// A server that reports a non-quiescent workspace keeps the wait open even
    /// between progress tasks, because none of its answers are final.
    fn await_startup_ready(&self, timeout: Duration) -> Result<(), SessionError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let answer = self.call(
                |reply| DriverMessage::AwaitReadiness { deadline, reply },
                Some(remaining),
            )?;
            let Some(grace_end) = answer
                .filter(Readiness::is_idle)
                .and_then(|readiness| readiness.pending_grace_end())
            else {
                // Either the server is idle with no grace window left, or the
                // window passed while it was still busy.
                return Ok(());
            };
            thread::sleep(
                grace_end
                    .min(deadline)
                    .saturating_duration_since(Instant::now()),
            );
        }
    }

    /// Run the diagnostics refresh pass the driver asked the manager for.
    pub(crate) fn refresh_diagnostics(&self) -> Result<(), SessionError> {
        let result = self.pull_refreshed_diagnostics();
        // The driver asks for the next pass only once this one has ended.
        self.send(DriverMessage::DiagnosticRefreshFinished)?;
        result
    }

    /// Wait before re-asking one lookup that answered empty, when that is still worth it.
    ///
    /// Returns `true` when the caller should ask again, and `false` once the
    /// empty answer is final.
    fn retry_empty_lookup(
        &self,
        attempts: &mut LookupAttempts<'_>,
        synced_for_lookup: bool,
    ) -> Result<bool, SessionError> {
        let Some(latest) = attempts.latest_reply else {
            return Ok(false);
        };
        let readiness = latest.readiness;
        if readiness.retry_budget_exhausted(attempts.deadline) {
            return Ok(false);
        }
        // A workspace that is still loading has not really answered yet, so that
        // wait is paced by the readiness signal rather than the attempt budget.
        if readiness.workspace_loading_since.is_some() {
            self.await_startup_ready(Self::WORKSPACE_LOADING_RETRY_DELAY)?;
            return Ok(true);
        }
        let worth_asking_again = readiness.should_retry_empty_lookup(
            attempts.started(),
            synced_for_lookup,
            latest.startup_ready_before_request,
            attempts.deadline,
        );
        if !worth_asking_again || attempts.empty_attempts >= Self::MAX_EMPTY_LOOKUP_ATTEMPTS {
            return Ok(false);
        }
        // Each re-ask waits twice as long as the previous one, capped, so the
        // retry window a late analysis pass needs is covered without spending
        // every attempt in the first half second.
        let backoff = (Self::LOOKUP_RETRY_DELAY * (1u32 << (attempts.empty_attempts - 1).min(4)))
            .min(Self::MAX_EMPTY_LOOKUP_BACKOFF);
        attempts.empty_attempts += 1;
        // A readiness wait returns as soon as the session looks idle, which is
        // the wrong shape for pacing a retry: the server needs time to finish
        // work it has not announced.
        thread::sleep(backoff);
        Ok(true)
    }

    /// Wait for an unready workspace before one empty signature help is final.
    ///
    /// Returns `true` when the server may still be unable to answer and the
    /// caller should ask again, and `false` once an empty answer is final.
    fn retry_signature_help_while_loading(
        &self,
        attempts: &mut LookupAttempts<'_>,
    ) -> Result<bool, SessionError> {
        let Some(latest) = attempts.latest_reply else {
            return Ok(false);
        };
        if latest.readiness.retry_budget_exhausted(attempts.deadline) {
            return Ok(false);
        }
        let loading = latest.readiness.workspace_loading_since.is_some();
        // A session that just spawned may not have received the server's first
        // status yet, so a few short waits separate a workspace that is still
        // loading from a position that genuinely has no signature.
        let may_still_be_unready = loading
            || (attempts.started()
                && attempts.empty_attempts < Self::MAX_SIGNATURE_HELP_READINESS_ATTEMPTS);
        if !may_still_be_unready {
            return Ok(false);
        }
        attempts.empty_attempts += 1;
        if loading {
            // A loading workspace announces when it is ready, so this wait can
            // end as soon as that happens.
            self.await_startup_ready(Self::WORKSPACE_LOADING_RETRY_DELAY)?;
        } else {
            thread::sleep(Self::LOOKUP_RETRY_DELAY);
        }
        Ok(true)
    }

    /// Return whether one references response only points back to the origin symbol.
    fn references_only_origin_result(
        request: &NavigationLookupRequest,
        targets: &[SessionNavigationTarget],
    ) -> bool {
        // A server can transiently return only the queried definition while
        // workspace references are still loading, which differs from a stable
        // result set that points to another location.
        targets.len() == 1
            && targets[0].path == request.document.file_path
            && targets[0].line == request.position.line
            && targets[0].character == request.position.character
    }

    /// Prepare to re-ask one lookup that was cancelled or invalidated by an edit.
    ///
    /// Returns `true` when the caller should ask again, and `false` once the
    /// retry budget is spent.
    fn retry_transient_lookup_failure(
        &self,
        attempts: &mut LookupAttempts<'_>,
    ) -> Result<bool, SessionError> {
        let Some(latest) = attempts.latest_reply else {
            return Ok(false);
        };
        if latest.readiness.retry_budget_exhausted(attempts.deadline) {
            return Ok(false);
        }
        // Unsaved-buffer lookups can race both document sync and server analysis.
        // One whole-text resend gives the server a coherent snapshot to retry on.
        if !attempts.forced_full_sync {
            attempts.resend_full_text = true;
            attempts.forced_full_sync = true;
        }
        self.await_startup_ready(Self::STARTUP_READY_TIMEOUT)?;
        Ok(true)
    }

    /// Return the retry budget for one navigation lookup kind.
    fn navigation_lookup_retry_timeout(kind: LookupKind) -> Duration {
        match kind {
            LookupKind::Definition => Self::LOOKUP_RETRY_TIMEOUT,
            LookupKind::References => Self::REFERENCES_LOOKUP_RETRY_TIMEOUT,
        }
    }

    /// Pull fresh diagnostics for all open documents while a refresh is pending.
    fn pull_refreshed_diagnostics(&self) -> Result<(), SessionError> {
        // A refresh-triggered pull can itself prompt another refresh request, so
        // the loop is bounded to avoid spinning if a server keeps requeueing
        // refresh work faster than diagnostics can settle.
        for _ in 0..Self::MAX_DIAGNOSTIC_REFRESH_PASSES {
            let Some(documents) =
                self.ask(|reply| DriverMessage::TakeDiagnosticRefresh { reply })?
            else {
                break;
            };
            for document in documents {
                self.request_document_diagnostics(&document.file_path, document.version)?;
            }
        }
        Ok(())
    }
}

/// Convert one protocol location into an editor-facing path and position.
fn normalize_location(location: LspLocation) -> Result<SessionNavigationTarget, SessionError> {
    Ok(SessionNavigationTarget {
        path: file_uri_to_path(&location.uri)?,
        line: location.line,
        character: location.character,
    })
}

/// Return one generic session error for a closed stdio transport.
fn transport_closed_error() -> SessionError {
    SessionError::Server(TRANSPORT_CLOSED_MESSAGE.to_string())
}

/// Convert one response error into the retry-aware session failure Ordex uses.
fn session_error_from_response(error: LspResponseError) -> SessionError {
    match error.code {
        REQUEST_CANCELLED_ERROR_CODE | SERVER_CANCELLED_ERROR_CODE => {
            SessionError::RequestCancelled(error.message)
        }
        CONTENT_MODIFIED_ERROR_CODE => SessionError::ContentModified(error.message),
        _ => SessionError::Server(error.message),
    }
}

/// Stable navigation lookup kinds supported by the session transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookupKind {
    Definition,
    References,
}

impl SessionDriver {
    /// Window a shutting-down server gets to answer the `shutdown` request.
    ///
    /// Exiting must not depend on a server that stopped answering, so the reply
    /// is awaited only long enough for a healthy one to send it.
    const SHUTDOWN_RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);
    /// Window a shutting-down server gets to receive `exit` and leave on its own
    /// before the session kills it.
    const GRACEFUL_EXIT_TIMEOUT: Duration = Duration::from_millis(100);

    /// Build the driver of one session whose server has not started yet.
    fn new(
        workspace_root: PathBuf,
        server: &'static LspServerDescriptor,
        configuration: Arc<LspConfigurationStore>,
        events: SessionEventSenders,
        mailbox: Sender<DriverMessage>,
    ) -> Self {
        Self {
            workspace_root,
            server,
            configuration,
            state: SessionState::new(),
            events,
            mailbox,
            next_request_id: 1,
            lifecycle: Lifecycle::Stopped,
            transport: None,
            next_generation: 1,
            pending_requests: HashMap::new(),
            pending_completion: None,
            pending_signature_help: None,
            readiness_waiters: Vec::new(),
            diagnostic_refresh_claimed: false,
        }
    }

    /// Serve the mailbox until the session asks this driver to shut down.
    fn run(mut self, mailbox: Receiver<DriverMessage>) {
        loop {
            #[allow(
                clippy::disallowed_methods,
                reason = "the driver's idle wait; every other thread only sends to it"
            )]
            let Ok(message) = mailbox.recv() else {
                self.shut_down(&mailbox);
                return;
            };
            match message {
                DriverMessage::Shutdown => {
                    self.shut_down(&mailbox);
                    return;
                }
                DriverMessage::AwaitReadiness { deadline, reply } => {
                    self.readiness_waiters
                        .push(ReadinessWaiter { deadline, reply });
                }
                DriverMessage::QueueDiagnosticRefresh => self.queue_diagnostic_refresh(),
                DriverMessage::TakeDiagnosticRefresh { reply } => {
                    let _ = reply.send(self.take_diagnostic_refresh());
                }
                DriverMessage::DiagnosticRefreshFinished => self.finish_diagnostic_refresh(),
                #[cfg(test)]
                DriverMessage::Inspect(inspect) => inspect(&mut self.state),
                DriverMessage::Inbound {
                    generation,
                    message,
                } => {
                    if self.is_current(generation) {
                        self.process_server_message(message);
                    }
                }
                DriverMessage::Written { generation, length } => {
                    if self.is_current(generation) {
                        self.record_written(length);
                    }
                }
                DriverMessage::TransportClosed { generation } => {
                    if self.is_current(generation) {
                        self.fail_transport(Duration::ZERO, TRANSPORT_CLOSED_MESSAGE);
                    }
                }
                command => self.dispatch(command),
            }
            self.answer_readiness_waiters();
        }
    }

    /// Answer every client waiting for an idle server once it is idle.
    fn answer_readiness_waiters(&mut self) {
        if self.readiness_waiters.is_empty() {
            return;
        }
        // A server that is not running announces no work, so nobody waits on it.
        let running = matches!(
            self.lifecycle,
            Lifecycle::Initializing { .. } | Lifecycle::Running
        );
        let idle = !running || self.state.is_idle();
        let readiness = self.state.readiness();
        let now = Instant::now();
        // A waiter whose window passed is answered too, never dropped: its
        // client may still be listening, and a dropped reply reads as a driver
        // that ended.
        self.readiness_waiters.retain(|waiter| {
            let answer = idle || waiter.deadline <= now;
            if answer {
                let _ = waiter.reply.send(readiness);
            }
            !answer
        });
    }

    /// Remember that open documents need their diagnostics pulled again.
    ///
    /// The manager hears about it once; repeats wait until its pass finishes.
    fn queue_diagnostic_refresh(&mut self) {
        self.state.pending_diagnostic_refresh = true;
        if !self.diagnostic_refresh_claimed {
            self.diagnostic_refresh_claimed = true;
            self.emit(SessionEvent::DiagnosticRefreshRequested);
        }
    }

    /// Hand the refresh pass the open documents, if a refresh is still pending.
    fn take_diagnostic_refresh(&mut self) -> Option<Vec<RefreshDocument>> {
        if !std::mem::take(&mut self.state.pending_diagnostic_refresh) {
            return None;
        }
        // Refresh requests apply to every tracked document, so the pass pulls
        // each at the editor version the server holds right now.
        let documents = self
            .state
            .documents
            .iter()
            .map(|(file_path, document)| RefreshDocument {
                file_path: file_path.clone(),
                version: document.editor_version,
            })
            .collect();
        Some(documents)
    }

    /// End one refresh pass, asking for another when refreshes arrived meanwhile.
    fn finish_diagnostic_refresh(&mut self) {
        self.diagnostic_refresh_claimed = false;
        if self.state.pending_diagnostic_refresh {
            self.queue_diagnostic_refresh();
        }
    }

    /// Allocate the next JSON-RPC request id for this session.
    fn take_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = if request_id == u64::MAX {
            1
        } else {
            request_id + 1
        };
        request_id
    }

    /// Return whether `generation` names the transport that is currently running.
    ///
    /// Returns `true` for traffic of the live transport, and `false` for traffic
    /// a stopped transport's threads sent before they ended.
    fn is_current(&self, generation: u64) -> bool {
        self.transport
            .as_ref()
            .is_some_and(|transport| transport.generation == generation)
    }

    /// Record bytes the writer thread finished handing to the server.
    fn record_written(&mut self, length: usize) {
        if let Some(transport) = self.transport.as_mut() {
            transport.outbound.record_written(length);
        }
    }

    /// Serve one client command now, queue it behind startup, or start a server for it.
    fn dispatch(&mut self, command: DriverMessage) {
        match &mut self.lifecycle {
            Lifecycle::Running => self.execute(command, false),
            Lifecycle::Initializing { queued, .. } => queued.push(command),
            Lifecycle::Stopped | Lifecycle::Failed => {
                if !command.starts_server() {
                    answer_without_server(command);
                    return;
                }
                match self.start() {
                    Ok(()) => self.dispatch(command),
                    Err(error) => fail_command(command, error),
                }
            }
        }
    }

    /// Serve one client command against the running server.
    ///
    /// `started` tells sync commands whether they waited for the server to start.
    fn execute(&mut self, command: DriverMessage, started: bool) {
        // Fire-and-forget failures need no report here: a failed send has
        // already failed the transport and every waiter on it.
        match command {
            DriverMessage::Lookup(command) => self.lookup(command, started),
            DriverMessage::PullDiagnostics {
                file_path,
                version,
                reuse_result_id,
                reply,
            } => self.pull_diagnostics(file_path, version, reuse_result_id, reply),
            DriverMessage::CancelCompletion => {
                if let Some(request_id) = self.pending_completion.take() {
                    self.supersede(request_id, SupersededRequestKind::Completion);
                }
            }
            DriverMessage::SyncDocument {
                request,
                mode,
                reply,
            } => {
                let outcome = self
                    .sync_document(&request, mode)
                    .map(|sent| SyncOutcome { sent, started });
                let _ = reply.send(outcome);
            }
            DriverMessage::CloseDocument { file_path } => {
                let _ = self.close_document(&file_path);
            }
            DriverMessage::SaveDocument { file_path, text } => {
                let _ = self.save_document(&file_path, &text);
            }
            DriverMessage::WatchedFileChanged { path, kind } => {
                let _ = self.watched_file_changed(&path, kind);
            }
            DriverMessage::AwaitReadiness { .. }
            | DriverMessage::QueueDiagnosticRefresh
            | DriverMessage::TakeDiagnosticRefresh { .. }
            | DriverMessage::DiagnosticRefreshFinished
            | DriverMessage::Inbound { .. }
            | DriverMessage::Written { .. }
            | DriverMessage::TransportClosed { .. }
            | DriverMessage::Shutdown => {}
            #[cfg(test)]
            DriverMessage::Inspect(_) => {}
        }
    }

    /// Sync one lookup's document when given and send its request, as one step.
    ///
    /// Nothing else the driver does can come between the two, so the request
    /// always sees the snapshot it was asked about.
    fn lookup(&mut self, command: LookupCommand, started: bool) {
        let LookupCommand {
            file_path,
            document,
            force_full,
            request,
            reply,
        } = command;
        let prepared =
            match self.prepare_lookup(&file_path, document.as_ref(), force_full, &request) {
                Ok(prepared) => prepared,
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            };
        // Completion and parameter hints follow the latest cursor state, so a
        // newer request supersedes the older one instead of waiting behind it.
        let superseded = match request {
            LookupRequest::Completion { .. } => self
                .pending_completion
                .replace(prepared.request_id)
                .map(|request_id| (request_id, SupersededRequestKind::Completion)),
            LookupRequest::SignatureHelp { .. } => self
                .pending_signature_help
                .replace(prepared.request_id)
                .map(|request_id| (request_id, SupersededRequestKind::SignatureHelp)),
            _ => None,
        };
        let rename = matches!(request, LookupRequest::Rename { .. });
        if rename {
            self.state.pending_apply_edit = None;
        }
        let startup_ready_before_request = self.state.startup_ready;
        self.pending_requests.insert(
            prepared.request_id,
            PendingRequest::Lookup {
                reply,
                rename,
                started,
                synced: prepared.synced,
                startup_ready_before_request,
            },
        );
        if let Some((request_id, kind)) = superseded {
            self.supersede(request_id, kind);
        }
        // A failed send has already failed this request along with the transport.
        let _ = self.send_payload(&prepared.payload);
    }

    /// Bring one lookup's document up to date and build its request.
    fn prepare_lookup(
        &mut self,
        file_path: &Path,
        document: Option<&DocumentSyncRequest>,
        force_full: bool,
        request: &LookupRequest,
    ) -> Result<PreparedLookup, SessionError> {
        let synced = match document {
            Some(document) if force_full => {
                self.sync_document(document, SyncMode::Full)?;
                true
            }
            Some(document) => self.sync_document(document, SyncMode::IncrementalUnlessCurrent)?,
            None => false,
        };
        let request_id = self.take_request_id();
        let payload = self.state.lookup_payload(request_id, file_path, request)?;
        Ok(PreparedLookup {
            request_id,
            payload,
            synced,
        })
    }

    /// Answer one in-flight request as superseded and ask the server to drop it.
    fn supersede(&mut self, request_id: u64, kind: SupersededRequestKind) {
        if let Some(PendingRequest::Lookup { reply, .. }) =
            self.pending_requests.remove(&request_id)
        {
            let _ = reply.send(Err(SessionError::Superseded(kind)));
        }
        // Stale work must not hold up newer requests and saves on the server.
        let _ = self.send_payload(&cancel_request_notification(request_id));
    }

    /// Pull diagnostics for one open document when the server supports them.
    fn pull_diagnostics(
        &mut self,
        file_path: PathBuf,
        version: i32,
        reuse_result_id: bool,
        reply: SyncSender<Result<(), SessionError>>,
    ) {
        let request = self
            .state
            .document_diagnostic_provider
            .as_ref()
            .zip(self.state.documents.get(&file_path))
            .map(|(provider, document)| {
                let previous_result_id = reuse_result_id
                    .then(|| document.diagnostic_result_id.clone())
                    .flatten();
                (provider.identifier.clone(), previous_result_id)
            });
        let Some((identifier, previous_result_id)) = request else {
            let _ = reply.send(Ok(()));
            return;
        };
        let request_id = self.take_request_id();
        let payload = document_diagnostic_request(
            request_id,
            &file_path,
            identifier.as_deref(),
            previous_result_id.as_deref(),
        );
        self.pending_requests.insert(
            request_id,
            PendingRequest::Diagnostics {
                file_path,
                version,
                reply,
            },
        );
        // A failed send has already failed this request along with the transport.
        let _ = self.send_payload(&payload);
    }

    /// Hand one server response to whoever waits for it.
    fn answer(
        &mut self,
        request_id: u64,
        pending: PendingRequest,
        response: Result<Option<json::JsonValue>, SessionError>,
    ) {
        // A client that stopped waiting has nobody left to tell.
        match pending {
            PendingRequest::Lookup {
                reply,
                rename,
                started,
                synced,
                startup_ready_before_request,
            } => {
                if self.pending_completion == Some(request_id) {
                    self.pending_completion = None;
                }
                if self.pending_signature_help == Some(request_id) {
                    self.pending_signature_help = None;
                }
                let applied_edit = if rename {
                    self.state.pending_apply_edit.take()
                } else {
                    None
                };
                let _ = reply.send(Ok(LookupReply {
                    response,
                    applied_edit,
                    context: ReplyContext {
                        started,
                        synced,
                        startup_ready_before_request,
                        readiness: self.state.readiness(),
                    },
                }));
            }
            PendingRequest::Diagnostics {
                file_path,
                version,
                reply,
            } => {
                let outcome =
                    response.and_then(|result| self.apply_diagnostics(&file_path, version, result));
                let _ = reply.send(outcome);
            }
            PendingRequest::Shutdown { reply } => {
                let _ = reply.send(());
            }
        }
    }

    /// Apply one pulled diagnostics report and forward its diagnostics.
    fn apply_diagnostics(
        &mut self,
        file_path: &Path,
        version: i32,
        result: Option<json::JsonValue>,
    ) -> Result<(), SessionError> {
        // Pull diagnostics use request/response transport, so forward the
        // resulting snapshot explicitly instead of waiting for a push event.
        let report = parse_document_diagnostic_report(result.as_ref(), file_path, version)?;
        self.state
            .apply_document_diagnostic_report(file_path, &report);
        if let Some(update) = report.diagnostics {
            self.emit(SessionEvent::Diagnostics(update));
        }
        Ok(())
    }

    /// Spawn the server, start its transport threads, and send `initialize`.
    ///
    /// This is the one place a session forgets the previous process's state.
    fn start(&mut self) -> Result<(), SessionError> {
        append_lsp_trace_line("DEBUG", &self.workspace_root, "DRIVER", "start()");
        // Server descriptors can append workspace-scoped startup arguments, so
        // resolve the final command line before constructing the child process.
        let command_args = self
            .server
            .command_args(&self.workspace_root)
            .map_err(|error| SessionError::Spawn(SessionSpawnError::new(self.server, error)))?;
        let child = Command::new(self.server.command_program())
            .args(&command_args)
            .current_dir(&self.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| SessionError::Spawn(SessionSpawnError::new(self.server, error)))?;
        let mut process = ServerProcess(child);
        let stdin = process.0.stdin.take().ok_or(SessionError::MissingStdin)?;
        let stdout = process.0.stdout.take().ok_or(SessionError::MissingStdout)?;
        let generation = self.next_generation;
        self.next_generation += 1;
        let (writer, payloads) = mpsc::channel();
        let threads = TransportThreadContext {
            generation,
            mailbox: self.mailbox.clone(),
            server_name: self.server.display_name,
            workspace_root: self.workspace_root.clone(),
        };
        self.transport = Some(Transport {
            generation,
            process,
            outbound: OutboundChannel::new(writer),
            reader_thread: spawn_reader_thread(stdout, threads.clone()),
            writer_thread: spawn_writer_thread(stdin, payloads, threads),
        });
        // A new child knows nothing about the documents and capabilities the
        // session recorded for the one it replaces.
        self.state.reset();
        let request_id = self.take_request_id();
        self.lifecycle = Lifecycle::Initializing {
            request_id,
            queued: Vec::new(),
        };
        self.send_payload(&initialize_request(
            request_id,
            &self.workspace_root,
            &self.configuration,
            self.server.display_name,
        ))
    }

    /// Apply the `initialize` response and serve the commands queued behind it.
    fn finish_initialize(
        &mut self,
        result: Option<json::JsonValue>,
        error: Option<LspResponseError>,
    ) {
        let capabilities = match error {
            Some(error) => Err(error.message),
            None => self
                .state
                .apply_capabilities(result.as_ref())
                .map_err(|error| error.to_string()),
        };
        if let Err(failure) = capabilities {
            self.fail_transport(Duration::ZERO, &failure);
            return;
        }
        if self.send_payload(&initialized_notification()).is_err() {
            return;
        }
        self.emit(SessionEvent::Capabilities(SessionCapabilities {
            completion: self.state.completion_provider.clone(),
            signature_help: self.state.signature_help_provider.clone(),
        }));
        let Lifecycle::Initializing { queued, .. } =
            std::mem::replace(&mut self.lifecycle, Lifecycle::Running)
        else {
            return;
        };
        for command in queued {
            // A command that broke the transport leaves the rest without a server.
            if matches!(self.lifecycle, Lifecycle::Running) {
                self.execute(command, true);
            } else {
                fail_command(command, transport_closed_error());
            }
        }
    }

    /// Stop a transport that broke, failing every waiter and queued command.
    fn fail_transport(&mut self, grace: Duration, failure: &str) {
        self.stop_transport(grace, failure);
        if let Lifecycle::Initializing { queued, .. } =
            std::mem::replace(&mut self.lifecycle, Lifecycle::Failed)
        {
            for command in queued {
                fail_command(command, SessionError::Server(failure.to_string()));
            }
        }
    }

    /// Frame and queue one payload, abandoning a transport that stopped draining.
    ///
    /// A failure has already failed the transport by the time it is returned.
    fn send_payload(&mut self, payload: &json::JsonValue) -> Result<(), SessionError> {
        append_lsp_trace_line(
            self.server.display_name,
            &self.workspace_root,
            "OUT",
            &payload.dump(),
        );
        let mut framed = Vec::new();
        write_message(&mut framed, payload).expect("framing into memory cannot fail");
        let Some(transport) = self.transport.as_mut() else {
            return Err(transport_closed_error());
        };
        match transport.outbound.push(framed) {
            Ok(()) => Ok(()),
            Err(OutboundError::Closed) => {
                self.fail_transport(Duration::ZERO, TRANSPORT_CLOSED_MESSAGE);
                Err(transport_closed_error())
            }
            Err(OutboundError::Overflow) => {
                // The child is killed rather than asked to exit, because `exit`
                // would go into the same backlog that already overflowed.
                let failure = format!(
                    "language server \"{}\" stopped reading its input and was stopped",
                    self.server.display_name
                );
                self.fail_transport(Duration::ZERO, &failure);
                Err(SessionError::Server(failure))
            }
        }
    }

    /// Send `didOpen` or `didChange` for one snapshot as `mode` asks.
    ///
    /// Returns `Ok(true)` when the snapshot was sent, and `Ok(false)` when the
    /// server already holds this version or a newer one.
    fn sync_document(
        &mut self,
        request: &DocumentSyncRequest,
        mode: SyncMode,
    ) -> Result<bool, SessionError> {
        let language_id = self
            .server
            .lsp_language_id(&request.file_path)
            .ok_or_else(|| {
                SessionError::Server("unsupported LSP language for document".to_string())
            })?;
        let (payload, protocol_version) = {
            let state = &self.state;
            let skip = match mode {
                SyncMode::Full => state.holds_newer_document(&request.file_path, request.version),
                SyncMode::FullUnlessCurrent | SyncMode::IncrementalUnlessCurrent => {
                    state.should_skip_document_sync(&request.file_path, request.version)
                }
            };
            if skip {
                return Ok(false);
            }
            let text = request.text.to_string();
            let protocol_version =
                state.next_document_protocol_version(&request.file_path, request.version);
            let payload = if !state.documents.contains_key(&request.file_path) {
                did_open_notification(&request.file_path, language_id, protocol_version, &text)
            } else if mode == SyncMode::IncrementalUnlessCurrent {
                // Prefer the negotiated sync mode, with a whole-document fallback
                // for stale or empty edit queues.
                state.change_notification(request, protocol_version, &text)
            } else {
                did_change_notification(
                    &request.file_path,
                    protocol_version,
                    &[LspTextChange { range: None, text }],
                )
            };
            (payload, protocol_version)
        };
        self.send_payload(&payload)?;
        self.state
            .record_synced_document(&request.file_path, request.version, protocol_version);
        Ok(true)
    }

    /// Forget one document and send `didClose` when the server tracks open files.
    fn close_document(&mut self, file_path: &Path) -> Result<(), SessionError> {
        let removed = self.state.documents.remove(file_path).is_some();
        if removed && self.state.text_document_sync.open_close {
            self.send_payload(&did_close_notification(file_path))?;
        }
        Ok(())
    }

    /// Send `didSave` for one already-synchronized document when the server wants it.
    fn save_document(&mut self, file_path: &Path, text: &Rope) -> Result<(), SessionError> {
        let Some(save_options) = self.state.text_document_sync.save else {
            return Ok(());
        };
        if !self.state.documents.contains_key(file_path) {
            return Ok(());
        }
        // Convert the rope lazily so save notifications stay cheap for servers
        // that only need the URI and not the full saved contents.
        let text = save_options.include_text.then(|| text.to_string());
        self.send_payload(&did_save_notification(file_path, text.as_deref()))
    }

    /// Report one filesystem change when a registered watcher covers it.
    fn watched_file_changed(
        &mut self,
        path: &Path,
        kind: LspFileChangeKind,
    ) -> Result<(), SessionError> {
        let watched = self
            .state
            .watched_file_registrations
            .values()
            .flatten()
            .any(|watcher| watcher.matches(path, kind));
        if !watched {
            return Ok(());
        }
        let change = LspWatchedFileChange {
            path: path.to_path_buf(),
            kind,
        };
        self.send_payload(&did_change_watched_files_notification(&[change]))
    }

    /// Apply one message the server sent, replying to it when it is a request.
    fn process_server_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Response { id, result, error } => {
                if let Lifecycle::Initializing { request_id, .. } = &self.lifecycle
                    && *request_id == id
                {
                    self.finish_initialize(result, error);
                    return;
                }
                // A superseded request was already answered and needs nothing more.
                let Some(pending) = self.pending_requests.remove(&id) else {
                    return;
                };
                self.state.startup_ready = true;
                let response = match error {
                    Some(error) => Err(session_error_from_response(error)),
                    None => Ok(result),
                };
                self.answer(id, pending, response);
            }
            ServerMessage::Request { id, method, params } => {
                if method == "workspace/diagnostic/refresh" {
                    // The server requests a client-initiated re-pull once fresh
                    // document diagnostics are ready after background analysis.
                    self.queue_diagnostic_refresh();
                }
                self.reply_to_server_request(id, &method, params.as_ref());
            }
            ServerMessage::Notification { method, params } => {
                // Notifications can carry progress updates, so surface them before
                // marking the session as ready for follow-up request work.
                self.handle_notification(&method, params.as_ref());
                self.state.startup_ready = true;
            }
        }
    }

    /// Reply to one server-initiated request with a best-effort success payload.
    fn reply_to_server_request(&mut self, id: u64, method: &str, params: Option<&json::JsonValue>) {
        // A request Ordex cannot decode gets no reply, the same as a request
        // that never arrived.
        match method {
            "workspace/applyEdit" => {
                let Ok(edit) = parse_apply_edit_request(params) else {
                    return;
                };
                self.state.pending_apply_edit = Some(edit);
            }
            "client/registerCapability" => {
                let Ok(registrations) = parse_watched_files_registrations(params) else {
                    return;
                };
                for registration in registrations {
                    self.state
                        .watched_file_registrations
                        .insert(registration.id, registration.watchers);
                }
            }
            "client/unregisterCapability" => {
                let Ok(ids) = parse_watched_files_unregistrations(params) else {
                    return;
                };
                for id in ids {
                    self.state.watched_file_registrations.remove(&id);
                }
            }
            _ => {}
        }
        let result = server_request_result(
            method,
            params,
            &self.configuration,
            self.server.display_name,
        );
        // A failed reply already stopped the transport, which the driver loop checks.
        let _ = self.send_payload(&server_request_response(id, result));
    }

    /// Apply one server notification and forward any event it carries.
    fn handle_notification(&mut self, method: &str, params: Option<&json::JsonValue>) {
        // A notification Ordex cannot decode carries nothing it could act on.
        if let Ok(Some(notification)) = parse_progress_notification(method, params) {
            self.state.apply_progress_notification(&notification);
            self.emit(SessionEvent::Progress(notification));
            return;
        }
        if let Ok(Some(status)) = parse_server_status_notification(method, params) {
            self.state.apply_server_status(&status);
            return;
        }
        if let Ok(Some(diagnostics)) = parse_publish_diagnostics_notification(method, params) {
            let diagnostics = self
                .state
                .normalize_published_diagnostics_version(diagnostics);
            self.emit(SessionEvent::Diagnostics(diagnostics));
        }
    }

    /// Forward one event to the manager, tagged with this session's origin.
    fn emit(&self, event: SessionEvent) {
        self.events.emit(event, &self.workspace_root, self.server);
    }

    /// Shut a running server down within bounded windows, then stop its transport.
    fn shut_down(mut self, mailbox: &Receiver<DriverMessage>) {
        if matches!(self.lifecycle, Lifecycle::Running) {
            // Ask the server to shut down cleanly first so it can flush any in-flight
            // responses and exit on its own before Ordex escalates to termination.
            let request_id = self.take_request_id();
            let (reply, response) = mpsc::sync_channel(1);
            self.pending_requests
                .insert(request_id, PendingRequest::Shutdown { reply });
            if self.send_payload(&shutdown_request(request_id)).is_ok() {
                // A server that stopped answering never sends this reply, so
                // exiting offers it a window rather than waiting on it.
                self.serve_transport_until(mailbox, Self::SHUTDOWN_RESPONSE_TIMEOUT, |_| {
                    !matches!(response.try_recv(), Err(TryRecvError::Empty))
                });
            }
            // Follow the shutdown request with `exit` even when the graceful path
            // fails so the server still gets the standard terminal notification.
            if self.send_payload(&exit_notification()).is_ok() {
                // The writer thread owns the pipe, so `exit` gets a bounded chance
                // to be handed over before the child's handles are taken away.
                self.serve_transport_until(mailbox, Self::GRACEFUL_EXIT_TIMEOUT, |driver| {
                    driver
                        .transport
                        .as_ref()
                        .is_none_or(|transport| transport.outbound.is_drained())
                });
            }
        }
        self.stop_transport(Self::GRACEFUL_EXIT_TIMEOUT, TRANSPORT_CLOSED_MESSAGE);
    }

    /// Keep applying transport traffic until `finished` holds or `window` passes.
    ///
    /// Client commands that arrive meanwhile are failed, because a driver that
    /// is shutting down starts no new work.
    fn serve_transport_until(
        &mut self,
        mailbox: &Receiver<DriverMessage>,
        window: Duration,
        finished: impl Fn(&Self) -> bool,
    ) {
        let deadline = Instant::now() + window;
        while self.transport.is_some() && !finished(self) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Ok(message) = mailbox.recv_timeout(remaining) else {
                return;
            };
            match message {
                DriverMessage::Inbound {
                    generation,
                    message,
                } => {
                    if self.is_current(generation) {
                        self.process_server_message(message);
                    }
                }
                DriverMessage::Written { generation, length } => {
                    if self.is_current(generation) {
                        self.record_written(length);
                    }
                }
                DriverMessage::TransportClosed { generation } => {
                    if self.is_current(generation) {
                        self.fail_transport(Duration::ZERO, TRANSPORT_CLOSED_MESSAGE);
                    }
                }
                DriverMessage::Shutdown => {}
                command => fail_command(command, transport_closed_error()),
            }
        }
    }

    /// Stop the child process and both transport threads, failing every waiter.
    fn stop_transport(&mut self, grace: Duration, failure: &str) {
        append_lsp_trace_line(
            "DEBUG",
            &self.workspace_root,
            "DRIVER",
            &format!(
                "stop_transport failure={failure:?} had_transport={} pending={}\n{}",
                self.transport.is_some(),
                self.pending_requests.len(),
                std::backtrace::Backtrace::force_capture()
            ),
        );
        if let Some(transport) = self.transport.take() {
            transport.stop(grace);
            // Triggers of a server that is gone must not keep firing requests.
            self.emit(SessionEvent::Capabilities(SessionCapabilities::default()));
        }
        self.pending_completion = None;
        self.pending_signature_help = None;
        // A client that stopped waiting has nobody left to tell.
        for (_, pending) in self.pending_requests.drain() {
            let error = SessionError::Server(failure.to_string());
            match pending {
                PendingRequest::Lookup { reply, .. } => {
                    let _ = reply.send(Err(error));
                }
                PendingRequest::Diagnostics { reply, .. } => {
                    let _ = reply.send(Err(error));
                }
                PendingRequest::Shutdown { .. } => {}
            }
        }
    }
}

/// Answer one client command that cannot be served with `error`.
///
/// Commands that expect no answer are dropped, which is all a session without
/// a server can do with them.
fn fail_command(command: DriverMessage, error: SessionError) {
    append_lsp_trace_line("DEBUG", Path::new("-"), "DRIVER", &format!("fail_command {error}"));
    // A client that stopped waiting has nobody left to tell.
    match command {
        DriverMessage::Lookup(command) => {
            let _ = command.reply.send(Err(error));
        }
        DriverMessage::PullDiagnostics { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        DriverMessage::SyncDocument { reply, .. } => {
            let _ = reply.send(Err(error));
        }
        _ => {}
    }
}

/// Answer one command that does not start a server while none is running.
///
/// A server that is not running holds no documents, so there is nothing to
/// pull diagnostics for and nothing to notify.
fn answer_without_server(command: DriverMessage) {
    if let DriverMessage::PullDiagnostics { reply, .. } = command {
        // A client that stopped waiting has nobody left to tell.
        let _ = reply.send(Ok(()));
    }
}

/// What the reader and writer threads need to report to their driver.
#[derive(Clone)]
struct TransportThreadContext {
    /// Generation of the transport the thread belongs to.
    generation: u64,
    /// Sending side of the driver's mailbox.
    mailbox: Sender<DriverMessage>,
    /// Server name used in trace lines.
    server_name: &'static str,
    /// Workspace root used in trace lines.
    workspace_root: PathBuf,
}

impl TransportThreadContext {
    /// Append one trace line for this transport.
    fn trace(&self, direction: &str, body: &str) {
        append_lsp_trace_line(self.server_name, &self.workspace_root, direction, body);
    }
}

/// Start the reader thread that decodes server output into the driver's mailbox.
fn spawn_reader_thread(stdout: ChildStdout, context: TransportThreadContext) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            match read_message(&mut stdout) {
                Ok(message) => {
                    context.trace("IN", &format!("{message:?}"));
                    let inbound = DriverMessage::Inbound {
                        generation: context.generation,
                        message,
                    };
                    // A driver that is gone no longer listens to this transport.
                    if context.mailbox.send(inbound).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    context.trace("IN", &format!("transport closed: {error}"));
                    let _ = context.mailbox.send(DriverMessage::TransportClosed {
                        generation: context.generation,
                    });
                    return;
                }
            }
        }
    })
}

/// Start the writer thread that hands framed payloads to the server's stdin.
///
/// Every outbound message goes through this thread, so a server that stops
/// reading its stdin stalls only here and never the driver or the editor.
fn spawn_writer_thread(
    mut stdin: ChildStdin,
    payloads: Receiver<Vec<u8>>,
    context: TransportThreadContext,
) -> JoinHandle<()> {
    thread::spawn(move || {
        loop {
            #[allow(
                clippy::disallowed_methods,
                reason = "the writer's idle wait; it ends when the driver closes the payload channel"
            )]
            let Ok(framed) = payloads.recv() else {
                return;
            };
            if let Err(error) = stdin.write_all(&framed).and_then(|()| stdin.flush()) {
                context.trace("OUT", &format!("transport closed: {error}"));
                let _ = context.mailbox.send(DriverMessage::TransportClosed {
                    generation: context.generation,
                });
                return;
            }
            let _ = context.mailbox.send(DriverMessage::Written {
                generation: context.generation,
                length: framed.len(),
            });
        }
    })
}

/// Wait briefly for a clean child-process exit after sending shutdown notifications.
///
/// Returns `true` when the child exited on its own within the grace period, and
/// `false` when the caller should escalate to a forced kill.
fn wait_for_graceful_shutdown(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.try_wait().ok().flatten().is_some()
}

/// Append one opt-in LSP trace line when `ORDEX_LSP_TRACE` names a writable file.
fn append_lsp_trace_line(server_name: &str, workspace_root: &Path, direction: &str, body: &str) {
    let Ok(path) = std::env::var("ORDEX_LSP_TRACE") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(
        file,
        "{timestamp_ms} {direction} {server_name} {} {body}",
        workspace_root.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::project::ProjectRootKind;
    use crate::lsp::protocol::path_to_file_uri;
    use crate::lsp::server::RUST_ANALYZER;
    use crate::lsp::test_servers::{
        FakeRustAnalyzerConfig, write_fake_rust_analyzer, write_non_reading_rust_analyzer,
    };
    use std::fs;
    use test_utils::{EnvVarGuard, TempTree, lock_process_environment};

    /// Build one reusable workspace value for session unit tests.
    fn test_workspace() -> ProjectWorkspace {
        ProjectWorkspace {
            root_path: PathBuf::from("/tmp/workspace"),
            kind: ProjectRootKind::CargoWorkspace,
            marker_path: PathBuf::from("/tmp/workspace/Cargo.toml"),
        }
    }

    /// Return one repository fixture path for session tests that use the LSP fixtures.
    fn fixture_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    /// Build one real temporary Cargo workspace for fake-server session tests.
    fn temp_workspace() -> TempTree {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "Cargo.toml",
            "[package]\nname = \"fake_session_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write main.rs");
        tree
    }

    /// Wait until the driver has applied server traffic that `ready` accepts.
    ///
    /// Returns `true` when the awaited state arrived inside the timeout, and
    /// `false` when the wait gave up before it did.
    fn drain_until(
        session: &SessionHandle,
        ready: impl Fn(&SessionState) -> bool + Clone + Send + 'static,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let ready = ready.clone();
            if inspect(session, move |state| ready(state)) {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Run `work` against the state the driver of `session` owns and return its result.
    fn inspect<T: Send + 'static>(
        session: &SessionHandle,
        work: impl FnOnce(&mut SessionState) -> T + Send + 'static,
    ) -> T {
        let (sender, receiver) = mpsc::channel();
        session
            .mailbox
            .send(DriverMessage::Inspect(Box::new(move |state| {
                let _ = sender.send(work(state));
            })))
            .expect("the driver should be running");
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the driver should run the inspection")
    }

    /// Return one test workspace descriptor rooted at `tree`.
    fn tree_workspace(tree: &TempTree) -> ProjectWorkspace {
        ProjectWorkspace {
            root_path: tree.path().to_path_buf(),
            kind: ProjectRootKind::CargoWorkspace,
            marker_path: tree.path().join("Cargo.toml"),
        }
    }

    /// Run `work` against `session` on a worker thread and return what it produced.
    fn on_worker<T: Send + 'static>(
        session: &SessionHandle,
        work: impl FnOnce(&SessionClient) -> T + Send + 'static,
    ) -> T {
        let (sender, receiver) = mpsc::channel();
        SessionHandle::spawn_worker([session], move |clients| {
            let _ = sender.send(work(&clients[0]));
        });
        receiver
            .recv_timeout(Duration::from_secs(120))
            .expect("the worker should finish")
    }

    /// Build one driver for `workspace` whose mailbox nobody serves.
    fn idle_driver(workspace: ProjectWorkspace) -> SessionDriver {
        let (mailbox, _receiver) = mpsc::channel();
        SessionDriver::new(
            workspace.root_path,
            &RUST_ANALYZER,
            Arc::new(LspConfigurationStore::default()),
            SessionEventSenders::detached(),
            mailbox,
        )
    }

    /// Confirm that request ids advance monotonically across one session.
    #[test]
    fn test_take_request_id_advances_monotonically() {
        let mut driver = idle_driver(test_workspace());

        assert_eq!(driver.take_request_id(), 1);
        assert_eq!(driver.take_request_id(), 2);
    }

    /// A readiness wait that runs out must end as a timeout, never as a dead driver.
    ///
    /// The client bounds the wait with the same deadline the driver prunes
    /// waiters by, so whichever notices the deadline first, the client must get
    /// an answer or a timeout. A driver that drops an expired waiter's reply
    /// makes a client that is still listening report a closed transport.
    #[test]
    fn test_expired_readiness_waiter_is_answered_rather_than_dropped() {
        let mut driver = idle_driver(test_workspace());
        driver.lifecycle = Lifecycle::Running;
        driver
            .state
            .active_progress_tokens
            .insert("indexing".to_string());
        let (reply, answer) = mpsc::sync_channel(1);
        driver.readiness_waiters.push(ReadinessWaiter {
            deadline: Instant::now(),
            reply,
        });

        driver.answer_readiness_waiters();

        let readiness = answer
            .try_recv()
            .expect("an expired waiter must be answered, not dropped");
        assert!(
            !readiness.is_idle(),
            "the answer must not claim a busy server is idle"
        );
        assert!(driver.readiness_waiters.is_empty());
    }

    /// Confirm stale sync work cannot move the tracked document version backward.
    #[test]
    fn test_should_skip_document_sync_for_stale_version() {
        let mut state = SessionState::new();
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        state.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 4,
                protocol_version: 7,
                diagnostic_result_id: None,
            },
        );

        assert!(state.should_skip_document_sync(&file_path, 3));
        assert!(state.should_skip_document_sync(&file_path, 4));
        assert!(!state.should_skip_document_sync(&file_path, 5));
        assert!(!state.should_skip_document_sync(Path::new("/tmp/workspace/src/lib.rs"), 1));
    }

    /// Confirm a forced whole-text resend never moves a document back to an older snapshot.
    ///
    /// A lookup that retries late still carries the snapshot it started from; a
    /// driver that resent it after newer edits would make the server answer
    /// other requests against text the editor no longer shows.
    #[test]
    fn test_holds_newer_document_rejects_only_older_snapshots() {
        let mut state = SessionState::new();
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        state.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 4,
                protocol_version: 7,
                diagnostic_result_id: None,
            },
        );

        assert!(state.holds_newer_document(&file_path, 3));
        assert!(!state.holds_newer_document(&file_path, 4));
        assert!(!state.holds_newer_document(&file_path, 5));
        assert!(!state.holds_newer_document(Path::new("/tmp/workspace/src/lib.rs"), 1));
    }

    /// Confirm missing executables report one PATH-specific startup message.
    #[test]
    fn test_session_spawn_error_formats_missing_path_message() {
        let error = SessionSpawnError::new(
            &RUST_ANALYZER,
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing rust-analyzer"),
        );

        assert!(error.is_missing_from_path());
        assert_eq!(
            error.to_string(),
            "language server \"rust-analyzer\" is not in PATH; install \"rust-analyzer\" or add it to PATH"
        );
        assert_eq!(
            SessionError::Spawn(error).to_string(),
            "language server \"rust-analyzer\" is not in PATH; install \"rust-analyzer\" or add it to PATH"
        );
    }

    /// Confirm repeated syncs for one editor version still advance the LSP version.
    #[test]
    fn test_next_document_protocol_version_advances_for_repeat_syncs() {
        let mut state = SessionState::new();
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        state.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 4,
                protocol_version: 7,
                diagnostic_result_id: None,
            },
        );

        assert_eq!(state.next_document_protocol_version(&file_path, 4), 8);
        assert_eq!(state.next_document_protocol_version(&file_path, 5), 8);
        assert_eq!(
            state.next_document_protocol_version(Path::new("/tmp/workspace/src/lib.rs"), 0),
            1
        );
    }

    /// Confirm empty navigation retries stay enabled for startup and fresh sync races.
    #[test]
    fn test_should_retry_empty_lookup_only_during_startup_window() {
        let idle = SessionState::new().readiness();
        let deadline = Instant::now() + Duration::from_secs(1);

        assert!(idle.should_retry_empty_lookup(true, false, true, deadline));
        assert!(idle.should_retry_empty_lookup(false, false, false, deadline));
        assert!(idle.should_retry_empty_lookup(false, true, true, deadline));
        assert!(!idle.should_retry_empty_lookup(false, false, true, deadline));

        let busy = Readiness {
            active_progress: true,
            ..idle
        };
        assert!(busy.should_retry_empty_lookup(false, false, true, deadline));

        let recently_busy = Readiness {
            recent_progress_deadline: Some(Instant::now() + Duration::from_millis(250)),
            ..idle
        };
        assert!(recently_busy.should_retry_empty_lookup(false, false, true, deadline));
    }

    /// Confirm warm completion requests do not retry empty results just because sync ran.
    #[test]
    fn test_lookup_completion_does_not_retry_empty_results_after_warm_sync() {
        let lock = lock_process_environment();
        // Prepend the fake server to PATH so the session exercises a deterministic
        // completion exchange instead of depending on a real rust-analyzer binary.
        let tree = temp_workspace();
        let log_path = tree.path().join("completion.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::empty_completion(&log_path, &[".", ":"]),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);

        // Warm the session first so the lookup exercises the post-startup path
        // where a fresh sync alone must not keep empty completion retries alive.
        let warm_request = DocumentSyncRequest {
            file_path: file_path.clone(),
            version: 1,
            text: Rope::from_str("fn main() {}\n"),
            changes: Vec::new(),
        };
        on_worker(&session, move |client| client.sync_document(&warm_request))
            .expect("warm session sync");
        inspect(&session, |state| state.startup_ready = true);
        let started_at = Instant::now();
        let request = CompletionLookupRequest {
            document: DocumentSyncRequest {
                file_path: file_path.clone(),
                version: 2,
                text: Rope::from_str("fn main() {\n    std::mem::s\n}\n"),
                changes: Vec::new(),
            },
            force_full_sync: true,
            position: LspPosition {
                line: 1,
                character: 15,
            },
            trigger_text: None,
        };
        let items = on_worker(&session, move |client| client.lookup_completion(&request))
            .expect("lookup completion");

        assert!(items.is_empty());
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "empty warm completion should return promptly"
        );
        assert_eq!(
            fs::read_to_string(&log_path)
                .expect("read completion log")
                .lines()
                .count(),
            1,
            "warm empty completion should not retry after a fresh sync"
        );
    }

    /// Confirm references retries recognize the self-only placeholder result.
    #[test]
    fn test_references_only_origin_result_matches_definition_position() {
        let request = NavigationLookupRequest {
            document: DocumentSyncRequest {
                file_path: PathBuf::from("/tmp/workspace/src/main.rs"),
                version: 3,
                text: Rope::from_str("fn main() {}\n"),
                changes: Vec::new(),
            },
            force_full_sync: false,
            position: LspPosition {
                line: 4,
                character: 13,
            },
        };
        let origin_target = SessionNavigationTarget {
            path: PathBuf::from("/tmp/workspace/src/main.rs"),
            line: 4,
            character: 13,
        };
        let shifted_target = SessionNavigationTarget {
            path: PathBuf::from("/tmp/workspace/src/main.rs"),
            line: 7,
            character: 13,
        };

        assert!(SessionClient::references_only_origin_result(
            &request,
            &[origin_target]
        ));
        assert!(!SessionClient::references_only_origin_result(
            &request,
            &[shifted_target]
        ));
    }

    /// Confirm response errors map to retry-aware session variants by LSP code.
    #[test]
    fn test_session_error_from_response_uses_lsp_error_codes() {
        assert!(matches!(
            session_error_from_response(LspResponseError {
                code: REQUEST_CANCELLED_ERROR_CODE,
                message: "request cancelled".to_string(),
            }),
            SessionError::RequestCancelled(_)
        ));
        assert!(matches!(
            session_error_from_response(LspResponseError {
                code: CONTENT_MODIFIED_ERROR_CODE,
                message: "content modified".to_string(),
            }),
            SessionError::ContentModified(_)
        ));
        assert!(matches!(
            session_error_from_response(LspResponseError {
                code: -32001,
                message: "server error".to_string(),
            }),
            SessionError::Server(_)
        ));
    }

    /// Confirm pushed diagnostics use the editor version for the latest synced snapshot.
    #[test]
    fn test_normalize_published_diagnostics_version_maps_latest_protocol_version() {
        let mut state = SessionState::new();
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        state.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 5,
                protocol_version: 2,
                diagnostic_result_id: None,
            },
        );
        let diagnostics = LspFileDiagnostics::new(file_path, Some(2), Vec::new());

        // Push diagnostics report the session's protocol version, so map the
        // latest tracked one back to the editor version that gates visibility.
        let normalized = state.normalize_published_diagnostics_version(diagnostics);

        assert_eq!(normalized.version, Some(5));
    }

    /// Confirm pull-diagnostics reports retain the latest reusable result id.
    #[test]
    fn test_apply_document_diagnostic_report_updates_result_id() {
        let mut state = SessionState::new();
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        state.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 5,
                protocol_version: 2,
                diagnostic_result_id: None,
            },
        );
        let report = DocumentDiagnosticReport {
            result_id: Some("diag-1".to_string()),
            diagnostics: None,
        };

        // Pull diagnostics reuse the server's opaque result id across subsequent
        // requests, so store the latest one on the tracked document state.
        state.apply_document_diagnostic_report(&file_path, &report);

        assert_eq!(
            state
                .documents
                .get(&file_path)
                .and_then(|document| document.diagnostic_result_id.as_deref()),
            Some("diag-1")
        );
    }

    /// Confirm empty lookup results are re-asked across the whole retry window.
    ///
    /// A retry budget only buys time if the waits between re-asks actually
    /// elapse. Pacing them with a wait that returns as soon as the session looks
    /// idle spends the entire budget within milliseconds, so the lookup reports
    /// nothing found while the server is still a second away from answering.
    #[test]
    fn test_empty_lookup_retries_are_spaced_across_the_retry_window() {
        let lock = lock_process_environment();
        // Stay empty for longer than the doubling re-asks reach in their first
        // few attempts, but well inside the navigation retry window.
        let empty_for = Duration::from_millis(4_000);
        let tree = temp_workspace();
        let log_path = tree.path().join("definition.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::empty_definitions_before_answering(
                &log_path,
                empty_for.as_millis() as u64,
            ),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);
        let request = NavigationLookupRequest {
            document: DocumentSyncRequest {
                file_path: file_path.clone(),
                version: 1,
                text: Rope::from_str("fn main() {}\n"),
                changes: Vec::new(),
            },
            force_full_sync: false,
            position: LspPosition {
                line: 0,
                character: 3,
            },
        };

        let targets = on_worker(&session, move |client| client.lookup_definition(&request))
            .expect("definition lookup");

        assert_eq!(
            targets,
            vec![SessionNavigationTarget {
                path: file_path,
                line: 0,
                character: 3,
            }],
            "the lookup must still be asking when the answer becomes available"
        );
        // Spanning that window has to cost a handful of requests, not one per
        // retry delay.
        let requests = fs::read_to_string(&log_path)
            .expect("read definition log")
            .lines()
            .filter(|line| line.contains("definition"))
            .count();
        assert!(
            requests <= SessionClient::MAX_EMPTY_LOOKUP_ATTEMPTS,
            "{requests} definition requests should stay within the empty-retry budget of {}",
            SessionClient::MAX_EMPTY_LOOKUP_ATTEMPTS
        );
    }

    /// Confirm signature help waits for a slow workspace instead of showing nothing.
    #[test]
    fn test_lookup_signature_help_waits_for_a_slow_workspace_to_finish_loading() {
        let lock = lock_process_environment();
        // The server cannot answer until its workspace loads, which takes longer
        // than the warm lookup budget allows.
        let loading =
            Duration::from_millis(SessionClient::LOOKUP_RETRY_TIMEOUT.as_millis() as u64 + 2_000);
        let tree = temp_workspace();
        let log_path = tree.path().join("signature.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::cold_workspace_definition(
                &log_path,
                loading.as_millis() as u64,
            ),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);
        let request = SignatureHelpLookupRequest {
            document: DocumentSyncRequest {
                file_path,
                version: 1,
                text: Rope::from_str("fn main() {\n    helper(\n}\n"),
                changes: Vec::new(),
            },
            force_full_sync: false,
            position: LspPosition {
                line: 1,
                character: 11,
            },
            trigger_text: Some("(".to_string()),
            is_retrigger: false,
        };

        let help = on_worker(&session, move |client| {
            client.lookup_signature_help(&request)
        })
        .expect("signature help lookup");

        let help = help.expect("a loaded workspace should answer the signature request");
        assert_eq!(help.signatures.len(), 1);
        assert_eq!(help.signatures[0].label, "fn helper(value: i32)");
    }

    /// Confirm a loaded workspace still dismisses empty signature help at once.
    #[test]
    fn test_lookup_signature_help_does_not_retry_on_a_loaded_workspace() {
        let lock = lock_process_environment();
        // With no workspace-loading report the server is treated as loaded, so an
        // empty answer must dismiss the popup rather than keep asking.
        let tree = temp_workspace();
        let log_path = tree.path().join("signature.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::signature_help_without_answers(&log_path),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);

        // Warm the session first so the lookup runs against a server whose
        // startup status has already been observed.
        let warm_request = DocumentSyncRequest {
            file_path: file_path.clone(),
            version: 1,
            text: Rope::from_str("fn main() {}\n"),
            changes: Vec::new(),
        };
        on_worker(&session, move |client| client.sync_document(&warm_request))
            .expect("warm session sync");

        let started_at = Instant::now();
        let request = SignatureHelpLookupRequest {
            document: DocumentSyncRequest {
                file_path,
                version: 2,
                text: Rope::from_str("fn main() {\n    helper(\n}\n"),
                changes: Vec::new(),
            },
            force_full_sync: false,
            position: LspPosition {
                line: 1,
                character: 11,
            },
            trigger_text: Some("(".to_string()),
            is_retrigger: false,
        };
        let help = on_worker(&session, move |client| {
            client.lookup_signature_help(&request)
        })
        .expect("signature help lookup");

        assert!(help.is_none());
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "an empty answer from a loaded workspace should return promptly"
        );
        assert_eq!(
            fs::read_to_string(&log_path)
                .expect("read signature log")
                .lines()
                .filter(|line| line.contains("signature-help"))
                .count(),
            1,
            "a loaded workspace should be asked once before the popup is dismissed"
        );
    }

    /// Confirm background server work does not turn one empty result into a flood.
    #[test]
    fn test_lookup_completion_does_not_flood_while_background_work_runs() {
        let lock = lock_process_environment();
        // The server answers empty while reporting a task that never finishes,
        // which is what rust-analyzer looks like during ordinary typing.
        let tree = temp_workspace();
        let log_path = tree.path().join("completion.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::empty_completion_during_background_work(
                &log_path,
                &[".", ":"],
            ),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);

        let warm_request = DocumentSyncRequest {
            file_path: file_path.clone(),
            version: 1,
            text: Rope::from_str("fn main() {}\n"),
            changes: Vec::new(),
        };
        on_worker(&session, move |client| client.sync_document(&warm_request))
            .expect("warm session sync");
        assert!(
            drain_until(&session, |state| !state.active_progress_tokens.is_empty()),
            "the fake server should be reporting one in-flight task"
        );

        let request = CompletionLookupRequest {
            document: DocumentSyncRequest {
                file_path,
                version: 2,
                text: Rope::from_str("fn main() {\n    std::mem::s\n}\n"),
                changes: Vec::new(),
            },
            force_full_sync: true,
            position: LspPosition {
                line: 1,
                character: 15,
            },
            trigger_text: None,
        };
        let items = on_worker(&session, move |client| client.lookup_completion(&request))
            .expect("lookup completion");

        assert!(items.is_empty());
        let requests = fs::read_to_string(&log_path)
            .expect("read completion log")
            .lines()
            .filter(|line| line.contains("completion"))
            .count();
        assert!(
            requests <= SessionClient::MAX_EMPTY_LOOKUP_ATTEMPTS,
            "{requests} completion requests should stay within the empty-retry budget of {}",
            SessionClient::MAX_EMPTY_LOOKUP_ATTEMPTS
        );
    }

    /// Confirm saved files reach servers that registered a matching watcher.
    #[test]
    fn test_notify_watched_file_change_reports_registered_globs() {
        let lock = lock_process_environment();
        let tree = temp_workspace();
        let log_path = tree.path().join("watched.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::watched_files(&log_path, "**/*.rs"),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);

        let open_request = DocumentSyncRequest {
            file_path: file_path.clone(),
            version: 1,
            text: Rope::from_str("fn main() {}\n"),
            changes: Vec::new(),
        };
        on_worker(&session, move |client| client.sync_document(&open_request))
            .expect("start session");

        assert!(
            drain_until(&session, |state| !state
                .watched_file_registrations
                .is_empty()),
            "the fake server should have registered one watcher"
        );

        // The unwatched path goes first, so by the time the watched one reaches
        // the server, a wrongly forwarded `notes.txt` would already be logged.
        session
            .notify_watched_file_change(&tree.path().join("notes.txt"), LspFileChangeKind::Changed);
        session.notify_watched_file_change(&file_path, LspFileChangeKind::Created);

        let logged = wait_for_log_line(&log_path, "watched ");
        assert_eq!(
            logged,
            format!("watched 1 {}", path_to_file_uri(&file_path)),
            "the server should see one created-file event for the saved path"
        );
        let watched_lines = fs::read_to_string(&log_path)
            .expect("read watched log")
            .lines()
            .filter(|line| line.contains("watched "))
            .count();
        assert_eq!(
            watched_lines, 1,
            "a path outside every registered glob must stay unreported"
        );
    }

    /// Concurrent lookups and syncs must only ever move a document forward on the server.
    ///
    /// Workers race to send snapshots of one document at increasing editor
    /// versions, some through background syncs and some carried by lookups that
    /// force a whole-text resend. In whatever order they reach the driver, the
    /// server must see strictly increasing protocol versions and never a snapshot
    /// older than one it already holds; a driver that resends a late lookup's
    /// stale snapshot fails the second property.
    #[test]
    fn test_concurrent_lookups_and_syncs_keep_document_versions_monotonic() {
        let lock = lock_process_environment();
        let tree = temp_workspace();
        let log_path = tree.path().join("documents.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::document_syncs(&log_path, &["."]),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);
        let workers = 8;
        let rounds = 12;
        let next_version = Arc::new(std::sync::atomic::AtomicI32::new(1));
        let (finished_sender, finished) = mpsc::channel();

        for worker in 0..workers {
            let next_version = Arc::clone(&next_version);
            let finished_sender = finished_sender.clone();
            let file_path = file_path.clone();
            SessionHandle::spawn_worker([&session], move |clients| {
                let client = &clients[0];
                for round in 0..rounds {
                    let version = next_version.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let document = DocumentSyncRequest {
                        file_path: file_path.clone(),
                        version,
                        text: Rope::from_str(&format!("// version {version}\nfn main() {{}}\n")),
                        changes: Vec::new(),
                    };
                    // Only what reached the server matters, not what each call returned.
                    let _ = if (worker + round) % 2 == 0 {
                        client.sync_document(&document)
                    } else {
                        client
                            .lookup_completion(&CompletionLookupRequest {
                                document,
                                force_full_sync: true,
                                position: LspPosition {
                                    line: 1,
                                    character: 0,
                                },
                                trigger_text: None,
                            })
                            .map(|_| ())
                    };
                }
                let _ = finished_sender.send(());
            });
        }
        for _ in 0..workers {
            finished
                .recv_timeout(Duration::from_secs(60))
                .expect("every worker should finish");
        }

        let log = fs::read_to_string(&log_path).expect("read document log");
        let syncs = log
            .lines()
            .filter(|line| line.contains("did-open ") || line.contains("did-change "))
            .map(|line| {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                let protocol_version = fields[2].parse::<i32>().expect("protocol version");
                let editor_version = fields[5].parse::<i32>().expect("editor version");
                (protocol_version, editor_version)
            })
            .collect::<Vec<_>>();
        assert!(
            syncs.len() > workers,
            "the workers should have synced the document"
        );
        for pair in syncs.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "protocol versions must strictly increase: {syncs:?}"
            );
            assert!(
                pair[0].1 <= pair[1].1,
                "the server must never receive an older snapshot: {syncs:?}"
            );
        }
    }

    /// Wait until `log_path` contains one line holding `needle` and return it.
    fn wait_for_log_line(log_path: &Path, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(contents) = fs::read_to_string(log_path)
                && let Some(line) = contents.lines().find(|line| line.contains(needle))
            {
                let start = line.find(needle).expect("needle position");
                return line[start..].to_string();
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("log should contain a line with {needle:?}");
    }

    /// Confirm definitions still resolve when the workspace loads slowly.
    #[test]
    fn test_lookup_definition_waits_for_a_slow_workspace_to_finish_loading() {
        let lock = lock_process_environment();
        // The server stays unqueryable for longer than the warm lookup budget, so
        // a client that stops retrying on wall-clock alone reports no definition
        // even though the symbol resolves once the workspace finishes loading.
        let loading =
            Duration::from_millis(SessionClient::LOOKUP_RETRY_TIMEOUT.as_millis() as u64 + 2_000);
        let tree = temp_workspace();
        let log_path = tree.path().join("definition.log");
        write_fake_rust_analyzer(
            &tree,
            &FakeRustAnalyzerConfig::cold_workspace_definition(
                &log_path,
                loading.as_millis() as u64,
            ),
        );
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);
        let request = NavigationLookupRequest {
            document: DocumentSyncRequest {
                file_path: file_path.clone(),
                version: 1,
                text: Rope::from_str("fn main() {}\n"),
                changes: Vec::new(),
            },
            force_full_sync: false,
            position: LspPosition {
                line: 0,
                character: 3,
            },
        };

        let targets = on_worker(&session, move |client| client.lookup_definition(&request))
            .expect("definition lookup");

        assert_eq!(
            targets,
            vec![SessionNavigationTarget {
                path: file_path,
                line: 0,
                character: 3,
            }]
        );
        // Retries must also back off while the workspace loads, because polling at
        // the warm cadence floods the loading server with requests it cannot answer.
        let definition_requests = fs::read_to_string(&log_path)
            .expect("read definition log")
            .lines()
            .count();
        let warm_cadence_requests =
            (loading.as_millis() / SessionClient::LOOKUP_RETRY_DELAY.as_millis()) as usize;
        assert!(
            definition_requests < warm_cadence_requests / 4,
            "{definition_requests} requests should stay well below the warm cadence of \
             {warm_cadence_requests}"
        );
    }

    /// Confirm stale skipped syncs do not issue a second diagnostics pull.
    #[test]
    fn test_sync_document_skips_diagnostics_for_stale_version() {
        let lock = lock_process_environment();
        // Prepend the fake server to PATH so the session exercises a deterministic
        // initialize + diagnostic exchange instead of depending on a real LSP binary.
        let tree = temp_workspace();
        let log_path = tree.path().join("diagnostics.log");
        write_fake_rust_analyzer(&tree, &FakeRustAnalyzerConfig::diagnostics_only(&log_path));
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let file_path = tree.path().join("src/main.rs");
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);
        let fresh_request = DocumentSyncRequest {
            file_path: file_path.clone(),
            version: 5,
            text: Rope::from_str("fn main() {\n    let value = 1;\n}\n"),
            changes: Vec::new(),
        };
        let stale_request = DocumentSyncRequest {
            file_path,
            version: 4,
            text: Rope::from_str("fn main() {}\n"),
            changes: Vec::new(),
        };

        on_worker(&session, move |client| client.sync_document(&fresh_request))
            .expect("sync fresh request");
        on_worker(&session, move |client| client.sync_document(&stale_request))
            .expect("sync stale request");

        // The fresh sync needs one pull, while the stale sync should stop after the
        // skip check instead of issuing a second redundant diagnostic request.
        let diagnostic_requests = fs::read_to_string(log_path)
            .expect("read diagnostics log")
            .lines()
            .count();

        assert_eq!(diagnostic_requests, 1);
    }

    /// Confirm rename waits for the workspace graph to include cross-file references.
    #[test]
    fn test_lookup_rename_returns_workspace_edit() {
        let _lock = lock_process_environment();
        let workspace_root = fixture_path("tests/fixtures/lsp/workspace_one");
        let lib_rs = workspace_root.join("src/lib.rs");
        let lib_text = std::fs::read_to_string(&lib_rs).expect("read lib.rs");
        let rename_line = lib_text
            .lines()
            .position(|line| line.contains("helper_value() -> i32"))
            .expect("helper_value definition line");
        let rename_character = lib_text
            .lines()
            .nth(rename_line)
            .and_then(|line| line.find("helper_value"))
            .expect("helper_value definition column");
        let session = SessionHandle::new(
            ProjectWorkspace {
                root_path: workspace_root.clone(),
                kind: ProjectRootKind::CargoWorkspace,
                marker_path: workspace_root.join("Cargo.toml"),
            },
            &RUST_ANALYZER,
        );
        let document = DocumentSyncRequest {
            file_path: lib_rs,
            version: 0,
            text: Rope::from_str(&lib_text),
            changes: Vec::new(),
        };
        let position = LspPosition {
            line: rename_line,
            character: rename_character,
        };
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            let request = NavigationLookupRequest {
                document: document.clone(),
                force_full_sync: true,
                position,
            };
            let references = on_worker(&session, move |client| client.lookup_references(&request))
                .expect("references request should succeed");
            // Rename becomes stable once the server reports the cross-file use site,
            // so keep probing briefly until the workspace graph settles.
            if references
                .iter()
                .any(|entry| entry.path.ends_with("src/main.rs"))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "references should include main.rs before rename: {:?}",
                references
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        let request = RenameLookupRequest {
            document,
            force_full_sync: true,
            position,
            new_name: "helper_total".to_string(),
        };

        let edit = on_worker(&session, move |client| client.lookup_rename(&request))
            .expect("rename request should succeed")
            .expect("rename should return edits");
        assert!(
            edit.document_edits
                .iter()
                .any(|entry| entry.path.ends_with("src/lib.rs"))
        );
        assert!(
            edit.document_edits
                .iter()
                .any(|entry| entry.path.ends_with("src/main.rs"))
        );
    }

    #[test]
    fn test_outbound_channel_preserves_send_order() {
        let (writer, payloads) = mpsc::channel();
        let mut outbound = OutboundChannel::new(writer);
        for version in 1..=3 {
            outbound
                .push(format!("file{version}.rs").into_bytes())
                .expect("queue payload");
        }

        for version in 1..=3 {
            let framed = payloads.try_recv().expect("queued payload");
            let framed = String::from_utf8(framed).expect("framed payload is utf-8");
            assert_eq!(
                framed,
                format!("file{version}.rs"),
                "payloads must reach the writer in send order"
            );
        }
    }

    /// A backlog nobody confirms must stop growing at the ceiling.
    ///
    /// Pushing never waits for the writer, so an outbound channel that kept
    /// accepting payloads for a server that stopped reading would grow without
    /// bound; confirmed writes must free that room again.
    #[test]
    fn test_outbound_channel_overflows_at_the_backlog_ceiling() {
        let (writer, _payloads) = mpsc::channel();
        let mut outbound = OutboundChannel::new(writer);
        let payload = vec![b'x'; 64 * 1024];
        let attempts = 2 * OutboundChannel::MAX_OUTBOUND_QUEUE_BYTES / payload.len();

        let overflow = (0..attempts).find_map(|_| outbound.push(payload.clone()).err());

        assert_eq!(overflow, Some(OutboundError::Overflow));
        outbound.record_written(OutboundChannel::MAX_OUTBOUND_QUEUE_BYTES);
        assert!(outbound.is_drained());
        outbound
            .push(payload)
            .expect("confirmed writes must make room again");
    }

    /// A server that never drains its stdin must cost the session, not the editor.
    ///
    /// The server answers the handshake and then stops reading, so a lookup waits
    /// on a reply that never comes while payloads pile up behind the full pipe.
    /// Once the backlog reaches the ceiling the session must fail that waiter
    /// with an error naming the wedged server, rather than grow without bound or
    /// leave the lookup waiting.
    #[test]
    fn test_backlog_overflow_fails_waiting_requests() {
        let lock = lock_process_environment();
        let tree = temp_workspace();
        write_non_reading_rust_analyzer(&tree);
        let _path_guard = EnvVarGuard::prepend_to_path(&lock, tree.path());
        let session = SessionHandle::new(tree_workspace(&tree), &RUST_ANALYZER);
        let file_path = tree.path().join("src/main.rs");
        let request = NavigationLookupRequest {
            document: DocumentSyncRequest {
                file_path: file_path.clone(),
                version: 1,
                text: Rope::from_str("fn main() {}\n"),
                changes: Vec::new(),
            },
            force_full_sync: false,
            position: LspPosition {
                line: 0,
                character: 3,
            },
        };
        let (lookup_sender, lookup_result) = mpsc::channel();
        SessionHandle::spawn_worker([&session], move |clients| {
            let _ = lookup_sender.send(clients[0].lookup_definition(&request));
        });
        assert!(
            drain_until(&session, |state| state.documents.len() == 1),
            "the lookup should have opened its document"
        );

        // Whole-text resends of a large document build the backlog quickly.
        let bulky_text = Rope::from_str(&"x".repeat(64 * 1024));
        let attempts = 2 * OutboundChannel::MAX_OUTBOUND_QUEUE_BYTES / (64 * 1024);
        for version in 0..attempts {
            let (reply, _answer) = mpsc::sync_channel(1);
            // The driver may already have given up on the transport.
            let _ = session.mailbox.send(DriverMessage::SyncDocument {
                request: DocumentSyncRequest {
                    file_path: file_path.clone(),
                    version: version as i32 + 2,
                    text: bulky_text.clone(),
                    changes: Vec::new(),
                },
                mode: SyncMode::Full,
                reply,
            });
        }

        let overflow_error = lookup_result
            .recv_timeout(Duration::from_secs(30))
            .expect("the lookup should end")
            .expect_err("the overflow must fail the waiting lookup");
        assert!(
            overflow_error.to_string().contains("stopped reading"),
            "overflow must name the wedged server: {overflow_error}"
        );
    }
}
