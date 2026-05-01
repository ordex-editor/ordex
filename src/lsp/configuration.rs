//! Server-specific configuration payloads used during initialize and configuration requests.

use super::server::LspServerId;
use json::{JsonValue, object};

/// Build one best-effort configuration response payload for `workspace/configuration`.
pub(crate) fn workspace_configuration_result(params: Option<&JsonValue>) -> JsonValue {
    JsonValue::Array(
        params
            .map(|params| {
                params["items"]
                    .members()
                    .map(|item| workspace_configuration_value(item["section"].as_str()))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// Attach any built-in server-specific initialization options to `params`.
pub(crate) fn apply_initialization_options(params: &mut JsonValue, server_id: LspServerId) {
    if let Some(options) = initialization_options(server_id) {
        params["initializationOptions"] = options;
    }
}

/// Return the built-in initialization options for `server_id`, if Ordex has any.
fn initialization_options(server_id: LspServerId) -> Option<JsonValue> {
    match server_id {
        LspServerId::RustAnalyzer => Some(rust_analyzer_configuration()),
        _ => None,
    }
}

/// Return one configuration payload for the requested workspace section.
fn workspace_configuration_value(section: Option<&str>) -> JsonValue {
    let Some(section) = section else {
        return JsonValue::Null;
    };
    if !section.starts_with("rust-analyzer") {
        return JsonValue::Null;
    }
    let mut value = rust_analyzer_configuration();
    // Nested section requests should receive the matching subtree so the server
    // sees the same values through both top-level and subsection lookups.
    for segment in section.split('.').skip(1) {
        value = value[segment].clone();
        if value.is_null() {
            return JsonValue::Null;
        }
    }
    value
}

/// Return the built-in configuration payload used for save-driven Rust diagnostics.
fn rust_analyzer_configuration() -> JsonValue {
    object! {
        checkOnSave: true,
        check: {
            command: "check",
        }
    }
}
