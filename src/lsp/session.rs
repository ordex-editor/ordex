//! Shared `rust-analyzer` process sessions reused across requests in one workspace.

use super::project::ProjectWorkspace;
use super::protocol::{
    LspLocation, LspPosition, ProtocolError, ServerMessage, definition_request,
    did_change_notification, did_open_notification, exit_notification, file_uri_to_path,
    initialize_request, initialized_notification, parse_definition_result, read_message,
    shutdown_request, write_message,
};
use crate::unsafe_io::poll_fd;
use ropey::Rope;
use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// One synced document tracked by a shared rust-analyzer session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDocumentState {
    pub(crate) version: i32,
    pub(crate) is_open: bool,
}

/// Input needed to execute one definition lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionLookupRequest {
    /// Canonical filesystem path for the source document.
    pub(crate) file_path: PathBuf,
    /// Monotonic document version sent with this snapshot.
    pub(crate) version: i32,
    /// Cheaply cloned document snapshot stored as a rope.
    pub(crate) text: Rope,
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
}

impl LspSession {
    const STARTUP_READY_TIMEOUT: Duration = Duration::from_secs(2);

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
        }
    }

    /// Execute one definition lookup against the running language server.
    pub(crate) fn lookup_definition(
        &mut self,
        request: &DefinitionLookupRequest,
    ) -> Result<Vec<SessionDefinitionTarget>, SessionError> {
        let started_now = self.ensure_started()?;
        self.sync_document(request)?;
        if started_now {
            self.await_startup_ready(Self::STARTUP_READY_TIMEOUT)?;
        }
        let request_id = self.take_request_id();
        self.write_payload(&definition_request(
            request_id,
            &request.file_path,
            request.position,
        ))?;
        let result = self.read_response(request_id)?;
        let locations = parse_definition_result(result.as_ref()).map_err(SessionError::Protocol)?;
        if locations.is_empty() && started_now {
            return Err(SessionError::Server(
                "language server is still indexing this workspace; try again shortly".to_string(),
            ));
        }
        locations
            .into_iter()
            .map(|location| self.normalize_location(location))
            .collect()
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
        let _ = self.read_response(request_id)?;
        self.write_payload(&initialized_notification())?;
        Ok(true)
    }

    /// Send `didOpen` or full-text `didChange` so the server sees the current buffer snapshot.
    fn sync_document(&mut self, request: &DefinitionLookupRequest) -> Result<(), SessionError> {
        let text = request.text.to_string();
        let state = self.documents.get(&request.file_path).cloned();
        let payload = if let Some(previous) = state {
            if previous.version == request.version {
                return Ok(());
            }
            did_change_notification(&request.file_path, request.version, &text)
        } else {
            did_open_notification(&request.file_path, request.version, &text)
        };
        self.write_payload(&payload)?;
        self.documents.insert(
            request.file_path.clone(),
            SessionDocumentState {
                version: request.version,
                is_open: true,
            },
        );
        Ok(())
    }

    /// Wait for the server to emit post-startup traffic before the first lookup.
    fn await_startup_ready(&mut self, timeout: Duration) -> Result<(), SessionError> {
        // rust-analyzer typically emits diagnostics or a refresh request once
        // it has started processing the opened document. Waiting for that
        // traffic avoids an unconditional sleep on every fresh session start.
        if let Some(ServerMessage::Notification { .. } | ServerMessage::Response { .. }) =
            self.read_message_with_timeout(timeout)?
        {}
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
                ServerMessage::Notification { .. } => continue,
                ServerMessage::Response { id, result, error } if id == request_id => {
                    if let Some(error) = error {
                        return Err(SessionError::Server(error));
                    }
                    return Ok(result);
                }
                ServerMessage::Response { .. } => continue,
            }
        }
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

    /// Confirm that request ids advance monotonically across one session.
    #[test]
    fn test_take_request_id_advances_monotonically() {
        let workspace = ProjectWorkspace {
            root_path: PathBuf::from("/tmp/workspace"),
            kind: crate::lsp::project::ProjectKind::CargoWorkspace,
            manifest_path: PathBuf::from("/tmp/workspace/Cargo.toml"),
        };
        let mut session = LspSession::new(workspace, PathBuf::from("rust-analyzer"));

        assert_eq!(session.take_request_id(), 1);
        assert_eq!(session.take_request_id(), 2);
    }
}
