//! LSP integration for project-scoped editor features.

pub(crate) mod diagnostics;
pub(crate) mod manager;
pub(crate) mod progress;
pub(crate) mod project;
pub(crate) mod protocol;
pub(crate) mod server;
pub(crate) mod session;

pub(crate) use diagnostics::{LspDiagnostic, LspDiagnosticSeverity, LspFileDiagnostics};
pub(crate) use manager::{
    CodeActionLookupOutcome, CodeActionLookupResult, CodeActionRequestSnapshot,
    CompletionLookupOutcome, CompletionLookupResult, CompletionRequestSnapshot,
    DocumentSaveSnapshot, DocumentSyncOutcome, DocumentSyncSnapshot, HoverLookupOutcome,
    HoverLookupResult, HoverRequestSnapshot, LspManager, NavigationKind, NavigationLookupOutcome,
    NavigationLookupResult, NavigationRequestSnapshot, NavigationTarget, RenameLookupOutcome,
    RenameLookupResult, RenameRequestSnapshot,
};
pub(crate) use protocol::LspCodeAction;

#[cfg(test)]
/// Return one process-wide lock for tests that mutate PATH or depend on the real LSP binary.
pub(crate) fn lsp_test_environment_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
