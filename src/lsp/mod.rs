//! LSP integration for project-scoped code navigation.

pub(crate) mod manager;
pub(crate) mod project;
pub(crate) mod protocol;
pub(crate) mod session;

pub(crate) use manager::{
    DefinitionLookupOutcome, DefinitionLookupResult, DefinitionTarget, DocumentSyncOutcome,
    DocumentSyncSnapshot, LspManager,
};
