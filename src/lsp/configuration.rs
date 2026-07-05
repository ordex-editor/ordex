//! Server-specific configuration payloads used during initialize and configuration requests.

use crate::lsp::user_config::LspConfigSettings;
use json::{JsonValue, object};
use std::collections::HashMap;

/// In-memory per-server configuration tree used by LSP protocol helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspConfigurationStore {
    server_settings: HashMap<String, JsonValue>,
}

impl Default for LspConfigurationStore {
    /// Build one store populated with built-in defaults only.
    fn default() -> Self {
        let mut server_settings = HashMap::new();
        server_settings.insert(
            "rust-analyzer".to_string(),
            default_rust_analyzer_configuration(),
        );
        Self { server_settings }
    }
}

impl LspConfigurationStore {
    /// Build one merged store from built-in defaults plus `lsp.cfg` overrides.
    pub(crate) fn from_user_settings(user_settings: &LspConfigSettings) -> Self {
        let mut merged = Self::default();
        for (server_name, user_value) in &user_settings.server_settings {
            let target = merged
                .server_settings
                .entry(server_name.clone())
                .or_insert_with(|| object! {});
            deep_merge_json(target, user_value.clone());
        }
        merged
    }

    /// Return one full configuration payload for `server_name`, if available.
    pub(crate) fn server_configuration(&self, server_name: &str) -> Option<JsonValue> {
        self.server_settings.get(server_name).cloned()
    }

    /// Return one nested configuration payload for `section`.
    pub(crate) fn section_configuration(
        &self,
        server_name: &str,
        section: Option<&str>,
    ) -> JsonValue {
        let Some(section_name) = section else {
            return JsonValue::Null;
        };
        if !section_name.starts_with(server_name) {
            return JsonValue::Null;
        }
        let mut value = self
            .server_configuration(server_name)
            .unwrap_or(JsonValue::Null);
        // Nested section lookups should return just the requested subtree so
        // servers can request either top-level or deep section names.
        for segment in section_name.split('.').skip(1) {
            value = value[segment].clone();
            if value.is_null() {
                return JsonValue::Null;
            }
        }
        value
    }
}

/// Build one best-effort configuration response payload for `workspace/configuration`.
pub(crate) fn workspace_configuration_result(
    configuration: &LspConfigurationStore,
    server_name: &str,
    params: Option<&JsonValue>,
) -> JsonValue {
    JsonValue::Array(
        params
            .map(|params| {
                params["items"]
                    .members()
                    .map(|item| {
                        configuration.section_configuration(server_name, item["section"].as_str())
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// Attach any server-specific initialization options to `params`.
pub(crate) fn apply_initialization_options(
    params: &mut JsonValue,
    configuration: &LspConfigurationStore,
    server_name: &str,
) {
    if let Some(options) = configuration.server_configuration(server_name) {
        params["initializationOptions"] = options;
    }
}

/// Return the built-in rust-analyzer payload used for save-driven diagnostics.
fn default_rust_analyzer_configuration() -> JsonValue {
    object! {
        checkOnSave: true,
        check: {
            command: "check",
        }
    }
}

/// Recursively merge one JSON value into another.
fn deep_merge_json(target: &mut JsonValue, incoming: JsonValue) {
    match (target, incoming) {
        (JsonValue::Object(target_obj), JsonValue::Object(incoming_obj)) => {
            // Object entries merge by key so user overrides can replace specific
            // nested values without dropping sibling built-in defaults.
            for (key, value) in incoming_obj.iter() {
                if target_obj[key].is_null() {
                    target_obj[key] = value.clone();
                } else {
                    deep_merge_json(&mut target_obj[key], value.clone());
                }
            }
        }
        (target_slot, incoming_value) => *target_slot = incoming_value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::user_config::LspConfigSettings;

    /// Verify default stores include rust-analyzer's save-diagnostic defaults.
    #[test]
    fn test_default_store_contains_rust_analyzer_defaults() {
        let store = LspConfigurationStore::default();

        assert_eq!(
            store
                .server_configuration("rust-analyzer")
                .expect("rust analyzer config")["checkOnSave"]
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            store
                .server_configuration("rust-analyzer")
                .expect("rust analyzer config")["check"]["command"]
                .as_str(),
            Some("check")
        );
    }

    /// Verify user settings override built-in nested keys while preserving siblings.
    #[test]
    fn test_from_user_settings_overrides_default_subtrees() {
        let mut user = LspConfigSettings::default();
        user.server_settings.insert(
            "rust-analyzer".to_string(),
            object! {
                check: {
                    command: "clippy",
                }
            },
        );

        let store = LspConfigurationStore::from_user_settings(&user);

        assert_eq!(
            store
                .server_configuration("rust-analyzer")
                .expect("rust analyzer config")["checkOnSave"]
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            store
                .server_configuration("rust-analyzer")
                .expect("rust analyzer config")["check"]["command"]
                .as_str(),
            Some("clippy")
        );
    }

    /// Verify section lookups return top-level and nested rust-analyzer payloads.
    #[test]
    fn test_section_configuration_supports_nested_sections() {
        let store = LspConfigurationStore::default();

        assert_eq!(
            store.section_configuration("rust-analyzer", Some("rust-analyzer"))["checkOnSave"]
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            store.section_configuration("rust-analyzer", Some("rust-analyzer.check"))["command"]
                .as_str(),
            Some("check")
        );
    }
}
