//! LSP integration for project-scoped editor features.

pub(crate) mod configuration;
pub(crate) mod diagnostics;
pub(crate) mod manager;
pub(crate) mod progress;
pub(crate) mod project;
pub(crate) mod protocol;
pub(crate) mod server;
pub(crate) mod session;
#[cfg(test)]
pub(crate) mod test_servers;

pub(crate) use diagnostics::{LspDiagnostic, LspDiagnosticSeverity, LspFileDiagnostics};
pub(crate) use manager::{
    CodeActionLookupOutcome, CodeActionLookupResult, CodeActionRequestSnapshot,
    CompletionLookupOutcome, CompletionLookupResult, CompletionRequestSnapshot,
    DocumentSaveSnapshot, DocumentSyncOutcome, DocumentSyncSnapshot, HoverLookupOutcome,
    HoverLookupResult, HoverRequestSnapshot, LspManager, NavigationKind, NavigationLookupOutcome,
    NavigationLookupResult, NavigationRequestSnapshot, NavigationTarget, RenameLookupOutcome,
    RenameLookupResult, RenameRequestSnapshot, SignatureHelpLookupOutcome,
    SignatureHelpLookupResult, SignatureHelpRequestSnapshot,
};
pub(crate) use protocol::LspCodeAction;
