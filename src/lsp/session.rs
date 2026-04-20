//! Shared language-server process sessions reused across requests in one workspace.

use super::diagnostics::LspFileDiagnostics;
use super::project::ProjectWorkspace;
use super::protocol::{
    CompletionProvider, DocumentDiagnosticProvider, DocumentDiagnosticReport, LspCompletionItem,
    LspLocation, LspPosition, LspProgressNotification, LspResponseError, LspTextChange,
    LspWorkspaceEdit, ProtocolError, ServerMessage, TextDocumentSyncKind, TextDocumentSyncOptions,
    completion_request, definition_request, did_change_notification, did_close_notification,
    did_open_notification, did_save_notification, document_diagnostic_request, exit_notification,
    file_uri_to_path, hover_request, initialize_request, initialized_notification,
    parse_apply_edit_request, parse_completion_provider, parse_completion_result,
    parse_document_diagnostic_provider, parse_document_diagnostic_report, parse_hover_result,
    parse_location_result, parse_progress_notification, parse_publish_diagnostics_notification,
    parse_text_document_sync_options, parse_workspace_edit_result, read_message,
    references_request, rename_request, server_request_response, server_request_result,
    shutdown_request, write_message,
};
use super::server::LspServerDescriptor;
use crate::unsafe_io::poll_fd;
use ropey::Rope;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

/// Input needed to execute one completion lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionLookupRequest {
    /// Document snapshot that must be visible to the server before completion.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based completion position in LSP coordinates.
    pub(crate) position: LspPosition,
    /// Recently typed character used to mark one immediate trigger request.
    pub(crate) trigger_character: Option<String>,
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
    Spawn(io::Error),
    MissingStdin,
    MissingStdout,
    Protocol(ProtocolError),
    RequestCancelled(String),
    ContentModified(String),
    Server(String),
}

