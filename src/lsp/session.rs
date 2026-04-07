//! Shared rust-analyzer process sessions reused across requests in one workspace.

use super::project::ProjectWorkspace;
use super::protocol::{
    LspLocation, LspPosition, ProtocolError, ServerMessage, definition_request,
    did_change_notification, did_open_notification, exit_notification, file_uri_to_path,
    initialize_request, initialized_notification, parse_definition_result, read_message,
    shutdown_request, write_message,
};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// One synced document tracked by a shared rust-analyzer session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDocumentState {
    pub(crate) version: i32,
    pub(crate) is_open: bool,
}

/// Input needed to execute one definition lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionLookupRequest {
    pub(crate) file_path: PathBuf,
    pub(crate) version: i32,
    pub(crate) text: String,
    pub(crate) position: LspPosition,
}

/// One normalized definition location returned from rust-analyzer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDefinitionTarget {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
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

/// One reusable rust-analyzer process keyed by workspace root.
#[derive(Debug)]
pub(crate) struct LspSession {
    workspace: ProjectWorkspace,
    server_command: PathBuf,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    next_request_id: i32,
    documents: HashMap<PathBuf, SessionDocumentState>,
}

impl LspSession {
    /// Create one lazily-started rust-analyzer session for `workspace`.
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

    /// Execute one definition lookup against rust-analyzer.
    pub(crate) fn lookup_definition(
        &mut self,
        request: &DefinitionLookupRequest,
    ) -> Result<Vec<SessionDefinitionTarget>, SessionError> {
        self.ensure_started()?;
        self.sync_document(request)?;
        let request_id = self.take_request_id();
        self.write_payload(&definition_request(
            request_id,
            &request.file_path,
            request.position,
        ))?;
        let result = self.read_response(request_id)?;
        let locations = parse_definition_result(result.as_ref()).map_err(SessionError::Protocol)?;
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
        let request_id = self.take_request_id();
        let _ = self.write_payload(&shutdown_request(request_id));
        let _ = self.read_response(request_id);
        let _ = self.write_payload(&exit_notification());
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.stdout = None;
        self.documents.clear();
    }

    /// Start rust-analyzer and complete the initialize handshake when needed.
    fn ensure_started(&mut self) -> Result<(), SessionError> {
        if self.child.is_some() {
            return Ok(());
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
        Ok(())
    }

    /// Send `didOpen` or full-text `didChange` so rust-analyzer sees the current buffer snapshot.
    fn sync_document(&mut self, request: &DefinitionLookupRequest) -> Result<(), SessionError> {
        let state = self.documents.get(&request.file_path).cloned();
        let payload = if let Some(previous) = state {
            if previous.version == request.version {
                return Ok(());
            }
            did_change_notification(&request.file_path, request.version, &request.text)
        } else {
            did_open_notification(&request.file_path, request.version, &request.text)
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

    /// Send one JSON-RPC payload to the child process.
    fn write_payload(&mut self, payload: &json::JsonValue) -> Result<(), SessionError> {
        let stdin = self.stdin.as_mut().ok_or(SessionError::MissingStdin)?;
        write_message(stdin, payload).map_err(SessionError::Protocol)
    }

    /// Read responses until the requested id arrives, skipping notifications.
    fn read_response(&mut self, request_id: i32) -> Result<Option<json::JsonValue>, SessionError> {
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
    fn take_request_id(&mut self) -> i32 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}

impl Drop for LspSession {
    /// Ensure child processes do not outlive the session object.
    fn drop(&mut self) {
        self.shutdown();
    }
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
