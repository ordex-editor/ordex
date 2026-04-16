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
    DocumentSaveSnapshot, DocumentSyncOutcome, DocumentSyncSnapshot, HoverLookupOutcome,
    HoverLookupResult, HoverRequestSnapshot, LspManager, NavigationKind, NavigationLookupOutcome,
    NavigationLookupResult, NavigationRequestSnapshot, NavigationTarget, RenameLookupOutcome,
    RenameLookupResult, RenameRequestSnapshot,
};
