//! Shared `rust-analyzer` process sessions reused across requests in one workspace.

use super::project::ProjectWorkspace;
use super::protocol::{
    LspLocation, LspPosition, LspTextChange, ProtocolError, ServerMessage, TextDocumentSyncKind,
    definition_request, did_change_notification, did_open_notification, exit_notification,
    file_uri_to_path, initialize_request, initialized_notification, parse_definition_result,
    parse_text_document_sync_kind, read_message, server_request_response, server_request_result,
    shutdown_request, write_message,
};
use crate::unsafe_io::poll_fd;
use ropey::Rope;
use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// One synced document tracked by a shared rust-analyzer session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDocumentState {
    /// Most recent editor-owned document version accepted by the session.
    pub(crate) editor_version: i32,
    /// Most recent LSP protocol version sent to the server for this document.
    pub(crate) protocol_version: i32,
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

/// Input needed to execute one definition lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionLookupRequest {
    /// Document snapshot that must be visible to the server before lookup.
    pub(crate) document: DocumentSyncRequest,
    /// Whether the editor still has unsaved buffer edits for this snapshot.
    pub(crate) force_full_sync: bool,
    /// Zero-based lookup position in LSP coordinates.
    pub(crate) position: LspPosition,
}

/// One normalized definition location returned from rust-analyzer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDefinitionTarget {
    /// Canonical filesystem path for the resolved target.
    pub(crate) path: PathBuf,
    /// Zero-based line index.
    pub(crate) line: usize,
    /// Zero-based UTF-16 code-unit column.
    pub(crate) character: usize,
}

/// Failure returned while starting or querying one rust-analyzer session.
#[derive(Debug)]
pub(crate) enum SessionError {
    Spawn(io::Error),
    MissingStdin,
    MissingStdout,
    Protocol(ProtocolError),
    Server(String),
}

impl fmt::Display for SessionError {
    /// Format one session failure for status messages and tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "failed to start rust-analyzer: {error}"),
            Self::MissingStdin => write!(f, "rust-analyzer did not expose stdin"),
            Self::MissingStdout => write!(f, "rust-analyzer did not expose stdout"),
            Self::Protocol(error) => write!(f, "{error}"),
            Self::Server(error) => write!(f, "{error}"),
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
    server_command: PathBuf,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    next_request_id: u64,
    documents: HashMap<PathBuf, SessionDocumentState>,
    text_document_sync: TextDocumentSyncKind,
    startup_ready: bool,
}

impl LspSession {
    const STARTUP_READY_TIMEOUT: Duration = Duration::from_secs(2);
    const LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(150);
    const LOOKUP_RETRY_TIMEOUT: Duration = Duration::from_secs(7);

    /// Create one lazily-started `rust-analyzer` session for `workspace`.
    pub(crate) fn new(workspace: ProjectWorkspace, server_command: PathBuf) -> Self {
        Self {
            workspace,
            server_command,
            child: None,
            stdin: None,
            stdout: None,
            next_request_id: 1,
            documents: HashMap::new(),
            text_document_sync: TextDocumentSyncKind::Full,
            startup_ready: false,
        }
    }

    /// Synchronize one document snapshot into the running language server.
    pub(crate) fn sync_document(
        &mut self,
        request: &DocumentSyncRequest,
    ) -> Result<(), SessionError> {
        self.ensure_started()?;
        self.apply_document_sync(request)?;
        Ok(())
    }

