//! Shared language-server process sessions reused across requests in one workspace.

use super::diagnostics::{LspDiagnostic, LspFileDiagnostics};
use super::project::ProjectWorkspace;
use super::protocol::{
    CompletionProvider, DocumentDiagnosticProvider, DocumentDiagnosticReport, LspCodeAction,
    LspCompletionItem, LspLocation, LspPosition, LspProgressNotification, LspRange,
    LspResponseError, LspSignatureHelp, LspTextChange, LspWorkspaceEdit, ProtocolError,
    ServerMessage, SignatureHelpProvider, TextDocumentSyncKind, TextDocumentSyncOptions,
    cancel_request_notification, code_action_request, completion_request, definition_request,
    did_change_notification, did_close_notification, did_open_notification, did_save_notification,
    document_diagnostic_request, exit_notification, file_uri_to_path, hover_request,
    initialize_request, initialized_notification, parse_apply_edit_request,
    parse_code_action_result, parse_completion_provider, parse_completion_result,
    parse_document_diagnostic_provider, parse_document_diagnostic_report, parse_hover_result,
    parse_location_result, parse_progress_notification, parse_publish_diagnostics_notification,
    parse_signature_help_provider, parse_signature_help_result, parse_text_document_sync_options,
    parse_workspace_edit_result, read_message, references_request, rename_request,
    server_request_response, server_request_result, shutdown_request, signature_help_request,
    write_message,
};
use super::server::LspServerDescriptor;
use ropey::Rope;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One event forwarded from the session transport into higher-level orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionEvent {
    Progress(LspProgressNotification),
    Diagnostics(LspFileDiagnostics),
}

/// Type-erased event callback used by `LspSession` so transport code can
/// forward notifications without depending on manager channels or editor state.
type EventSink<'a> = dyn FnMut(SessionEvent) + 'a;

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

/// Summary of the document-sync work completed before one lookup request.
struct LookupPreparation {
    /// Whether starting the server was part of this lookup preparation.
    started: bool,
    /// Whether this lookup resent document text because it forced or needed sync.
    synced_for_lookup: bool,
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
                write!(f, "{}", LspSession::COMPLETION_SUPERSEDED_MESSAGE)
            }
            Self::Superseded(SupersededRequestKind::SignatureHelp) => {
                write!(f, "{}", LspSession::SIGNATURE_HELP_SUPERSEDED_MESSAGE)
            }
            Self::RequestCancelled(error) | Self::ContentModified(error) | Self::Server(error) => {
                write!(f, "{error}")
            }
        }
    }
}

impl std::error::Error for SessionError {}

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
    /// In-flight completion request ids paired with per-request cancellation flags.
    pending_completion_requests: HashMap<u64, Arc<AtomicBool>>,
    /// In-flight signature-help request ids paired with per-request cancellation flags.
    pending_signature_help_requests: HashMap<u64, Arc<AtomicBool>>,
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
}

/// Child-process handles that stay owned by the session while the reader thread runs.
struct SessionRuntime {
    /// Child process for the running language server, if startup succeeded.
    child: Option<Child>,
    /// Writable stdin handle used for outgoing JSON-RPC traffic.
    stdin: Option<ChildStdin>,
    /// Dedicated reader thread that owns stdout and routes inbound messages.
    reader_thread: Option<JoinHandle<()>>,
}

/// Shared transport bookkeeping used by the reader thread and concurrent callers.
struct SessionTransportShared {
    /// Queue of inbound notifications and server requests awaiting session handling.
    pending_messages: Mutex<VecDeque<ServerMessage>>,
    /// Condition variable that wakes waiters when the pending-message queue changes.
    pending_message_signal: Condvar,
    /// Per-request response channels keyed by JSON-RPC request id.
    response_waiters: Mutex<HashMap<u64, SyncSender<TransportResponse>>>,
    /// Whether the transport has observed EOF or another terminal read failure.
    closed: AtomicBool,
}

/// Local cancellation metadata for a request superseded by a newer lookup.
struct RequestCancellation<'a> {
    flag: &'a Arc<AtomicBool>,
    superseded_kind: SupersededRequestKind,
}

/// Response delivered back to the specific request waiter that owns one request id.
enum TransportResponse {
    Result(Option<json::JsonValue>),
    Error(LspResponseError),
}

/// One reusable language-server process keyed by workspace root.
pub(crate) struct LspSession {
    /// Workspace root and project metadata that scope this session instance.
    workspace: ProjectWorkspace,
    /// Built-in server descriptor that owns command resolution and language ids.
    server: &'static LspServerDescriptor,
    /// Child-process handles and the reader-thread join handle for this session.
    runtime: Mutex<SessionRuntime>,
    /// Mutable session state shared across requests, notifications, and retries.
    state: Mutex<SessionState>,
    /// Shared transport queues and waiter routing used by all concurrent callers.
    transport_shared: Arc<SessionTransportShared>,
    /// Monotonic JSON-RPC request id allocator for this session.
    next_request_id: AtomicU64,
    /// Startup gate that ensures only one caller can spawn and initialize the
    /// session at a time while other callers wait for the completed handshake.
    startup_lock: Mutex<()>,
    /// Guard that prevents nested refresh drains from recursively re-entering the
    /// document-diagnostics pull loop while another refresh pass is already active.
    diagnostic_refresh_active: AtomicBool,
}

impl LspSession {
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
    /// Synthetic cancellation message used when a newer completion supersedes an older one.
    const COMPLETION_SUPERSEDED_MESSAGE: &'static str = "completion request superseded";
    /// Synthetic cancellation message used when a newer signature-help request supersedes an older one.
    const SIGNATURE_HELP_SUPERSEDED_MESSAGE: &'static str = "signature-help request superseded";
    /// LSP error code for one cancelled client request.
    const REQUEST_CANCELLED_ERROR_CODE: i32 = -32800;
    /// LSP error code for one request invalidated by newer document contents.
    const CONTENT_MODIFIED_ERROR_CODE: i32 = -32801;
    /// LSP error code for one server-side cancellation.
    const SERVER_CANCELLED_ERROR_CODE: i32 = -32802;
    /// Extra retry window after the latest progress event so lookups can bridge
    /// the short gap between the visible progress ending and definitions resolving.
    const RECENT_PROGRESS_RETRY_WINDOW: Duration = Duration::from_millis(500);

    /// Create one lazily-started language-server session for `workspace`.
    pub(crate) fn new(workspace: ProjectWorkspace, server: &'static LspServerDescriptor) -> Self {
        Self {
            workspace,
            server,
            runtime: Mutex::new(SessionRuntime {
                child: None,
                stdin: None,
                reader_thread: None,
            }),
            state: Mutex::new(SessionState {
                documents: HashMap::new(),
                active_progress_tokens: HashSet::new(),
                recent_progress_deadline: None,
                pending_apply_edit: None,
                pending_completion_requests: HashMap::new(),
                pending_signature_help_requests: HashMap::new(),
                text_document_sync: TextDocumentSyncOptions::default(),
                document_diagnostic_provider: None,
                completion_provider: None,
                signature_help_provider: None,
                pending_diagnostic_refresh: false,
                startup_ready: false,
            }),
            transport_shared: Arc::new(SessionTransportShared {
                pending_messages: Mutex::new(VecDeque::new()),
                pending_message_signal: Condvar::new(),
                response_waiters: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
            }),
            next_request_id: AtomicU64::new(1),
            startup_lock: Mutex::new(()),
            diagnostic_refresh_active: AtomicBool::new(false),
        }
    }