impl fmt::Display for SessionError {
    /// Format one session failure for status messages and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "failed to start language server: {error}"),
            Self::MissingStdin => write!(f, "language server did not expose stdin"),
            Self::MissingStdout => write!(f, "language server did not expose stdout"),
            Self::Protocol(error) => write!(f, "{error}"),
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

/// One reusable language-server process keyed by workspace root.
#[derive(Debug)]
pub(crate) struct LspSession {
    workspace: ProjectWorkspace,
    server: &'static LspServerDescriptor,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    next_request_id: u64,
    documents: HashMap<PathBuf, SessionDocumentState>,
    /// Tokens for progress tasks that have begun and not yet ended, used to keep
    /// navigation retries alive while the language server still reports active work.
    active_progress_tokens: HashSet<String>,
    /// Deadline that keeps empty-navigation retries alive briefly after the most
    /// recent progress event so the index can become queryable after visible work ends.
    recent_progress_deadline: Option<Instant>,
    /// Most recent workspace edit requested through `workspace/applyEdit`.
    pending_apply_edit: Option<LspWorkspaceEdit>,
    text_document_sync: TextDocumentSyncOptions,
    document_diagnostic_provider: Option<DocumentDiagnosticProvider>,
    completion_provider: Option<CompletionProvider>,
    /// Whether a `workspace/diagnostic/refresh` request arrived and still needs
    /// one follow-up pull pass for the currently tracked open documents.
    pending_diagnostic_refresh: bool,
    startup_ready: bool,
}

impl LspSession {
    /// Maximum wait for one startup message before the session treats the server
    /// as ready enough to continue with the current request.
    const STARTUP_READY_TIMEOUT: Duration = Duration::from_secs(2);
    /// Delay between navigation retries while startup work is settling.
    const LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(150);
    /// Total retry budget for one navigation lookup that races startup indexing.
    const LOOKUP_RETRY_TIMEOUT: Duration = Duration::from_secs(10);
    /// Total retry budget for one pull-diagnostics request cancelled during analysis.
    const DIAGNOSTIC_RETRY_TIMEOUT: Duration = Duration::from_secs(2);
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
            child: None,
            stdin: None,
            stdout: None,
            next_request_id: 1,
            documents: HashMap::new(),
            active_progress_tokens: HashSet::new(),
            recent_progress_deadline: None,
            pending_apply_edit: None,
            text_document_sync: TextDocumentSyncOptions::default(),
            document_diagnostic_provider: None,
            completion_provider: None,
            pending_diagnostic_refresh: false,
            startup_ready: false,
        }
    }

    /// Return the built-in server descriptor that owns this session.
    pub(crate) fn server_descriptor(&self) -> &'static LspServerDescriptor {
        self.server
    }

    /// Synchronize one document snapshot into the running language server.
    pub(crate) fn sync_document(
        &mut self,
        request: &DocumentSyncRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        self.synchronize_document(request, progress_sink)?;
        self.request_document_diagnostics(&request.file_path, request.version, progress_sink)?;
        Ok(())
    }

    /// Synchronize one document snapshot for a save lifecycle before `didSave`.
    pub(crate) fn sync_document_for_save(
        &mut self,
        request: &DocumentSyncRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        self.synchronize_document(request, progress_sink)
    }

    /// Execute one definition lookup against the running language server.
    pub(crate) fn lookup_definition(
        &mut self,
        request: &NavigationLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_navigation(request, LookupKind::Definition, progress_sink)
    }

    /// Execute one references lookup against the running language server.
    pub(crate) fn lookup_references(
        &mut self,
        request: &NavigationLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        self.lookup_navigation(request, LookupKind::References, progress_sink)
    }

    /// Execute one hover lookup against the running language server.
    pub(crate) fn lookup_hover(
        &mut self,
        request: &HoverLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        self.lookup_hover_request(request, progress_sink)
    }

    /// Execute one completion lookup against the running language server.
    pub(crate) fn lookup_completion(
        &mut self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        self.lookup_completion_request(request, progress_sink)
    }

    /// Return whether the running session advertises `character` as a completion trigger.
    ///
    /// Returns `true` when the initialized server wants immediate completion after
    /// `character`, and `false` when the client should use ordinary debounce timing.
    pub(crate) fn completion_trigger_support(&self, trigger_text: &str) -> Option<bool> {
        self.completion_provider
            .as_ref()
            .map(|provider| provider.supports_trigger_text(trigger_text))
    }

    /// Execute one rename lookup against the running language server.
    pub(crate) fn lookup_rename(
        &mut self,
        request: &RenameLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        self.lookup_rename_request(request, progress_sink)
    }

    /// Shut down the child process if it was started.
    pub(crate) fn shutdown(&mut self) {
        if self.child.is_none() {
            return;
        }
        // Ask the server to shut down cleanly first so it can flush any in-flight
        // responses and exit on its own before Ordex escalates to termination.
        let request_id = self.take_request_id();
        // Shutdown still reuses the ordinary response-reading path, and that path
        // can observe late progress notifications while the session is draining.
        // A no-op sink preserves the shared logic without reopening UI updates.
        let mut ignore_events = |_| {};
        let _ = self.write_payload(&shutdown_request(request_id));
        let _ = self.read_response(request_id, &mut ignore_events);
        let _ = self.write_payload(&exit_notification());
        if let Some(mut child) = self.child.take()
            && !wait_for_graceful_shutdown(&mut child, Duration::from_millis(100))
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.stdout = None;
        self.documents.clear();
        self.active_progress_tokens.clear();
        self.recent_progress_deadline = None;
        self.pending_apply_edit = None;
        self.completion_provider = None;
        self.pending_diagnostic_refresh = false;
        self.startup_ready = false;
    }

    /// Start the language server and complete the initialize handshake when needed.
    ///
    /// Returns `Ok(true)` when this call spawned a fresh child process, and
    /// `Ok(false)` when an existing child was already running.
    fn ensure_started(&mut self, progress_sink: &mut EventSink<'_>) -> Result<bool, SessionError> {
        if self.child.is_some() {
            return Ok(false);
        }
        // Server descriptors can append workspace-scoped startup arguments, so
        // resolve the final command line before constructing the child process.
        let command_args = self
            .server
            .command_args(&self.workspace.root_path)
            .map_err(SessionError::Spawn)?;
        let mut command = Command::new(self.server.command_program());
        command
            .args(&command_args)
            .current_dir(&self.workspace.root_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(SessionError::Spawn)?;
        let stdin = child.stdin.take().ok_or(SessionError::MissingStdin)?;
        let stdout = child.stdout.take().ok_or(SessionError::MissingStdout)?;
        self.stdin = Some(stdin);
        self.stdout = Some(BufReader::new(stdout));
        self.child = Some(child);

        let request_id = self.take_request_id();
        self.write_payload(&initialize_request(request_id, &self.workspace.root_path))?;
        let result = self.read_response(request_id, progress_sink)?;
        self.text_document_sync =
            parse_text_document_sync_options(result.as_ref()).map_err(SessionError::Protocol)?;
        self.document_diagnostic_provider =
            parse_document_diagnostic_provider(result.as_ref()).map_err(SessionError::Protocol)?;
        self.completion_provider =
            parse_completion_provider(result.as_ref()).map_err(SessionError::Protocol)?;
        self.write_payload(&initialized_notification())?;
        self.startup_ready = false;
        Ok(true)
    }

    /// Send `didOpen` or `didChange` so the server sees the current buffer snapshot.
    fn apply_document_sync(&mut self, request: &DocumentSyncRequest) -> Result<(), SessionError> {
        if self.should_skip_document_sync(&request.file_path, request.version) {
            return Ok(());
        }
        let text = request.text.to_string();
        let protocol_version =
            self.next_document_protocol_version(&request.file_path, request.version);
        let language_id = self
            .server
            .lsp_language_id(&request.file_path)
            .ok_or_else(|| {
                SessionError::Server("unsupported LSP language for document".to_string())
            })?;
        let payload = if self.documents.contains_key(&request.file_path) {
            // Once the document is open, prefer the negotiated sync mode but
            // keep a whole-document fallback for stale or empty edit queues.
            self.change_notification(request, protocol_version, &text)
        } else {
            did_open_notification(&request.file_path, language_id, protocol_version, &text)
        };
        self.write_payload(&payload)?;
        let diagnostic_result_id = self
            .documents
            .get(&request.file_path)
            .and_then(|state| state.diagnostic_result_id.clone());
        self.documents.insert(
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
    fn synchronize_document(
        &mut self,
        request: &DocumentSyncRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        let started = self.ensure_started(progress_sink)?;
        if self.should_skip_document_sync(&request.file_path, request.version) {
            return Ok(());
        }
        // Debounced background sync favors one coherent full-text snapshot over a
        // long queued edit batch so diagnostics always reflect the live buffer.
        self.force_full_document_sync(request)?;
        if started {
            // Startup progress often arrives immediately after `didOpen`, so the
            // first background sync waits briefly to surface launch-time feedback.
            self.await_startup_ready(Self::STARTUP_READY_TIMEOUT, progress_sink)?;
        }
        Ok(())
    }

    /// Send one full-text sync even when the tracked version already matches.
    fn force_full_document_sync(
        &mut self,
        request: &DocumentSyncRequest,
    ) -> Result<(), SessionError> {
        let text = request.text.to_string();
        let protocol_version =
            self.next_document_protocol_version(&request.file_path, request.version);
        let language_id = self
            .server
            .lsp_language_id(&request.file_path)
            .ok_or_else(|| {
                SessionError::Server("unsupported LSP language for document".to_string())
            })?;
        let payload = if self.documents.contains_key(&request.file_path) {
            did_change_notification(
                &request.file_path,
                protocol_version,
                &[LspTextChange { range: None, text }],
            )
        } else {
            did_open_notification(&request.file_path, language_id, protocol_version, &text)
        };
        self.write_payload(&payload)?;
        let diagnostic_result_id = self
            .documents
            .get(&request.file_path)
            .and_then(|state| state.diagnostic_result_id.clone());
        self.documents.insert(
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
        self.documents
            .get(file_path)
            .is_some_and(|previous| previous.editor_version >= request_version)
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

    /// Send `didSave` for one already-synchronized document when the server wants it.
    pub(crate) fn save_document(
        &mut self,
        file_path: &Path,
        text: &Rope,
    ) -> Result<(), SessionError> {
        let Some(save_options) = self.text_document_sync.save else {
            return Ok(());
        };
        if self.child.is_none() || !self.documents.contains_key(file_path) {
            return Ok(());
        }
        // Convert the rope lazily so save notifications stay cheap for servers
        // that only need the URI and not the full saved contents.
        let text = save_options.include_text.then(|| text.to_string());
        self.write_payload(&did_save_notification(file_path, text.as_deref()))
    }

    /// Pull fresh diagnostics for one synchronized document when the server supports it.
    pub(crate) fn request_document_diagnostics(
        &mut self,
        file_path: &Path,
        version: i32,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        let Some(provider) = self.document_diagnostic_provider.as_ref() else {
            return Ok(());
        };
        let identifier = provider.identifier.clone();
        let previous_result_id = self
            .documents
            .get(file_path)
            .and_then(|state| state.diagnostic_result_id.clone());
        if self.child.is_none() || !self.documents.contains_key(file_path) {
            return Ok(());
        }
        let deadline = Instant::now() + Self::DIAGNOSTIC_RETRY_TIMEOUT;
        loop {
            let request_id = self.take_request_id();
            self.write_payload(&document_diagnostic_request(
                request_id,
                file_path,
                identifier.as_deref(),
                previous_result_id.as_deref(),
            ))?;
            match self.read_response(request_id, progress_sink) {
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
                    // converging, so wait briefly and retry the same explicit pull.
                    self.await_startup_ready(Self::LOOKUP_RETRY_DELAY, progress_sink)?;
                }
                Err(SessionError::RequestCancelled(_)) => {
                    // Once the bounded retry window expires, treat the cancelled pull
                    // as best-effort and let the queued refresh request repull later.
                    self.pending_diagnostic_refresh = true;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Send `didClose` for one tracked document and forget its transport state.
    pub(crate) fn close_document(&mut self, file_path: &Path) -> Result<(), SessionError> {
        let removed = self.documents.remove(file_path);
        if removed.is_none() || self.child.is_none() || !self.text_document_sync.open_close {
            return Ok(());
        }
        self.write_payload(&did_close_notification(file_path))
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

    /// Wait for the server to emit post-startup traffic before the first lookup.
    fn await_startup_ready(
        &mut self,
        timeout: Duration,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let wait = remaining.min(Self::LOOKUP_RETRY_DELAY);
            let Some(message) = self.read_message_with_timeout(wait)? else {
                // Startup waits stop only after the server is visibly idle and the
                // short post-progress grace window has expired. That avoids firing
                // rename requests in the gap between progress ending and symbol
                // data becoming queryable across the workspace.
                if self.active_progress_tokens.is_empty()
                    && (!self.startup_ready || !self.has_recent_progress())
                {
                    return Ok(());
                }
                // A timeout while startup work is still active is not conclusive,
                // so keep polling until the bounded readiness window expires.
                continue;
            };
            self.process_server_message(message, progress_sink, None)?;
        }
        Ok(())
    }

    /// Send one JSON-RPC payload to the child process.
    fn write_payload(&mut self, payload: &json::JsonValue) -> Result<(), SessionError> {
        let stdin = self.stdin.as_mut().ok_or(SessionError::MissingStdin)?;
        write_message(stdin, payload).map_err(SessionError::Protocol)
    }

    /// Read one server message if stdout becomes readable before `timeout`.
    fn read_message_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<ServerMessage>, SessionError> {
        let stdout = self.stdout.as_mut().ok_or(SessionError::MissingStdout)?;
        if !Self::stdout_has_message_ready(stdout.get_ref(), timeout)
            .map_err(ProtocolError::Io)
            .map_err(SessionError::Protocol)?
        {
            return Ok(None);
        }
        read_message(stdout)
            .map(Some)
            .map_err(SessionError::Protocol)
    }

    /// Drain unsolicited server traffic without waiting for a request response.
    ///
    /// Returns `true` when at least one progress notification was forwarded, and
    /// `false` when no newly visible progress arrived during this poll.
    pub(crate) fn poll_notifications(
        &mut self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        let mut saw_progress = false;
        loop {
            let Some(message) = self.read_message_with_timeout(Duration::ZERO)? else {
                self.flush_pending_diagnostic_refresh(progress_sink)?;
                return Ok(saw_progress);
            };
            saw_progress |= self
                .process_server_message(message, progress_sink, None)?
                .saw_progress;
        }
    }

    /// Return whether stdout has readable bytes before `timeout`.
    ///
    /// Returns `true` when `poll` reported readable data for the child stdout,
    /// and `false` when the timeout elapsed or only non-readable events arrived.
    fn stdout_has_message_ready(stdout: &ChildStdout, timeout: Duration) -> io::Result<bool> {
        let outcome = poll_fd(stdout, poll_timeout_ms(timeout))?;
        // `ready` reports whether `poll` woke up before the timeout, while the
        // `POLLIN` bit confirms that the wake-up includes bytes we can read.
        Ok(outcome.ready && (outcome.revents & libc::POLLIN) != 0)
    }

    /// Read responses until the requested id arrives, skipping notifications.
    fn read_response(
        &mut self,
        request_id: u64,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<json::JsonValue>, SessionError> {
        loop {
            let stdout = self.stdout.as_mut().ok_or(SessionError::MissingStdout)?;
            let message = read_message(stdout)?;
            if let ProcessedResponse::Matched(result) = self
                .process_server_message(message, progress_sink, Some(request_id))?
                .response
            {
                self.flush_pending_diagnostic_refresh(progress_sink)?;
                return Ok(result);
            }
        }
    }

    /// Process one incoming server message for the active loop variant.
    fn process_server_message(
        &mut self,
        message: ServerMessage,
        progress_sink: &mut EventSink<'_>,
        awaited_response_id: Option<u64>,
    ) -> Result<ProcessedMessage, SessionError> {
        match message {
            ServerMessage::Request { id, method, params } => {
                self.reply_to_server_request(id, &method, params.as_ref())?;
                if method == "workspace/diagnostic/refresh" {
                    // The server requests a client-initiated re-pull once fresh
                    // document diagnostics are ready after background analysis.
                    self.pending_diagnostic_refresh = true;
                }
                Ok(ProcessedMessage::default())
            }
            ServerMessage::Notification { method, params } => {
                // Notifications can carry progress updates, so surface them before
                // marking the session as ready for follow-up request work.
                let saw_progress =
                    self.handle_notification(&method, params.as_ref(), progress_sink)?;
                self.startup_ready = true;
                Ok(ProcessedMessage {
                    saw_progress,
                    ready_signal: true,
                    response: ProcessedResponse::None,
                })
            }
            ServerMessage::Response { id, result, error } => {
                self.startup_ready = true;
                if awaited_response_id == Some(id) {
                    if let Some(error) = error {
                        return Err(self.session_error_from_response(error));
                    }
                    return Ok(ProcessedMessage {
                        saw_progress: false,
                        ready_signal: true,
                        response: ProcessedResponse::Matched(result),
                    });
                }
                Ok(ProcessedMessage {
                    saw_progress: false,
                    ready_signal: true,
                    response: ProcessedResponse::None,
                })
            }
        }
    }

    /// Execute one navigation request after the document snapshot is already synced.
    fn lookup_navigation_once(
        &mut self,
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
        self.write_payload(&payload)?;
        let result = self.read_response(request_id, progress_sink)?;
        let locations = parse_location_result(result.as_ref()).map_err(SessionError::Protocol)?;
        locations
            .into_iter()
            .map(|location| self.normalize_location(location))
            .collect()
    }

    /// Execute one hover request after the document snapshot is already synced.
    fn lookup_hover_once(
        &mut self,
        request: &HoverLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        let request_id = self.take_request_id();
        let payload = hover_request(request_id, &request.document.file_path, request.position);
        self.write_payload(&payload)?;
        let result = self.read_response(request_id, progress_sink)?;
        Ok(parse_hover_result(result.as_ref())
            .map_err(SessionError::Protocol)?
            .map(Cow::into_owned))
    }

    /// Execute one completion request after the document snapshot is already synced.
    fn lookup_completion_once(
        &mut self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let Some(provider) = self.completion_provider.as_ref() else {
            return Err(SessionError::Server(
                "language server does not support completions".to_string(),
            ));
        };
        let completion_provider = provider.clone();
        let request_id = self.take_request_id();
        let trigger_character = request
            .trigger_character
            .as_deref()
            .filter(|character| completion_provider.supports_trigger_text(character));
        let payload = completion_request(
            request_id,
            &request.document.file_path,
            request.position,
            trigger_character,
        );
        self.write_payload(&payload)?;
        let result = self.read_response(request_id, progress_sink)?;
        parse_completion_result(result.as_ref()).map_err(SessionError::Protocol)
    }

    /// Execute one rename request after the document snapshot is already synced.
    fn lookup_rename_once(
        &mut self,
        request: &RenameLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        let request_id = self.take_request_id();
        self.pending_apply_edit = None;
        let payload = rename_request(
            request_id,
            &request.document.file_path,
            request.position,
            &request.new_name,
        );
        self.write_payload(&payload)?;
        let result = self.read_response(request_id, progress_sink)?;
        let response_edit =
            parse_workspace_edit_result(result.as_ref()).map_err(SessionError::Protocol)?;
        Ok(response_edit.or_else(|| self.pending_apply_edit.take()))
    }

    /// Synchronize the request document before starting one symbol lookup.
    fn prepare_lookup_document(
        &mut self,
        document: &DocumentSyncRequest,
        force_full_sync: bool,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        let started = self.ensure_started(progress_sink)?;
        if force_full_sync {
            // Unsaved buffers can race with the proactive sync worker, so resend
            // a whole-document snapshot immediately before the lookup.
            self.force_full_document_sync(document)?;
            self.await_startup_ready(Self::LOOKUP_RETRY_DELAY, progress_sink)?;
        } else {
            self.apply_document_sync(document)?;
        }
        Ok(started)
    }

    /// Wait for one lookup iteration to become ready and return the prior readiness state.
    fn prepare_lookup_iteration(
        &mut self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        let startup_ready_before_request = self.startup_ready;
        if !startup_ready_before_request {
            self.await_startup_ready(Self::STARTUP_READY_TIMEOUT, progress_sink)?;
        }
        Ok(startup_ready_before_request)
    }

    /// Retry one empty lookup result while startup work may still be settling.
    fn retry_empty_lookup(
        &mut self,
        started: bool,
        startup_ready_before_request: bool,
        deadline: Instant,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        if self.should_retry_empty_lookup(started, startup_ready_before_request, deadline) {
            // Fresh sessions can answer before indexing settles, so keep polling
            // briefly after the first empty hit.
            self.await_startup_ready(Self::LOOKUP_RETRY_DELAY, progress_sink)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Retry one transient content-modified failure after forcing a full sync once.
    fn retry_content_modified_lookup(
        &mut self,
        document: &DocumentSyncRequest,
        forced_full_sync: &mut bool,
        deadline: Instant,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<bool, SessionError> {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        // Unsaved-buffer lookups can race the background sync path. One forced
        // full sync gives the server a coherent snapshot before the retry.
        if !*forced_full_sync {
            self.force_full_document_sync(document)?;
            *forced_full_sync = true;
        }
        self.await_startup_ready(Self::STARTUP_READY_TIMEOUT, progress_sink)?;
        Ok(true)
    }

    /// Execute one navigation lookup with the transient retry policy the server needs.
    fn lookup_navigation_with_retry(
        &mut self,
        request: &NavigationLookupRequest,
        kind: LookupKind,
        started: bool,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_navigation_once(request, kind, progress_sink) {
                Ok(targets) if !targets.is_empty() => return Ok(targets),
                Ok(targets) => {
                    if self.retry_empty_lookup(
                        started,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(targets);
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_content_modified_lookup(
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
        &mut self,
        request: &HoverLookupRequest,
        started: bool,
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
                        started,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(None);
                }
                Err(SessionError::ContentModified(error)) => {
                    // Unsaved-buffer hover requests can still race the debounced
                    // sync path, so one forced full sync is worth retrying before
                    // surfacing the server error to the user.
                    if self.retry_content_modified_lookup(
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
        &mut self,
        request: &CompletionLookupRequest,
        started: bool,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.prepare_lookup_iteration(progress_sink)?;
            match self.lookup_completion_once(request, progress_sink) {
                Ok(items) if !items.is_empty() => return Ok(items),
                Ok(items) => {
                    // Completion can race startup indexing the same way hover can,
                    // so an empty batch is still retryable inside the bounded
                    // readiness window before it becomes a final empty result.
                    if self.retry_empty_lookup(
                        started,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(items);
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_content_modified_lookup(
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

    /// Execute one rename lookup with the transient retry policy the server needs.
    fn lookup_rename_with_retry(
        &mut self,
        request: &RenameLookupRequest,
        started: bool,
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
                        started,
                        startup_ready_before_request,
                        deadline,
                        progress_sink,
                    )? {
                        continue;
                    }
                    return Ok(None);
                }
                Err(SessionError::ContentModified(error)) => {
                    if self.retry_content_modified_lookup(
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

    /// Execute one navigation lookup after synchronizing the request document.
    fn lookup_navigation(
        &mut self,
        request: &NavigationLookupRequest,
        kind: LookupKind,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<SessionNavigationTarget>, SessionError> {
        let started = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_navigation_with_retry(request, kind, started, progress_sink)
    }

    /// Execute one hover lookup after synchronizing the request document.
    fn lookup_hover_request(
        &mut self,
        request: &HoverLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<String>, SessionError> {
        let started = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_hover_with_retry(request, started, progress_sink)
    }

    /// Execute one completion lookup after synchronizing the request document.
    fn lookup_completion_request(
        &mut self,
        request: &CompletionLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Vec<LspCompletionItem>, SessionError> {
        let started = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_completion_with_retry(request, started, progress_sink)
    }

    /// Execute one rename lookup after synchronizing the request document.
    fn lookup_rename_request(
        &mut self,
        request: &RenameLookupRequest,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<Option<LspWorkspaceEdit>, SessionError> {
        let started = self.prepare_lookup_document(
            &request.document,
            request.force_full_sync,
            progress_sink,
        )?;
        self.lookup_rename_with_retry(request, started, progress_sink)
    }

    /// Return whether one empty navigation response should be retried.
    ///
    /// Returns `true` when startup timing may still hide a real result, and
    /// `false` when the empty result should be treated as final.
    fn should_retry_empty_lookup(
        &self,
        started: bool,
        startup_ready_before_request: bool,
        deadline: Instant,
    ) -> bool {
        // A running progress task means the server is still doing visible work
        // for this session, so an empty navigation response is not final yet.
        // Fresh sessions stay retryable inside the same deadline even before a
        // progress token arrives because startup indexing may begin slightly later.
        // A short post-progress grace window covers the gap between visible LSP
        // work ending and the server serving the finished symbol data.
        Instant::now() < deadline
            && (started
                || !startup_ready_before_request
                || !self.active_progress_tokens.is_empty()
                || self.has_recent_progress())
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

    /// Pull fresh diagnostics for all open documents after a refresh request.
    fn flush_pending_diagnostic_refresh(
        &mut self,
        progress_sink: &mut EventSink<'_>,
    ) -> Result<(), SessionError> {
        // A refresh-triggered pull can itself prompt another refresh request, so
        // bound the loop to avoid spinning forever if a server keeps requeueing
        // refresh work faster than diagnostics can settle. `remaining_passes`
        // caps the drain so one refresh burst cannot monopolize idle polling.
        let mut remaining_passes = 4;
        while self.pending_diagnostic_refresh && remaining_passes > 0 {
            self.pending_diagnostic_refresh = false;
            remaining_passes -= 1;
            // Refresh requests apply to every tracked document, so capture the
            // current editor versions before issuing any nested LSP requests.
            let documents = self
                .documents
                .iter()
                .map(|(path, state)| (path.clone(), state.editor_version))
                .collect::<Vec<_>>();
            for (file_path, version) in documents {
                self.request_document_diagnostics(&file_path, version, progress_sink)?;
            }
        }
        Ok(())
    }

    /// Reply to one server-initiated request with a best-effort success payload.
    fn reply_to_server_request(
        &mut self,
        id: u64,
        method: &str,
        params: Option<&json::JsonValue>,
    ) -> Result<(), SessionError> {
        if method == "workspace/applyEdit" {
            self.pending_apply_edit =
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
        &mut self,
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
        &mut self,
        file_path: &Path,
        report: &DocumentDiagnosticReport,
    ) {
        if let Some(state) = self.documents.get_mut(file_path)
            && report.result_id.is_some()
        {
            state.diagnostic_result_id = report.result_id.clone();
        }
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

    /// Return whether one recent progress event should still keep retries alive.
    ///
    /// Returns `true` while the session remains inside the short grace window
    /// after the latest progress event, and `false` once that window expires.
    fn has_recent_progress(&self) -> bool {
        self.recent_progress_deadline
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
    fn take_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        // Requests are serialized through the session mutex, so wrapping back to
        // `1` after `u64::MAX` cannot collide with an in-flight request id.
        self.next_request_id = if id == u64::MAX { 1 } else { id + 1 };
        id
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
    /// Whether this message indicates the server is ready for follow-up work.
    ready_signal: bool,
    /// Matched response state for a specific awaited request, if any.
    response: ProcessedResponse,
}

/// Response state produced after one server message is processed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum ProcessedResponse {
    /// This message was not the awaited response.
    #[default]
    None,
    /// This message matched the awaited response id and may carry a JSON result.
    Matched(Option<json::JsonValue>),
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

/// Convert one `Duration` into the bounded millisecond timeout accepted by `poll`.
fn poll_timeout_ms(timeout: Duration) -> i32 {
    timeout
        .as_millis()
        .min(i32::MAX as u128)
        .try_into()
        .unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::project::ProjectRootKind;
    use crate::lsp::server::RUST_ANALYZER;

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

    /// Confirm that request ids advance monotonically across one session.
    #[test]
    fn test_take_request_id_advances_monotonically() {
        let mut session = LspSession::new(test_workspace(), &RUST_ANALYZER);

        assert_eq!(session.take_request_id(), 1);
        assert_eq!(session.take_request_id(), 2);
    }

    /// Confirm stale sync work cannot move the tracked document version backward.
    #[test]
    fn test_should_skip_document_sync_for_stale_version() {
        let mut session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session.documents.insert(
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

    /// Confirm repeated syncs for one editor version still advance the LSP version.
    #[test]
    fn test_next_document_protocol_version_advances_for_repeat_syncs() {
        let mut session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session.documents.insert(
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

    /// Confirm empty navigation retries only stay enabled during startup races.
    #[test]
    fn test_should_retry_empty_lookup_only_during_startup_window() {
        let mut session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let deadline = Instant::now() + Duration::from_secs(1);

        assert!(session.should_retry_empty_lookup(true, true, deadline));
        assert!(session.should_retry_empty_lookup(false, false, deadline));
        assert!(!session.should_retry_empty_lookup(false, true, deadline));

        session
            .active_progress_tokens
            .insert("cargo-index".to_string());
        assert!(session.should_retry_empty_lookup(false, true, deadline));

        session.active_progress_tokens.clear();
        session.recent_progress_deadline = Some(Instant::now() + Duration::from_millis(250));
        assert!(session.should_retry_empty_lookup(false, true, deadline));
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
        let mut session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session.documents.insert(
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
        let mut session = LspSession::new(test_workspace(), &RUST_ANALYZER);
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session.documents.insert(
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
                .documents
                .get(&file_path)
                .and_then(|state| state.diagnostic_result_id.as_deref()),
            Some("diag-1")
        );
    }

    /// Confirm rename waits for the workspace graph to include cross-file references.
    #[test]
    fn test_lookup_rename_returns_workspace_edit() {
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
        let mut session = LspSession::new(
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
