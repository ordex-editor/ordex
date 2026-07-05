//! Built-in language-server descriptors and routing rules.

mod catalog;
mod routes;

#[cfg(test)]
pub(crate) use catalog::{
    CLANGD, GOPLS, HTML_LANGUAGE_SERVER, JSON_LANGUAGE_SERVER, PHPACTOR, PYLSP, RUFF,
    RUST_ANALYZER, TY, TYPESCRIPT_LANGUAGE_SERVER, YAML_LANGUAGE_SERVER,
};
pub(crate) use catalog::{LspServerDescriptor, LspServerId, ProjectDetection};
pub(crate) use routes::{
    LspRouteKind, is_known_server_display_name, language_for_path, route_servers,
    supported_project_description,
};
