//! App-owned orchestration for background LSP definition lookups.

use super::project::{WorkspaceError, detect_workspace_for_file};
use super::protocol::LspPosition;
use super::session::{DefinitionLookupRequest, LspSession, SessionDefinitionTarget, SessionError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

/// One jump target shown to the editor and picker UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionTarget {
    pub(crate) file_path: PathBuf,
    pub(crate) line: usize,
    pub(crate) character: usize,
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
    pub(crate) buffer_id: usize,
    pub(crate) lookup_token: u64,
    pub(crate) document_version: i32,
    pub(crate) outcome: DefinitionLookupOutcome,
}

/// Immutable snapshot of the active buffer used for a background lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionRequestSnapshot {
    pub(crate) buffer_id: usize,
    pub(crate) lookup_token: u64,
    pub(crate) document_version: i32,
    pub(crate) file_path: PathBuf,
    pub(crate) text: String,
    pub(crate) line: usize,
    pub(crate) character: usize,
}

/// One app-owned registry of reusable workspace-scoped rust-analyzer sessions.
pub(crate) struct LspManager {
    sessions: HashMap<PathBuf, Arc<Mutex<LspSession>>>,
    server_command: PathBuf,
    sender: Sender<DefinitionLookupResult>,
    receiver: Receiver<DefinitionLookupResult>,
    pending_requests: usize,
}

impl LspManager {
    /// Create one manager using `ORDEX_RUST_ANALYZER` or the default rust-analyzer command.
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sessions: HashMap::new(),
            server_command: std::env::var_os("ORDEX_RUST_ANALYZER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("rust-analyzer")),
            sender,
            receiver,
            pending_requests: 0,
        }
    }

    /// Start one background definition lookup from the supplied editor snapshot.
    pub(crate) fn request_definition(&mut self, snapshot: DefinitionRequestSnapshot) {
        self.pending_requests += 1;
        let sender = self.sender.clone();
        let server_command = self.server_command.clone();
        let session = match self.session_for_path(&snapshot.file_path, &server_command) {
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
                file_path: snapshot.file_path.clone(),
                version: snapshot.document_version,
                text: snapshot.text.clone(),
                position: LspPosition {
                    line: snapshot.line,
                    character: snapshot.character,
                },
            };
            let outcome = match session.lock() {
                Ok(mut session) => match session.lookup_definition(&request) {
                    Ok(targets) => targets_to_outcome(targets),
                    Err(SessionError::Spawn(error)) => {
                        DefinitionLookupOutcome::Unavailable(error.to_string())
                    }
                    Err(SessionError::MissingStdin | SessionError::MissingStdout) => {
                        DefinitionLookupOutcome::Unavailable(
                            "rust-analyzer did not expose its stdio transport".to_string(),
                        )
                    }
                    Err(SessionError::Protocol(error)) => {
                        DefinitionLookupOutcome::Error(error.to_string())
                    }
                    Err(SessionError::Server(error)) => DefinitionLookupOutcome::Error(error),
                },
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

    /// Drain any completed background lookups and apply them to `editor`.
    pub(crate) fn poll(&mut self, editor: &mut crate::editor_state::EditorState) -> bool {
        let mut changed = false;
        loop {
            match self.receiver.try_recv() {
                Ok(result) => {
                    self.pending_requests = self.pending_requests.saturating_sub(1);
                    changed |= editor.apply_definition_lookup_result(result);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending_requests = 0;
                    break;
                }
            }
        }
        changed
    }

    /// Return whether definition lookups are still running in the background.
    pub(crate) fn has_pending_work(&self) -> bool {
        self.pending_requests > 0
    }

    /// Resolve or create the reusable session for one file path.
    fn session_for_path(
        &mut self,
        file_path: &Path,
        server_command: &Path,
    ) -> Result<Arc<Mutex<LspSession>>, WorkspaceError> {
        let workspace = detect_workspace_for_file(file_path)?;
        if let Some(session) = self.sessions.get(&workspace.root_path) {
            return Ok(Arc::clone(session));
        }
        let root_path = workspace.root_path.clone();
        let session = Arc::new(Mutex::new(LspSession::new(
            workspace,
            server_command.to_path_buf(),
        )));
        self.sessions.insert(root_path, Arc::clone(&session));
        Ok(session)
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
    if targets.is_empty() {
        return DefinitionLookupOutcome::NotFound;
    }
    let targets = targets
        .into_iter()
        .map(|target| DefinitionTarget {
            display_label: format!(
                "{}:{}:{}",
                target.path.display(),
                target.line + 1,
                target.character + 1
            ),
            file_path: target.path,
            line: target.line,
            character: target.character,
        })
        .collect::<Vec<_>>();
    if targets.len() == 1 {
        DefinitionLookupOutcome::Single(targets[0].clone())
    } else {
        DefinitionLookupOutcome::Multiple(targets)
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
        let server_command = PathBuf::from("fake-rust-analyzer");
        let workspace_one_main = fixture_path("tests/fixtures/lsp/workspace_one/src/main.rs");
        let workspace_one_lib = fixture_path("tests/fixtures/lsp/workspace_one/src/lib.rs");
        let workspace_two_main = fixture_path("tests/fixtures/lsp/workspace_two/src/main.rs");

        // Opening two files from the same workspace should reuse the exact same session.
        let first = manager
            .session_for_path(&workspace_one_main, &server_command)
            .expect("create first workspace session");
        let second = manager
            .session_for_path(&workspace_one_lib, &server_command)
            .expect("reuse first workspace session");
        let third = manager
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
}
