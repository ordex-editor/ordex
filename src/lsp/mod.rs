//! LSP integration for project-scoped editor features.

pub(crate) mod manager;
pub(crate) mod progress;
pub(crate) mod project;
pub(crate) mod protocol;
pub(crate) mod session;

pub(crate) use manager::{
    DocumentSyncOutcome, DocumentSyncSnapshot, HoverLookupOutcome, HoverLookupResult,
    HoverRequestSnapshot, LspManager, NavigationKind, NavigationLookupOutcome,
    NavigationLookupResult, NavigationRequestSnapshot, NavigationTarget,
};