    /// Return the built-in server descriptor that owns this session.
    pub(crate) fn server_descriptor(&self) -> &'static LspServerDescriptor {
        self.server
    }

    /// Return whether the child process for this session is already running.
    fn is_running(&self) -> bool {
        self.runtime
            .lock()
            .expect("lock session runtime")
            .child
            .is_some()
    }

    /// Return one generic session error for a closed stdio transport.
    fn transport_closed_error() -> SessionError {
        SessionError::Server("language server transport closed".to_string())
    }

    /// Start the dedicated reader thread that routes responses by request id.
    fn spawn_reader_thread(&self, stdout: ChildStdout) -> JoinHandle<()> {
        let shared = Arc::clone(&self.transport_shared);
        let server_name = self.server.display_name.to_string();
        let workspace_root = self.workspace.root_path.clone();
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                match read_message(&mut stdout) {
                    Ok(message) => {
                        append_lsp_trace_line(
                            &server_name,
                            &workspace_root,
                            "IN",
                            &format!("{message:?}"),
                        );
                        match message {
                            ServerMessage::Response { id, result, error } => {
                                let waiter = shared
                                    .response_waiters
                                    .lock()
                                    .expect("lock transport waiters")
                                    .remove(&id);
                                if let Some(waiter) = waiter {
                                    let _ = waiter.send(match error {
                                        Some(error) => TransportResponse::Error(error),
                                        None => TransportResponse::Result(result),
                                    });
                                }
                            }
                            other => {
                                shared
                                    .pending_messages
                                    .lock()
                                    .expect("lock pending messages")
                                    .push_back(other);
                                shared.pending_message_signal.notify_all();
                            }
                        }
                    }
                    Err(error) => {
                        append_lsp_trace_line(
                            &server_name,
                            &workspace_root,
                            "IN",
                            &format!("transport closed: {error}"),
                        );
                        shared.closed.store(true, Ordering::SeqCst);
                        shared
                            .response_waiters
                            .lock()
                            .expect("lock transport waiters")
                            .clear();
                        shared.pending_message_signal.notify_all();
                        break;
                    }
                }
            }
        })
    }

    /// Register one waiting channel for `request_id` and return its receiver.
    fn register_response_waiter(&self, request_id: u64) -> Receiver<TransportResponse> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.transport_shared
            .response_waiters
            .lock()
            .expect("lock transport waiters")
            .insert(request_id, sender);
        receiver
    }

    /// Send one request payload after registering the waiter for its response id.
    fn write_request_payload(
        &self,
        request_id: u64,
        payload: &json::JsonValue,
    ) -> Result<Receiver<TransportResponse>, SessionError> {
        if self.transport_shared.closed.load(Ordering::SeqCst) {
            return Err(Self::transport_closed_error());
        }
        let receiver = self.register_response_waiter(request_id);
        if let Err(error) = self.write_payload(payload) {
            self.transport_shared
                .response_waiters
                .lock()
                .expect("lock transport waiters")
                .remove(&request_id);
            return Err(error);
        }
        Ok(receiver)
    }

    /// Wait until the reader thread queues any non-response message or the timeout expires.
    fn wait_for_pending_messages(&self, timeout: Duration) -> bool {
        let queue = self
            .transport_shared
            .pending_messages
            .lock()
            .expect("lock pending messages");
        if !queue.is_empty() {
            return true;
        }
        let (queue, _) = self
            .transport_shared
            .pending_message_signal
            .wait_timeout(queue, timeout)
            .expect("wait for pending messages");
        !queue.is_empty()
    }

    /// Drain queued notifications and server requests into the session state.
    fn drain_pending_messages(
        &self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        let messages = {
            let mut queue = self
                .transport_shared
                .pending_messages
                .lock()
                .expect("lock pending messages");
            queue.drain(..).collect::<Vec<_>>()
        };
        let mut saw_progress = false;
        for message in messages {
            saw_progress |= self
                .process_server_message(message, progress_sink)?
                .saw_progress;
        }
        Ok(saw_progress)
    }

    /// Mark one request id as cancelled locally and notify the server transport.
    fn cancel_request(&self, request_id: u64) {
        let _ = self.write_payload(&cancel_request_notification(request_id));
    }

    /// Cancel all pending completion requests because a newer operation superseded them.
    fn cancel_pending_completion_requests(&self) {
        let pending = {
            let mut state = self.state.lock().expect("lock session state");
            let pending = state
                .pending_completion_requests
                .drain()
                .collect::<Vec<_>>();
            for (_, cancelled) in &pending {
                cancelled.store(true, Ordering::SeqCst);
            }
            pending
        };
        // Save and newer completion requests should not wait behind stale completion work.
        for (request_id, _) in pending {
            self.cancel_request(request_id);
        }
    }

    /// Synchronize one document snapshot into the running language server.
    pub(crate) fn sync_document(
        &self,
        request: &DocumentSyncRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        if self.synchronize_document(request, progress_sink)? {
            self.request_document_diagnostics(&request.file_path, request.version, progress_sink)?;
        }
        Ok(())
    }

    /// Synchronize one document snapshot for a save lifecycle before `didSave`.
    pub(crate) fn sync_document_for_save(
        &self,
        request: &DocumentSyncRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        self.cancel_pending_completion_requests();
        self.synchronize_document(request, progress_sink)
            .map(|_| ())
    }

    /// Execute one definition lookup against the running language server.
    pub(crate) fn lookup_definition(
        &self,
        request: &NavigationLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_navigation(request, LookupKind::Definition, progress_sink)
    }

    /// Execute one references lookup against the running language server.
    pub(crate) fn lookup_references(
        &self,
        request: &NavigationLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_navigation(request, LookupKind::References, progress_sink)
    }

    /// Execute one hover lookup against the running language server.
    pub(crate) fn lookup_hover(
        &self,
        request: &HoverLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        self.lookup_hover_request(request, progress_sink)
    }

    /// Execute one signature-help lookup against the running language server.
    pub(crate) fn lookup_signature_help(
        &self,
        request: &SignatureHelpLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspSignatureHelp>, SessionError> {
        self.lookup_signature_help_request(request, progress_sink)
    }

    /// Execute one completion lookup against the running language server.
    pub(crate) fn lookup_completion(
        &self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        self.lookup_completion_request(request, progress_sink)
    }

    /// Return the longest server-advertised trigger text that matches `recent_text`.
    pub(crate) fn matching_completion_trigger(&self, recent_text: &str) -> Option<String> {
        self.state
            .lock()
            .expect("lock session state")
            .completion_provider
            .as_ref()
            .and_then(|provider| provider.matching_trigger_text(recent_text))
            .map(str::to_string)
    }

    /// Return the longest server-advertised signature-help trigger matching `recent_text`.
    pub(crate) fn matching_signature_help_trigger(&self, recent_text: &str) -> Option<String> {
        self.state
            .lock()
            .expect("lock session state")
            .signature_help_provider
            .as_ref()
            .and_then(|provider| provider.matching_trigger_text(recent_text))
            .map(str::to_string)
    }

    /// Return the maximum trigger-text length advertised by the running session.
    pub(crate) fn max_completion_trigger_chars(&self) -> usize {
        self.state
            .lock()
            .expect("lock session state")
            .completion_provider
            .as_ref()
            .map_or(0, CompletionProvider::max_trigger_text_chars)
    }

    /// Return the maximum signature-help trigger-text length advertised by the session.
    pub(crate) fn max_signature_help_trigger_chars(&self) -> usize {
        self.state
            .lock()
            .expect("lock session state")
            .signature_help_provider
            .as_ref()
            .map_or(0, SignatureHelpProvider::max_trigger_text_chars)
    }

    /// Execute one rename lookup against the running language server.
    pub(crate) fn lookup_rename(
        &self,
        request: &RenameLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        self.lookup_rename_request(request, progress_sink)
    }

    /// Execute one code-action lookup against the running language server.
    pub(crate) fn lookup_code_actions(
        &self,
        request: &CodeActionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCodeAction>, SessionError> {
        self.lookup_code_action_request(request, progress_sink)
    }

    /// Shut down the child process if it was started.
    pub(crate) fn shutdown(&self) {
        if !self.is_running() {
            return;
        }
        // Ask the server to shut down cleanly first so it can flush any in-flight
        // responses and exit on its own before Ordex escalates to termination.
        let request_id = self.take_request_id();
        // Shutdown still reuses the ordinary response-reading path, and that path
        // can observe late progress notifications while the session is draining.
        // A no-op sink preserves the shared logic without reopening UI updates.
        let mut ignore_events = |_| {};
        if let Ok(response_rx) =
            self.write_request_payload(request_id, &shutdown_request(request_id))
        {
            let _ = self.wait_for_response(request_id, response_rx, &mut ignore_events, None);
        }
        // Follow the shutdown request with `exit` even when the graceful path
        // fails so the server still gets the standard terminal notification.
        let _ = self.write_payload(&exit_notification());

        let mut runtime = self.runtime.lock().expect("lock session runtime");
        // Drop the child process and stdio handles first so the reader thread can
        // observe transport closure before the session tears down shared state.
        if let Some(mut child) = runtime.child.take()
            && !wait_for_graceful_shutdown(&mut child, Duration::from_millis(100))
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        runtime.stdin = None;
        // Join the reader thread so no background transport work outlives the
        // runtime handles or keeps appending messages into stale queues.
        if let Some(reader_thread) = runtime.reader_thread.take() {
            let _ = reader_thread.join();
        }
        // Wake blocked request waiters and clear queued transport data so a
        // shutting-down session cannot leave callers stuck on old messages.
        self.transport_shared.closed.store(true, Ordering::SeqCst);
        self.transport_shared
            .response_waiters
            .lock()
            .expect("lock transport waiters")
            .clear();
        self.transport_shared.pending_message_signal.notify_all();
        self.transport_shared
            .pending_messages
            .lock()
            .expect("lock pending messages")
            .clear();
        // Reset all tracked session state so any later restart begins from a
        // clean initialize handshake instead of stale document bookkeeping.
        let mut state = self.state.lock().expect("lock session state");
        state.documents.clear();
        state.active_progress_tokens.clear();
        state.recent_progress_deadline = None;
        state.pending_apply_edit = None;
        state.pending_completion_requests.clear();
        state.pending_signature_help_requests.clear();
        state.completion_provider = None;
        state.signature_help_provider = None;
        state.document_diagnostic_provider = None;
        state.text_document_sync = TextDocumentSyncOptions::default();
        state.pending_diagnostic_refresh = false;
        state.startup_ready = false;
    }

    /// Start the language server and complete the initialize handshake when needed.
    ///
    /// Returns `Ok(true)` when this call spawned a fresh child process, and
    /// `Ok(false)` when an existing child was already running.
    fn ensure_started(&self, progress_sink: &mut EventSink<'_>) -> Result<bool, SessionError> {
        let _startup_guard = self.startup_lock.lock().expect("lock startup gate");
        if self.is_running() {
            return Ok(false);
        }
        // Server descriptors can append workspace-scoped startup arguments, so
        // resolve the final command line before constructing the child process.
        let command_args = self
            .server
            .command_args(&self.workspace.root_path)
            .map_err(|error| SessionError::Spawn(SessionSpawnError::new(self.server, error)))?;
        let mut command = Command::new(self.server.command_program());
        command
            .args(&command_args)
            .current_dir(&self.workspace.root_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| SessionError::Spawn(SessionSpawnError::new(self.server, error)))?;
        let stdin = child.stdin.take().ok_or(SessionError::MissingStdin)?;
        let stdout = child.stdout.take().ok_or(SessionError::MissingStdout)?;
        self.transport_shared.closed.store(false, Ordering::SeqCst);
        let reader_thread = self.spawn_reader_thread(stdout);
        {
            let mut runtime = self.runtime.lock().expect("lock session runtime");
            runtime.stdin = Some(stdin);
            runtime.child = Some(child);
            runtime.reader_thread = Some(reader_thread);
        }

        let request_id = self.take_request_id();
        let response_rx = self.write_request_payload(
            request_id,
            &initialize_request(request_id, &self.workspace.root_path, self.server.id),
        )?;
        let result = self.wait_for_response(request_id, response_rx, progress_sink, None)?;
        {
            let mut state = self.state.lock().expect("lock session state");
            state.text_document_sync = parse_text_document_sync_options(result.as_ref())
                .map_err(SessionError::Protocol)?;
            state.document_diagnostic_provider =
                parse_document_diagnostic_provider(result.as_ref())
                    .map_err(SessionError::Protocol)?;
            state.completion_provider =
                parse_completion_provider(result.as_ref()).map_err(SessionError::Protocol)?;
            state.signature_help_provider =
                parse_signature_help_provider(result.as_ref()).map_err(SessionError::Protocol)?;
            state.startup_ready = false;
        }
        self.write_payload(&initialized_notification())?;
        Ok(true)
    }

    /// Send `didOpen` or `didChange` so the server sees the current buffer snapshot.
    fn apply_document_sync(&self, request: &DocumentSyncRequest) -> Result<(), SessionError> {
        if self.should_skip_document_sync(&request.file_path, request.version) {
            return Ok(());
        }
        let text = request.text.to_string();
        let language_id = self
            .server
            .lsp_language_id(&request.file_path)
            .ok_or_else(|| {
                SessionError::Server("unsupported LSP language for document".to_string())
            })?;
        let mut state = self.state.lock().expect("lock session state");
        let protocol_version = Self::next_document_protocol_version_from_state(
            &state,
            &request.file_path,
            request.version,
        );
        let payload = if state.documents.contains_key(&request.file_path) {
            // Once the document is open, prefer the negotiated sync mode but
            // keep a whole-document fallback for stale or empty edit queues.
            Self::change_notification_for_state(&state, request, protocol_version, &text)
        } else {
            did_open_notification(&request.file_path, language_id, protocol_version, &text)
        };
        self.write_payload(&payload)?;
        let diagnostic_result_id = state
            .documents
            .get(&request.file_path)
            .and_then(|document| document.diagnostic_result_id.clone());
        state.documents.insert(
            request.file_path.clone(),
            SessionDocumentState {
                editor_version: request.version,
                protocol_version,
                diagnostic_result_id,
            },
        );
        Ok(())
    }

    /// Synchronize one document snapshot without issuing follow-up diagnostic requests.
    ///
    /// Returns `Ok(true)` when the snapshot advanced session state, and `Ok(false)`
    /// when a newer synced version already made this request stale.
    fn synchronize_document(
        &self,
        request: &DocumentSyncRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        let started = self.ensure_started(progress_sink)?;
        if self.should_skip_document_sync(&request.file_path, request.version) {
            return Ok(false);
        }
        // Debounced background sync favors one coherent full-text snapshot over a
        // long queued edit batch so diagnostics always reflect the live buffer.
        self.force_full_document_sync(request)?;
        if started {
            // Startup progress often arrives immediately after `didOpen`, so the
            // first background sync waits briefly to surface launch-time feedback.
            self.await_startup_ready(Self::STARTUP_READY_TIMEOUT, progress_sink)?;
        }
        Ok(true)
    }

    /// Send one full-text sync even when the tracked version already matches.
    fn force_full_document_sync(&self, request: &DocumentSyncRequest) -> Result<(), SessionError> {
        let text = request.text.to_string();
        let language_id = self
            .server
            .lsp_language_id(&request.file_path)
            .ok_or_else(|| {
                SessionError::Server("unsupported LSP language for document".to_string())
            })?;
        let mut state = self.state.lock().expect("lock session state");
        let protocol_version = Self::next_document_protocol_version_from_state(
            &state,
            &request.file_path,
            request.version,
        );
        let payload = if state.documents.contains_key(&request.file_path) {
            did_change_notification(
                &request.file_path,
                protocol_version,
                &[LspTextChange { range: None, text }],
            )
        } else {
            did_open_notification(&request.file_path, language_id, protocol_version, &text)
        };
        self.write_payload(&payload)?;
        let diagnostic_result_id = state
            .documents
            .get(&request.file_path)
            .and_then(|document| document.diagnostic_result_id.clone());
        state.documents.insert(
            request.file_path.clone(),
            SessionDocumentState {
                editor_version: request.version,
                protocol_version,
                diagnostic_result_id,
            },
        );
        Ok(())
    }

    /// Return whether one queued sync request can no longer advance session state.
    ///
    /// Returns `true` when the tracked document already reached `request_version`
    /// or a newer version, and `false` when applying the request would move the
    /// session forward.
    fn should_skip_document_sync(&self, file_path: &Path, request_version: i32) -> bool {
        self.state
            .lock()
            .expect("lock session state")
            .documents
            .get(file_path)
            .is_some_and(|previous| previous.editor_version >= request_version)
    }

    /// Build one `didChange` payload using incremental sync when available.
    fn change_notification_for_state(
        state: &SessionState,
        request: &DocumentSyncRequest,
        protocol_version: i32,
        text: &str,
    ) -> json::JsonValue {
        let changes = if state.text_document_sync.change == TextDocumentSyncKind::Incremental
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

    /// Send `didSave` for one already-synchronized document when the server wants it.
    pub(crate) fn save_document(&self, file_path: &Path, text: &Rope) -> Result<(), SessionError> {
        let state = self.state.lock().expect("lock session state");
        let Some(save_options) = state.text_document_sync.save else {
            return Ok(());
        };
        if !self.is_running() || !state.documents.contains_key(file_path) {
            return Ok(());
        }
        // Convert the rope lazily so save notifications stay cheap for servers
        // that only need the URI and not the full saved contents.
        let text = save_options.include_text.then(|| text.to_string());
        drop(state);
        self.write_payload(&did_save_notification(file_path, text.as_deref()))
    }

    /// Pull fresh diagnostics for one synchronized document when the server supports it.
    pub(crate) fn request_document_diagnostics(
        &self,
        file_path: &Path,
        version: i32,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        let state = self.state.lock().expect("lock session state");
        let Some(provider) = state.document_diagnostic_provider.as_ref() else {
            return Ok(());
        };
        let identifier = provider.identifier.clone();
        let mut previous_result_id = state
            .documents
            .get(file_path)
            .and_then(|state| state.diagnostic_result_id.clone());
        if !self.is_running() || !state.documents.contains_key(file_path) {
            return Ok(());
        }
        drop(state);
        let deadline = Instant::now() + Self::DIAGNOSTIC_RETRY_TIMEOUT;
        loop {
            let request_id = self.take_request_id();
            let response_rx = self.write_request_payload(
                request_id,
                &document_diagnostic_request(
                    request_id,
                    file_path,
                    identifier.as_deref(),
                    previous_result_id.as_deref(),
                ),
            )?;
            match self.wait_for_response(request_id, response_rx, progress_sink, None) {
                Ok(result) => {
                    // Pull diagnostics use request/response transport, so forward the
                    // resulting snapshot explicitly instead of waiting for a push event.
                    let report =
                        parse_document_diagnostic_report(result.as_ref(), file_path, version)
                            .map_err(SessionError::Protocol)?;
                    self.apply_document_diagnostic_report(file_path, &report);
                    if let Some(update) = report.diagnostics {
                        progress_sink(SessionEvent::Diagnostics(update));
                    }
                    return Ok(());
                }
                Err(SessionError::RequestCancelled(_)) if Instant::now() < deadline => {
                    // Servers may cancel a diagnostic pull while analysis is still
                    // converging. Drop any previous result id before retrying so
                    // the follow-up request forces a fresh full report instead of
                    // repeating a cancelled incremental comparison.
                    previous_result_id = None;
                    self.await_startup_ready(Self::LOOKUP_RETRY_DELAY, progress_sink)?;
                }
                Err(SessionError::RequestCancelled(_)) => {
                    // Once the bounded retry window expires, treat the cancelled pull
                    // as best-effort and let the queued refresh request repull later.
                    self.state
                        .lock()
                        .expect("lock session state")
                        .pending_diagnostic_refresh = true;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Send `didClose` for one tracked document and forget its transport state.
    pub(crate) fn close_document(&self, file_path: &Path) -> Result<(), SessionError> {
        let mut state = self.state.lock().expect("lock session state");
        let removed = state.documents.remove(file_path);
        if removed.is_none() || !self.is_running() || !state.text_document_sync.open_close {
            return Ok(());
        }
        drop(state);
        self.write_payload(&did_close_notification(file_path))
    }

    /// Allocate the next LSP protocol version for one document path.
    #[cfg(test)]
    fn next_document_protocol_version(&self, file_path: &Path, request_version: i32) -> i32 {
        let state = self.state.lock().expect("lock session state");
        Self::next_document_protocol_version_from_state(&state, file_path, request_version)
    }

    /// Allocate the next LSP protocol version for one document path using held state.
    fn next_document_protocol_version_from_state(
        state: &SessionState,
        file_path: &Path,
        request_version: i32,
    ) -> i32 {
        state
            .documents
            .get(file_path)
            .map(|previous| previous.protocol_version.saturating_add(1))
            // LSP document versions must stay positive, so the first sync uses
            // version 1 when the caller has not recorded any prior version yet.
            .unwrap_or(request_version.max(1))
    }

    /// Wait for the server to emit post-startup traffic before the first lookup.
    fn await_startup_ready(
        &self,
        timeout: Duration,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.drain_pending_messages(progress_sink)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(Self::LOOKUP_RETRY_DELAY);
            {
                let state = self.state.lock().expect("lock session state");
                // Startup waits stop only after the server is visibly idle and the
                // short post-progress grace window has expired. That avoids firing
                // rename requests in the gap between progress ending and symbol
                // data becoming queryable across the workspace.
                if state.active_progress_tokens.is_empty()
                    && (!state.startup_ready || !Self::has_recent_progress_state(&state))
                {
                    return Ok(());
                }
            }
            if !self.wait_for_pending_messages(wait) {
                // A timeout while startup work is still active is not conclusive,
                // so keep polling until the bounded readiness window expires.
                continue;
            }
        }
        Ok(())
    }

    /// Send one JSON-RPC payload to the child process.
    fn write_payload(&self, payload: &json::JsonValue) -> Result<(), SessionError> {
        let mut runtime = self.runtime.lock().expect("lock session runtime");
        let stdin = runtime.stdin.as_mut().ok_or(SessionError::MissingStdin)?;
        append_lsp_trace_line(
            self.server.display_name,
            &self.workspace.root_path,
            "OUT",
            &payload.dump(),
        );
        write_message(stdin, payload).map_err(SessionError::Protocol)
    }

    /// Drain unsolicited server traffic without waiting for a request response.
    ///
    /// Returns `true` when at least one progress notification was forwarded, and
    /// `false` when no newly visible progress arrived during this poll.
    pub(crate) fn poll_notifications(
        &self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        self.drain_pending_messages(progress_sink)
    }

    /// Claim one queued diagnostic-refresh drain for background execution.
    ///
    /// Returns `true` when this caller acquired the refresh drain and must finish
    /// it on a worker thread, and `false` when no refresh is pending or another
    /// drain already owns the queued work.
    pub(crate) fn begin_pending_diagnostic_refresh(&self) -> bool {
        if self
            .diagnostic_refresh_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let has_pending_refresh = self
            .state
            .lock()
            .expect("lock session state")
            .pending_diagnostic_refresh;
        if has_pending_refresh {
            return true;
        }
        self.diagnostic_refresh_active
            .store(false, Ordering::SeqCst);
        false
    }

    /// Drain one previously claimed diagnostic-refresh pass on a worker thread.
    pub(crate) fn flush_claimed_diagnostic_refresh(
        &self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        let result = self.flush_pending_diagnostic_refresh(progress_sink);
        self.diagnostic_refresh_active
            .store(false, Ordering::SeqCst);
        result
    }

    /// Wait for one request response while continuing to process queued notifications.
    fn wait_for_response(
        &self,
        request_id: u64,
        response_rx: Receiver<TransportResponse>,
        progress_sink: &mut EventSink<'_>,
        cancellation: Option<RequestCancellation<'_>>,
    ) -> Result<Option<json::JsonValue>, SessionError> {
        loop {
            if let Some(cancellation) = cancellation.as_ref()
                && cancellation.flag.load(Ordering::SeqCst)
            {
                self.transport_shared
                    .response_waiters
                    .lock()
                    .expect("lock transport waiters")
                    .remove(&request_id);
                return Err(SessionError::Superseded(cancellation.superseded_kind));
            }
            self.drain_pending_messages(progress_sink)?;
            match response_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(TransportResponse::Result(result)) => {
                    self.state.lock().expect("lock session state").startup_ready = true;
                    self.flush_pending_diagnostic_refresh(progress_sink)?;
                    return Ok(result);
                }
                Ok(TransportResponse::Error(error)) => {
                    self.state.lock().expect("lock session state").startup_ready = true;
                    self.flush_pending_diagnostic_refresh(progress_sink)?;
                    return Err(self.session_error_from_response(error));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.transport_shared.closed.load(Ordering::SeqCst) {
                        return Err(Self::transport_closed_error());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Self::transport_closed_error());
                }
            }
        }
    }

    /// Process one incoming server message for the active loop variant.
    fn process_server_message(
        &self,
        message: ServerMessage,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<ProcessedMessage, SessionError> {
        match message {
            ServerMessage::Request { id, method, params } => {
                self.reply_to_server_request(id, &method, params.as_ref())?;
                if method == "workspace/diagnostic/refresh" {
                    // The server requests a client-initiated re-pull once fresh
                    // document diagnostics are ready after background analysis.
                    self.state
                        .lock()
                        .expect("lock session state")
                        .pending_diagnostic_refresh = true;
                }
                Ok(ProcessedMessage::default())
            }
            ServerMessage::Notification { method, params } => {
                // Notifications can carry progress updates, so surface them before
                // marking the session as ready for follow-up request work.
                let saw_progress =
                    self.handle_notification(&method, params.as_ref(), progress_sink)?;
                self.state.lock().expect("lock session state").startup_ready = true;
                Ok(ProcessedMessage { saw_progress })
            }
            ServerMessage::Response { .. } => Ok(ProcessedMessage {
                saw_progress: false,
            }),
        }
    }

    /// Execute one navigation request after the document snapshot is already synced.
    fn lookup_navigation_once(
        &self,
        request: &NavigationLookupRequest,
        kind: LookupKind,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        let request_id = self.take_request_id();
        let payload = match kind {
            LookupKind::Definition => {
                definition_request(request_id, &request.document.file_path, request.position)
            }
            LookupKind::References => {
                references_request(request_id, &request.document.file_path, request.position)
            }
        };
        let response_rx = self.write_request_payload(request_id, &payload)?;
        let result = self.wait_for_response(request_id, response_rx, progress_sink, None)?;
        let locations = parse_location_result(result.as_ref()).map_err(SessionError::Protocol)?;
        locations
            .into_iter()
            .map(|location| self.normalize_location(location))
            .collect()
    }

    /// Execute one hover request after the document snapshot is already synced.
    fn lookup_hover_once(
        &self,
        request: &HoverLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        let request_id = self.take_request_id();
        let payload = hover_request(request_id, &request.document.file_path, request.position);
        let response_rx = self.write_request_payload(request_id, &payload)?;
        let result = self.wait_for_response(request_id, response_rx, progress_sink, None)?;
        Ok(parse_hover_result(result.as_ref())
            .map_err(SessionError::Protocol)?
            .map(Cow::into_owned))
    }

    /// Execute one signature-help request after the document snapshot is already synced.
    fn lookup_signature_help_once(
        &self,
        request: &SignatureHelpLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspSignatureHelp>, SessionError> {
        let state = self.state.lock().expect("lock session state");
        let Some(provider) = state.signature_help_provider.as_ref() else {
            return Err(SessionError::Server(
                "language server does not support signature help".to_string(),
            ));
        };
        let trigger_text = request
            .trigger_text
            .as_deref()
            .filter(|typed_trigger| provider.supports_trigger_text(typed_trigger));
        drop(state);
        let request_id = self.take_request_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let superseded_requests = {
            let mut state = self.state.lock().expect("lock session state");
            let superseded_requests = std::mem::take(&mut state.pending_signature_help_requests);
            state
                .pending_signature_help_requests
                .insert(request_id, Arc::clone(&cancelled));
            superseded_requests
        };
        // Mature editors treat parameter hints as a latest-cursor-state feature,
        // so newer requests supersede older ones instead of waiting for them all.
        for (superseded_id, flag) in superseded_requests {
            flag.store(true, Ordering::SeqCst);
            self.cancel_request(superseded_id);
        }
        let payload = signature_help_request(
            request_id,
            &request.document.file_path,
            request.position,
            trigger_text,
            request.is_retrigger,
        );
        let response_rx = match self.write_request_payload(request_id, &payload) {
            Ok(response_rx) => response_rx,
            Err(error) => {
                self.state
                    .lock()
                    .expect("lock session state")
                    .pending_signature_help_requests
                    .remove(&request_id);
                return Err(error);
            }
        };
        let result = self.wait_for_response(
            request_id,
            response_rx,
            progress_sink,
            Some(RequestCancellation {
                flag: &cancelled,
                superseded_kind: SupersededRequestKind::SignatureHelp,
            }),
        );
        self.state
            .lock()
            .expect("lock session state")
            .pending_signature_help_requests
            .remove(&request_id);
        match result {
            Ok(result) => {
                parse_signature_help_result(result.as_ref()).map_err(SessionError::Protocol)
            }
            Err(SessionError::Superseded(SupersededRequestKind::SignatureHelp)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Execute one completion request after the document snapshot is already synced.
    fn lookup_completion_once(
        &self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let state = self.state.lock().expect("lock session state");
        let Some(provider) = state.completion_provider.as_ref() else {
            return Err(SessionError::Server(
                "language server does not support completions".to_string(),
            ));
        };
        let trigger_text = request
            .trigger_text
            .as_deref()
            .filter(|typed_trigger| provider.supports_trigger_text(typed_trigger));
        drop(state);
        let request_id = self.take_request_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let superseded_requests = {
            let mut state = self.state.lock().expect("lock session state");
            let superseded_requests = std::mem::take(&mut state.pending_completion_requests);
            state
                .pending_completion_requests
                .insert(request_id, Arc::clone(&cancelled));
            superseded_requests
        };
        // Completion requests become stale quickly while typing, so cancel any
        // older in-flight completion work before sending the latest lookup.
        for (superseded_id, flag) in superseded_requests {
            flag.store(true, Ordering::SeqCst);
            self.cancel_request(superseded_id);
        }
        let payload = completion_request(
            request_id,
            &request.document.file_path,
            request.position,
            trigger_text,
        );
        let response_rx = match self.write_request_payload(request_id, &payload) {
            Ok(response_rx) => response_rx,
            Err(error) => {
                self.state
                    .lock()
                    .expect("lock session state")
                    .pending_completion_requests
                    .remove(&request_id);
                return Err(error);
            }
        };
        let result = self.wait_for_response(
            request_id,
            response_rx,
            progress_sink,
            Some(RequestCancellation {
                flag: &cancelled,
                superseded_kind: SupersededRequestKind::Completion,
            }),
        );
        self.state
            .lock()
            .expect("lock session state")
            .pending_completion_requests
            .remove(&request_id);
        match result {
            Ok(result) => parse_completion_result(result.as_ref()).map_err(SessionError::Protocol),
            Err(SessionError::Superseded(SupersededRequestKind::Completion)) => {
                // A newer completion request already replaced this one, so report
                // an empty batch and let the freshest request own the popup state.
                Ok(Vec::new())
            }
            Err(error) => Err(error),
        }
    }

    /// Execute one rename request after the document snapshot is already synced.
    fn lookup_rename_once(
        &self,
        request: &RenameLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        let request_id = self.take_request_id();
        self.state
            .lock()
            .expect("lock session state")
            .pending_apply_edit = None;
        let payload = rename_request(
            request_id,
            &request.document.file_path,
            request.position,
            &request.new_name,
        );
        let response_rx = self.write_request_payload(request_id, &payload)?;
        let result = self.wait_for_response(request_id, response_rx, progress_sink, None)?;
        let response_edit =
            parse_workspace_edit_result(result.as_ref()).map_err(SessionError::Protocol)?;
        Ok(response_edit.or_else(|| {
            self.state
                .lock()
                .expect("lock session state")
                .pending_apply_edit
                .take()
        }))
    }

    /// Execute one code-action request after the document snapshot is already synced.
    fn lookup_code_action_once(
        &self,
        request: &CodeActionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCodeAction>, SessionError> {
        let request_id = self.take_request_id();
        let payload = code_action_request(
            request_id,
            &request.document.file_path,
            request.range,
            &request.diagnostics,
        );
        let response_rx = self.write_request_payload(request_id, &payload)?;
        let result = self.wait_for_response(request_id, response_rx, progress_sink, None)?;
        parse_code_action_result(result.as_ref()).map_err(SessionError::Protocol)
    }

    /// Synchronize the request document before starting one symbol lookup.
    fn prepare_lookup_document(
        &self,
        document: &DocumentSyncRequest,
        force_full_sync: bool,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<LookupPreparation, SessionError> {
        let started = self.ensure_started(progress_sink)?;
        let synced_for_lookup = force_full_sync
            || !self.should_skip_document_sync(&document.file_path, document.version);
        if force_full_sync {
            // Unsaved buffers can race with the proactive sync worker, so resend
            // a whole-document snapshot immediately before the lookup.
            self.force_full_document_sync(document)?;
            self.await_startup_ready(Self::LOOKUP_RETRY_DELAY, progress_sink)?;
        } else {
            self.apply_document_sync(document)?;
        }
        Ok(LookupPreparation {
            started,
            synced_for_lookup,
        })
    }

    /// Wait for one lookup iteration to become ready and return the prior readiness state.
    fn prepare_lookup_iteration(
        &self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        let startup_ready_before_request =
            self.state.lock().expect("lock session state").startup_ready;
        if !startup_ready_before_request {
            self.await_startup_ready(Self::STARTUP_READY_TIMEOUT, progress_sink)?;
        }
        Ok(startup_ready_before_request)
    }

    /// Retry one empty lookup result while startup work may still be settling.
    fn retry_empty_lookup(
        &self,
        started: bool,
        synced_for_lookup: bool,
        startup_ready_before_request: bool,
        deadline: Instant,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        if self.should_retry_empty_lookup(
            started,
            synced_for_lookup,
            startup_ready_before_request,
            deadline,
        ) {
            // Fresh sessions can answer before indexing settles, so keep polling
            // across the active retry budget after the first empty hit. Dirty-buffer
            // lookups also retry here because some servers need a short gap before
            // the synced text becomes queryable for symbol navigation.
            self.await_startup_ready(Self::LOOKUP_RETRY_DELAY, progress_sink)?;
            Ok(true)
        } else {
            Ok(false)
        }
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

    /// Retry one transient cancelled or content-modified lookup after forcing one full sync.
    fn retry_transient_lookup_failure(
        &self,
        document: &DocumentSyncRequest,
        forced_full_sync: &mut bool,
        deadline: Instant,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        // Unsaved-buffer lookups can race both document sync and server analysis.
        // One forced full sync gives the server a coherent snapshot before retrying.
        if !*forced_full_sync {
            self.force_full_document_sync(document)?;
            *forced_full_sync = true;
        }
        self.await_startup_ready(Self::STARTUP_READY_TIMEOUT, progress_sink)?;
        Ok(true)
    }

    /// Execute one navigation lookup with the transient retry policy the server needs.
    fn lookup_navigation_with_retry(
        &self,
        request: &NavigationLookupRequest,
        kind: LookupKind,
        preparation: LookupPreparation,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        let deadline = Instant::now() + Self::navigation_lookup_retry_timeout(kind);
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_navigation_once(request, kind, progress_sink) {
                Ok(targets) if !targets.is_empty() => {
                    // Some servers can report only the definition itself before
                    // cross-file reference indexing settles, so treat that startup
                    // placeholder like an empty result while the retry window is open.
                    if kind == LookupKind::References
                        && Self::references_only_origin_result(request, &targets)
                        && self.retry_empty_lookup(
                            preparation.started,
                            preparation.synced_for_lookup,
                            startup_ready_before_request,
                            deadline,
                            progress_sink,
                        )?
                    {
                        continue;
                    }
                    return Ok(targets);
                }
                Ok(targets) => {
                    if self.retry_empty_lookup(
                        preparation.started,
                        preparation.synced_for_lookup,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(targets);
                }
                Err(SessionError::RequestCancelled(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::RequestCancelled(error));
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::ContentModified(error));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one hover lookup with the transient retry policy the server needs.
    fn lookup_hover_with_retry(
        &self,
        request: &HoverLookupRequest,
        preparation: LookupPreparation,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_hover_once(request, progress_sink) {
                Ok(Some(text)) => return Ok(Some(text)),
                Ok(None) => {
                    // Hover can return empty while startup indexing is still
                    // catching up, so keep the same bounded retry window used by
                    // navigation lookups before concluding nothing is available.
                    if self.retry_empty_lookup(
                        preparation.started,
                        preparation.synced_for_lookup,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(None);
                }
                Err(SessionError::RequestCancelled(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::RequestCancelled(error));
                }
                Err(SessionError::ContentModified(error)) => {
                    // Unsaved-buffer hover requests can still race the debounced
                    // sync path, so one forced full sync is worth retrying before
                    // surfacing the server error to the user.
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::ContentModified(error));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one signature-help lookup with the transient retry policy the server needs.
    fn lookup_signature_help_with_retry(
        &self,
        request: &SignatureHelpLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspSignatureHelp>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_signature_help_once(request, progress_sink) {
                Ok(Some(help)) => return Ok(Some(help)),
                // Signature help is driven by the current cursor context. Like
                // VS Code parameter hints, an empty response should dismiss the
                // popup immediately instead of retrying older cursor states.
                Ok(None) => return Ok(None),
                Err(SessionError::RequestCancelled(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::RequestCancelled(error));
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::ContentModified(error));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one completion lookup with the transient retry policy the server needs.
    fn lookup_completion_with_retry(
        &self,
        request: &CompletionLookupRequest,
        preparation: LookupPreparation,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_completion_once(request, progress_sink) {
                Ok(items) if !items.is_empty() => return Ok(items),
                Ok(items) => {
                    if let Some(invoked_items) =
                        self.lookup_invoked_completion_fallback(request, progress_sink)?
                        && !invoked_items.is_empty()
                    {
                        return Ok(invoked_items);
                    }
                    // Completion can race startup indexing the same way hover can,
                    // so an empty batch is still retryable inside the bounded
                    // readiness window before it becomes a final empty result.
                    if self.retry_empty_lookup(
                        preparation.started,
                        preparation.synced_for_lookup,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(items);
                }
                Err(SessionError::RequestCancelled(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::RequestCancelled(error));
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::ContentModified(error));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Retry one empty trigger-driven completion as an ordinary invoked request.
    fn lookup_invoked_completion_fallback(
        &self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<Vec<LspCompletionItem>>, SessionError> {
        let Some(_) = request.trigger_text.as_ref() else {
            return Ok(None);
        };
        let mut invoked_request = request.clone();
        invoked_request.trigger_text = None;
        Ok(Some(
            self.lookup_completion_once(&invoked_request, progress_sink)?,
        ))
    }

    /// Execute one rename lookup with the transient retry policy the server needs.
    fn lookup_rename_with_retry(
        &self,
        request: &RenameLookupRequest,
        preparation: LookupPreparation,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_rename_once(request, progress_sink) {
                Ok(Some(edit)) => return Ok(Some(edit)),
                Ok(None) => {
                    if self.retry_empty_lookup(
                        preparation.started,
                        preparation.synced_for_lookup,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(None);
                }
                Err(SessionError::RequestCancelled(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::RequestCancelled(error));
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::ContentModified(error));
                }
                Err(SessionError::Server(error)) => return Err(SessionError::Server(error)),
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one code-action lookup with the transient retry policy the server needs.
    fn lookup_code_action_with_retry(
        &self,
        request: &CodeActionLookupRequest,
        preparation: LookupPreparation,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCodeAction>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_code_action_once(request, progress_sink) {
                Ok(actions) if !actions.is_empty() => return Ok(actions),
                Ok(actions) => {
                    if self.retry_empty_lookup(
                        preparation.started,
                        preparation.synced_for_lookup,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(actions);
                }
                Err(SessionError::RequestCancelled(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::RequestCancelled(error));
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_transient_lookup_failure(
                        &request.document,
                        &mut forced_full_sync,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Err(SessionError::ContentModified(error));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Execute one navigation lookup after synchronizing the request document.
    fn lookup_navigation(
        &self,
        request: &NavigationLookupRequest,
        kind: LookupKind,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        let preparation = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_navigation_with_retry(request, kind, preparation, progress_sink)
    }

    /// Execute one hover lookup after synchronizing the request document.
    fn lookup_hover_request(
        &self,
        request: &HoverLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        let preparation = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_hover_with_retry(request, preparation, progress_sink)
    }

    /// Execute one signature-help lookup after synchronizing the request document.
    fn lookup_signature_help_request(
        &self,
        request: &SignatureHelpLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspSignatureHelp>, SessionError> {
        self.prepare_lookup_document(&request.document, request.force_full_sync, progress_sink)?;
        self.lookup_signature_help_with_retry(request, progress_sink)
    }

    /// Execute one completion lookup after synchronizing the request document.
    fn lookup_completion_request(
        &self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let preparation = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_completion_with_retry(request, preparation, progress_sink)
    }

    /// Execute one rename lookup after synchronizing the request document.
    fn lookup_rename_request(
        &self,
        request: &RenameLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        let preparation = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_rename_with_retry(request, preparation, progress_sink)
    }

    /// Execute one code-action lookup after synchronizing the request document.
    fn lookup_code_action_request(
        &self,
        request: &CodeActionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCodeAction>, SessionError> {
        let preparation = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_code_action_with_retry(request, preparation, progress_sink)
    }

    /// Return whether one empty navigation response should be retried.
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
        let state = self.state.lock().expect("lock session state");
        Instant::now() < deadline
            && (synced_for_lookup
                || started
                || !startup_ready_before_request
                || !state.active_progress_tokens.is_empty()
                || Self::has_recent_progress_state(&state))
    }

    /// Return the retry budget for one navigation lookup kind.
    fn navigation_lookup_retry_timeout(kind: LookupKind) -> Duration {
        match kind {
            LookupKind::Definition => Self::LOOKUP_RETRY_TIMEOUT,
            LookupKind::References => Self::REFERENCES_LOOKUP_RETRY_TIMEOUT,
        }
    }

    /// Convert one response error into the retry-aware session failure Ordex uses.
    fn session_error_from_response(&self, error: LspResponseError) -> SessionError {
        match error.code {
            Self::REQUEST_CANCELLED_ERROR_CODE | Self::SERVER_CANCELLED_ERROR_CODE => {
                SessionError::RequestCancelled(error.message)
            }
            Self::CONTENT_MODIFIED_ERROR_CODE => SessionError::ContentModified(error.message),
            _ => SessionError::Server(error.message),
        }
    }

    /// Pull fresh diagnostics for all open documents after a claimed refresh request.
    fn flush_pending_diagnostic_refresh(
        &self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        // `diagnostic_refresh_active` ensures only one caller drains refresh
        // work at a time because callers must claim the drain before entering.
        // A refresh-triggered pull can itself prompt another refresh request, so
        // bound the loop to avoid spinning forever if a server keeps requeueing
        // refresh work faster than diagnostics can settle. `remaining_passes`
        // caps the drain so one refresh burst cannot monopolize idle polling.
        let mut remaining_passes = 4;
        while remaining_passes > 0 {
            let documents = {
                let mut state = self.state.lock().expect("lock session state");
                if !state.pending_diagnostic_refresh {
                    break;
                }
                state.pending_diagnostic_refresh = false;
                // Refresh requests apply to every tracked document, so capture the
                // current editor versions before issuing any nested LSP requests.
                state
                    .documents
                    .iter()
                    .map(|(path, state)| (path.clone(), state.editor_version))
                    .collect::<Vec<_>>()
            };
            remaining_passes -= 1;
            for (file_path, version) in documents {
                self.request_document_diagnostics(&file_path, version, progress_sink)?;
            }
        }
        Ok(())
    }

    /// Reply to one server-initiated request with a best-effort success payload.
    fn reply_to_server_request(
        &self,
        id: u64,
        method: &str,
        params: Option<&json::JsonValue>,
    ) -> Result<(), SessionError> {
        if method == "workspace/applyEdit" {
            self.state
                .lock()
                .expect("lock session state")
                .pending_apply_edit =
                Some(parse_apply_edit_request(params).map_err(SessionError::Protocol)?);
        }
        let result = server_request_result(method, params);
        self.write_payload(&server_request_response(id, result))
    }

    /// Decode one server notification and emit any forwarded payloads it contains.
    ///
    /// Returns `true` when the notification carried progress that was forwarded
    /// to the caller, and `false` when the notification was unrelated to progress.
    fn handle_notification(
        &self,
        method: &str,
        params: Option<&json::JsonValue>,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        if let Some(notification) = parse_progress_notification(method, params)? {
            self.apply_progress_notification(&notification);
            progress_sink(SessionEvent::Progress(notification));
            return Ok(true);
        }
        if let Some(diagnostics) = parse_publish_diagnostics_notification(method, params)? {
            progress_sink(SessionEvent::Diagnostics(
                self.normalize_published_diagnostics_version(diagnostics),
            ));
        }
        Ok(false)
    }

    /// Store the latest reusable pull-diagnostics result id for `file_path`, if any.
    fn apply_document_diagnostic_report(
        &self,
        file_path: &Path,
        report: &DocumentDiagnosticReport,
    ) {
        let mut state = self.state.lock().expect("lock session state");
        if let Some(document) = state.documents.get_mut(file_path)
            && report.result_id.is_some()
        {
            document.diagnostic_result_id = report.result_id.clone();
        }
    }

    /// Rewrite one pushed diagnostic version to the matching editor version when known.
    fn normalize_published_diagnostics_version(
        &self,
        mut diagnostics: LspFileDiagnostics,
    ) -> LspFileDiagnostics {
        let state = self.state.lock().expect("lock session state");
        if let Some(document) = state.documents.get(&diagnostics.file_path)
            && diagnostics.version == Some(document.protocol_version)
        {
            diagnostics.version = Some(document.editor_version);
        }
        diagnostics
    }

    /// Update in-flight token tracking from one typed progress notification.
    fn apply_progress_notification(&self, notification: &LspProgressNotification) {
        // Retry grace is refreshed by every progress event because the server
        // can finish the visible task shortly before navigation results become ready.
        let mut state = self.state.lock().expect("lock session state");
        state.recent_progress_deadline = Some(Instant::now() + Self::RECENT_PROGRESS_RETRY_WINDOW);
        match notification {
            LspProgressNotification::Begin { token, .. } => {
                state.active_progress_tokens.insert(token.clone());
            }
            LspProgressNotification::Report { .. } => {}
            LspProgressNotification::End { token, .. } => {
                state.active_progress_tokens.remove(token);
            }
        }
    }

    /// Return whether recent progress should keep retryable requests alive.
    ///
    /// Returns `true` when a progress event was observed within the active retry
    /// window, and `false` when that window has already expired or was never set.
    fn has_recent_progress_state(state: &SessionState) -> bool {
        state
            .recent_progress_deadline
            .is_some_and(|deadline| Instant::now() <= deadline)
    }

    /// Convert one protocol location into an editor-facing path and position.
    fn normalize_location(
        &self,
        location: LspLocation,
    ) -> Result<SessionNavigationTarget, SessionError> {
        Ok(SessionNavigationTarget {
            path: file_uri_to_path(&location.uri).map_err(SessionError::Protocol)?,
            line: location.line,
            character: location.character,
        })
    }

    /// Allocate the next JSON-RPC request id for this session.
    fn take_request_id(&self) -> u64 {
        self.next_request_id
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |id| {
                Some(if id == u64::MAX { 1 } else { id + 1 })
            })
            .expect("request id update should succeed")
    }
}

/// Stable navigation lookup kinds supported by the session transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookupKind {
    Definition,
    References,
}

/// Summary returned after one server message is processed by a session read loop.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ProcessedMessage {
    /// Whether this message delivered visible progress information.
    saw_progress: bool,
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

impl Drop for LspSession {
    /// Ensure child processes do not outlive the session object.
    fn drop(&mut self) {
        self.shutdown();
    }
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
    use crate::lsp::server::RUST_ANALYZER;
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
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

    /// Return one test workspace descriptor rooted at `tree`.
    fn tree_workspace(tree: &TempTree) -> ProjectWorkspace {
        ProjectWorkspace {
            root_path: tree.path().to_path_buf(),
            kind: ProjectRootKind::CargoWorkspace,
            marker_path: tree.path().join("Cargo.toml"),
        }
    }

    /// Write one fake server executable that logs diagnostic requests.
    fn write_fake_rust_analyzer(tree: &TempTree, log_path: &Path) {
        // The helper only needs initialize, diagnostic, and shutdown handling to
        // prove whether stale sync requests still trigger a pull-diagnostics roundtrip.
        tree.write_file(
            "rust-analyzer",
            &format!(
                r#"#!/usr/bin/env python3
import json, os, sys
LOG = {log_path:?}

def read_message():
    headers = {{}}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b'\r\n', b'\n'):
            break
        name, value = line.decode().split(':', 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers['content-length']))
    return json.loads(body)

def send(payload):
    data = json.dumps(payload).encode()
    sys.stdout.buffer.write(f'Content-Length: {{len(data)}}\r\n\r\n'.encode() + data)
    sys.stdout.buffer.flush()

while True:
    message = read_message()
    if message is None:
        break
    method = message.get('method')
    if method == 'initialize':
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{'capabilities': {{'textDocumentSync': {{'openClose': True, 'change': 1, 'save': {{}}}}, 'diagnosticProvider': {{'identifier': 'fake-server'}}}}}}}})
    elif method == 'textDocument/diagnostic':
        with open(LOG, 'a', encoding='utf-8') as handle:
            handle.write('diagnostic\n')
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': {{'kind': 'full', 'resultId': 'fake-result', 'items': []}}}})
    elif method == 'shutdown':
        send({{'jsonrpc': '2.0', 'id': message['id'], 'result': None}})
"#
            ),
        )
        .expect("write fake rust-analyzer");
        let script_path = tree.path().join("rust-analyzer");
        let mut permissions = fs::metadata(&script_path)
            .expect("stat fake rust-analyzer")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod fake rust-analyzer");
    }

    /// Confirm that request ids advance monotonically across one session.
    #[test]
    fn test_take_request_id_advances_monotonically() {
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);

        assert_eq!(session.take_request_id(), 1);
        assert_eq!(session.take_request_id(), 2);
    }

    /// Confirm stale sync work cannot move the tracked document version backward.
    #[test]
    fn test_should_skip_document_sync_for_stale_version() {
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session
            .state
            .lock()
            .expect("lock session state")
            .documents
            .insert(
                file_path.clone(),
                SessionDocumentState {
                    editor_version: 4,
                    protocol_version: 7,
                    diagnostic_result_id: None,
                },
            );

        assert!(session.should_skip_document_sync(&file_path, 3));
        assert!(session.should_skip_document_sync(&file_path, 4));
        assert!(!session.should_skip_document_sync(&file_path, 5));
        assert!(!session.should_skip_document_sync(Path::new("/tmp/workspace/src/lib.rs"), 1));
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
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session
            .state
            .lock()
            .expect("lock session state")
            .documents
            .insert(
                file_path.clone(),
                SessionDocumentState {
                    editor_version: 4,
                    protocol_version: 7,
                    diagnostic_result_id: None,
                },
            );

        assert_eq!(session.next_document_protocol_version(&file_path, 4), 8);
        assert_eq!(session.next_document_protocol_version(&file_path, 5), 8);
        assert_eq!(
            session.next_document_protocol_version(Path::new("/tmp/workspace/src/lib.rs"), 0),
            1
        );
    }

    /// Confirm empty navigation retries stay enabled for startup and fresh sync races.
    #[test]
    fn test_should_retry_empty_lookup_only_during_startup_window() {
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let deadline = Instant::now() + Duration::from_secs(1);

        assert!(session.should_retry_empty_lookup(true, false, true, deadline));
        assert!(session.should_retry_empty_lookup(false, false, false, deadline));
        assert!(session.should_retry_empty_lookup(false, true, true, deadline));
        assert!(!session.should_retry_empty_lookup(false, false, true, deadline));

        session
            .state
            .lock()
            .expect("lock session state")
            .active_progress_tokens
            .insert("cargo-index".to_string());
        assert!(session.should_retry_empty_lookup(false, false, true, deadline));

        let mut state = session.state.lock().expect("lock session state");
        state.active_progress_tokens.clear();
        state.recent_progress_deadline = Some(Instant::now() + Duration::from_millis(250));
        drop(state);
        assert!(session.should_retry_empty_lookup(false, false, true, deadline));
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

        assert!(LspSession::references_only_origin_result(
            &request,
            &[origin_target]
        ));
        assert!(!LspSession::references_only_origin_result(
            &request,
            &[shifted_target]
        ));
    }

    /// Confirm response errors map to retry-aware session variants by LSP code.
    #[test]
    fn test_session_error_from_response_uses_lsp_error_codes() {
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);

        assert!(matches!(
            session.session_error_from_response(LspResponseError {
                code: LspSession::REQUEST_CANCELLED_ERROR_CODE,
                message: "request cancelled".to_string(),
            }),
            SessionError::RequestCancelled(_)
        ));
        assert!(matches!(
            session.session_error_from_response(LspResponseError {
                code: LspSession::CONTENT_MODIFIED_ERROR_CODE,
                message: "content modified".to_string(),
            }),
            SessionError::ContentModified(_)
        ));
        assert!(matches!(
            session.session_error_from_response(LspResponseError {
                code: -32001,
                message: "server error".to_string(),
            }),
            SessionError::Server(_)
        ));
    }

    /// Confirm pushed diagnostics use the editor version for the latest synced snapshot.
    #[test]
    fn test_normalize_published_diagnostics_version_maps_latest_protocol_version() {
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session
            .state
            .lock()
            .expect("lock session state")
            .documents
            .insert(
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
        let normalized = session.normalize_published_diagnostics_version(diagnostics);

        assert_eq!(normalized.version, Some(5));
    }

    /// Confirm pull-diagnostics reports retain the latest reusable result id.
    #[test]
    fn test_apply_document_diagnostic_report_updates_result_id() {
        let session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session
            .state
            .lock()
            .expect("lock session state")
            .documents
            .insert(
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
        session.apply_document_diagnostic_report(&file_path, &report);

        assert_eq!(
            session
                .state
                .lock()
                .expect("lock session state")
                .documents
                .get(&file_path)
                .and_then(|state| state.diagnostic_result_id.as_deref()),
            Some("diag-1")
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
        write_fake_rust_analyzer(&tree, &log_path);
        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let mut combined_path = OsString::from(tree.path().as_os_str());
        combined_path.push(OsString::from(":"));
        combined_path.push(original_path);
        let _path_guard = EnvVarGuard::set(&lock, "PATH", combined_path);
        let file_path = tree.path().join("src/main.rs");
        let session = LspSession::new(tree_workspace(&tree), &RUST_ANALYZER);
        let mut ignore_events = |_| {};
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

        session
            .sync_document(&fresh_request, &mut ignore_events)
            .expect("sync fresh request");
        session
            .sync_document(&stale_request, &mut ignore_events)
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
        let session = LspSession::new(
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
        let mut ignore_progress = |_| {};
        let deadline = Instant::now() + Duration::from_secs(5);

        loop {
            let request = NavigationLookupRequest {
                document: document.clone(),
                force_full_sync: true,
                position,
            };
            let references = session
                .lookup_references(&request, &mut ignore_progress)
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

        let edit = session
            .lookup_rename(&request, &mut ignore_progress)
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
}