    /// Execute one definition lookup against the running language server.
    pub(crate) fn lookup_definition(
        &mut self,
        request: &DefinitionLookupRequest,
    ) -> Result<Vec<SessionDefinitionTarget>, SessionError> {
        let started = self.ensure_started()?;
        if request.force_full_sync {
            // Unsaved buffers can race with the proactive sync worker, so resend
            // a whole-document snapshot immediately before the lookup.
            self.force_full_document_sync(&request.document)?;
            self.await_startup_ready(Self::LOOKUP_RETRY_DELAY)?;
        } else {
            self.apply_document_sync(&request.document)?;
        }
        let deadline = Instant::now() + Self::LOOKUP_RETRY_TIMEOUT;
        let mut attempt = 0usize;
        let mut forced_full_sync = request.force_full_sync;

        loop {
            let startup_ready_before_request = self.startup_ready;
            if !startup_ready_before_request {
                self.await_startup_ready(Self::STARTUP_READY_TIMEOUT)?;
            }

            match self.lookup_definition_once(request) {
                Ok(locations) if !locations.is_empty() => {
                    return locations
                        .into_iter()
                        .map(|location| self.normalize_location(location))
                        .collect();
                }
                Ok(_)
                    if self.should_retry_empty_definition_lookup(
                        started,
                        startup_ready_before_request,
                        attempt,
                        deadline,
                    ) =>
                {
                    attempt += 1;
                    self.await_startup_ready(Self::LOOKUP_RETRY_DELAY)?;
                }
                Ok(locations) => {
                    return locations
                        .into_iter()
                        .map(|location| self.normalize_location(location))
                        .collect();
                }
                Err(SessionError::Server(error))
                    if self.should_retry_content_modified(&error, deadline) =>
                {
                    attempt += 1;
                    if !forced_full_sync {
                        // Retry one content-modified response with a whole-document
                        // sync, then let rust-analyzer settle before asking again.
                        self.force_full_document_sync(&request.document)?;
                        forced_full_sync = true;
                    }
                    self.await_startup_ready(Self::STARTUP_READY_TIMEOUT)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Shut down the child process if it was started.
    pub(crate) fn shutdown(&mut self) {
        if self.child.is_none() {
            return;
        }
        // Ask the server to shut down cleanly first so it can flush any in-flight
        // responses and exit on its own before Ordex escalates to termination.
        let request_id = self.take_request_id();
        let _ = self.write_payload(&shutdown_request(request_id));
        let _ = self.read_response(request_id);
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
        self.startup_ready = false;
    }

    /// Start the language server and complete the initialize handshake when needed.
    ///
    /// Returns `Ok(true)` when this call spawned a fresh child process, and
    /// `Ok(false)` when an existing child was already running.
    fn ensure_started(&mut self) -> Result<bool, SessionError> {
        if self.child.is_some() {
            return Ok(false);
        }
        let mut command = Command::new(&self.server_command);
        command
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
        let result = self.read_response(request_id)?;
        self.text_document_sync =
            parse_text_document_sync_kind(result.as_ref()).map_err(SessionError::Protocol)?;
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
        let payload = if self.documents.contains_key(&request.file_path) {
            // Once the document is open, prefer the negotiated sync mode but
            // keep a whole-document fallback for stale or empty edit queues.
            self.change_notification(request, protocol_version, &text)
        } else {
            did_open_notification(&request.file_path, protocol_version, &text)
        };
        self.write_payload(&payload)?;
        self.documents.insert(
            request.file_path.clone(),
            SessionDocumentState {
                editor_version: request.version,
                protocol_version,
            },
        );
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
        let payload = if self.documents.contains_key(&request.file_path) {
            did_change_notification(
                &request.file_path,
                protocol_version,
                &[LspTextChange { range: None, text }],
            )
        } else {
            did_open_notification(&request.file_path, protocol_version, &text)
        };
        self.write_payload(&payload)?;
        self.documents.insert(
            request.file_path.clone(),
            SessionDocumentState {
                editor_version: request.version,
                protocol_version,
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
        let changes = if self.text_document_sync == TextDocumentSyncKind::Incremental
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

    /// Allocate the next LSP protocol version for one document path.
    fn next_document_protocol_version(&self, file_path: &Path, request_version: i32) -> i32 {
        self.documents
            .get(file_path)
            .map(|previous| previous.protocol_version.saturating_add(1))
            .unwrap_or(request_version.max(1))
    }

    /// Wait for the server to emit post-startup traffic before the first lookup.
    fn await_startup_ready(&mut self, timeout: Duration) -> Result<(), SessionError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Some(message) = self.read_message_with_timeout(remaining)? else {
                return Ok(());
            };
            match message {
                ServerMessage::Request { id, method, params } => {
                    self.reply_to_server_request(id, &method, params.as_ref())?;
                }
                ServerMessage::Notification { .. } | ServerMessage::Response { .. } => {
                    self.startup_ready = true;
                    return Ok(());
                }
            }
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
    fn read_response(&mut self, request_id: u64) -> Result<Option<json::JsonValue>, SessionError> {
        loop {
            let stdout = self.stdout.as_mut().ok_or(SessionError::MissingStdout)?;
            match read_message(stdout)? {
                ServerMessage::Request { id, method, params } => {
                    self.reply_to_server_request(id, &method, params.as_ref())?;
                }
                ServerMessage::Notification { .. } => {
                    self.startup_ready = true;
                }
                ServerMessage::Response { id, result, error } if id == request_id => {
                    if let Some(error) = error {
                        return Err(SessionError::Server(error));
                    }
                    self.startup_ready = true;
                    return Ok(result);
                }
                ServerMessage::Response { .. } => {
                    self.startup_ready = true;
                }
            }
        }
    }

    /// Execute one definition request after the document snapshot is already synced.
    fn lookup_definition_once(
        &mut self,
        request: &DefinitionLookupRequest,
    ) -> Result<Vec<LspLocation>, SessionError> {
        let request_id = self.take_request_id();
        self.write_payload(&definition_request(
            request_id,
            &request.document.file_path,
            request.position,
        ))?;
        let result = self.read_response(request_id)?;
        parse_definition_result(result.as_ref()).map_err(SessionError::Protocol)
    }

    /// Return whether one empty definition response should be retried.
    ///
    /// Returns `true` when startup timing may still hide a real definition, and
    /// `false` when the empty result should be treated as final.
    fn should_retry_empty_definition_lookup(
        &self,
        started: bool,
        startup_ready_before_request: bool,
        attempt: usize,
        deadline: Instant,
    ) -> bool {
        // Fresh sessions are the flaky case in CI, so keep polling briefly even
        // after the first startup traffic arrives instead of trusting a single
        // early notification as proof that indexing already finished.
        (started || !startup_ready_before_request) && attempt < 16 && Instant::now() < deadline
    }

    /// Return whether one server error is transient enough to retry.
    ///
    /// Returns `true` when the error looks like rust-analyzer's temporary
    /// `ContentModified` failure and the retry window is still open, and `false`
    /// for permanent failures.
    fn should_retry_content_modified(&self, error: &str, deadline: Instant) -> bool {
        Instant::now() < deadline && error.to_ascii_lowercase().contains("content modified")
    }

    /// Reply to one server-initiated request with a best-effort success payload.
    fn reply_to_server_request(
        &mut self,
        id: u64,
        method: &str,
        params: Option<&json::JsonValue>,
    ) -> Result<(), SessionError> {
        let result = server_request_result(method, params);
        self.write_payload(&server_request_response(id, result))
    }

    /// Convert one protocol location into an editor-facing path and position.
    fn normalize_location(
        &self,
        location: LspLocation,
    ) -> Result<SessionDefinitionTarget, SessionError> {
        Ok(SessionDefinitionTarget {
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

    /// Build one reusable workspace value for session unit tests.
    fn test_workspace() -> ProjectWorkspace {
        ProjectWorkspace {
            root_path: PathBuf::from("/tmp/workspace"),
            kind: crate::lsp::project::ProjectKind::CargoWorkspace,
            manifest_path: PathBuf::from("/tmp/workspace/Cargo.toml"),
        }
    }

    /// Confirm that request ids advance monotonically across one session.
    #[test]
    fn test_take_request_id_advances_monotonically() {
        let mut session = LspSession::new(test_workspace(), PathBuf::from("rust-analyzer"));

        assert_eq!(session.take_request_id(), 1);
        assert_eq!(session.take_request_id(), 2);
    }

    /// Confirm stale sync work cannot move the tracked document version backward.
    #[test]
    fn test_should_skip_document_sync_for_stale_version() {
        let mut session = LspSession::new(test_workspace(), PathBuf::from("rust-analyzer"));
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 4,
                protocol_version: 7,
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
        let mut session = LspSession::new(test_workspace(), PathBuf::from("rust-analyzer"));
        let file_path = PathBuf::from("/tmp/workspace/src/main.rs");
        session.documents.insert(
            file_path.clone(),
            SessionDocumentState {
                editor_version: 4,
                protocol_version: 7,
            },
        );

        assert_eq!(session.next_document_protocol_version(&file_path, 4), 8);
        assert_eq!(session.next_document_protocol_version(&file_path, 5), 8);
        assert_eq!(
            session.next_document_protocol_version(Path::new("/tmp/workspace/src/lib.rs"), 0),
            1
        );
    }
}
