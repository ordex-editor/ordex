//! LSP-specific configuration loading from the dedicated `lsp.cfg` file.

use crate::config::{WarningCode, WarningEvent};
use crate::lsp::server::is_known_server_display_name;
use crate::toml_like_parser::{
    ParsedDocument, ParsedItem, ParsedSection, ParsedValue, ParserDiagnosticKind, parse_reader,
};
use json::{JsonValue, object};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

/// Include-path entry with source location metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IncludePathEntry {
    path: String,
    line: usize,
    line_content: String,
}

/// Parsed and merged LSP settings grouped by server display name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LspConfigSettings {
    pub(crate) server_settings: HashMap<String, JsonValue>,
}

/// Outcome of validating one parsed `lsp.cfg` document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ValidationReport {
    settings: LspConfigSettings,
    include_paths: Vec<IncludePathEntry>,
    warnings: Vec<WarningEvent>,
}

/// Final load result for one startup or reload attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LspConfigLoadOutcome {
    pub(crate) settings: LspConfigSettings,
    pub(crate) warnings: Vec<WarningEvent>,
}

/// Load one LSP config file, process includes, and merge valid settings.
pub(crate) fn load_lsp_config(path: &Path) -> LspConfigLoadOutcome {
    let main_doc = match parse_lsp_config_file(path) {
        Ok(document) => document,
        Err(error) => {
            return LspConfigLoadOutcome {
                settings: LspConfigSettings::default(),
                warnings: vec![WarningEvent::new(
                    WarningCode::InvalidSection,
                    format!(
                        "Could not read LSP config file `{}`; built-in defaults used ({error})",
                        path.display()
                    ),
                    path,
                    None,
                    None,
                )],
            };
        }
    };

    let mut aggregate = validate_document(&main_doc);
    let include_paths = aggregate.include_paths.clone();
    // Includes are applied after the main file so they can intentionally override
    // previously seen server settings with project- or machine-specific values.
    for include in include_paths {
        let include_path = resolve_include_path(path, &include.path);
        match parse_lsp_config_file(&include_path) {
            Ok(include_doc) => {
                let include_report = validate_document(&include_doc);
                merge_reports(&mut aggregate, include_report);
            }
            Err(error) => aggregate.warnings.push(
                WarningEvent::new(
                    WarningCode::MissingInclude,
                    format!("Missing include `{}` ({error})", include_path.display()),
                    &include_path,
                    Some("include".to_string()),
                    None,
                )
                .with_position(include.line, None, Some(include.line_content)),
            ),
        }
    }

    LspConfigLoadOutcome {
        settings: aggregate.settings,
        warnings: aggregate.warnings,
    }
}

/// Open one LSP config file and parse it through the shared parser.
fn parse_lsp_config_file(path: &Path) -> io::Result<ParsedDocument> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    parse_reader(path, reader)
}

/// Resolve one include path relative to the main config file location.
fn resolve_include_path(base_path: &Path, include_path: &str) -> PathBuf {
    let include = PathBuf::from(include_path);
    if include.is_absolute() {
        return include;
    }
    base_path
        .parent()
        .map(|parent| parent.join(&include))
        .unwrap_or(include)
}

/// Validate one parsed document and convert its valid values into server JSON trees.
fn validate_document(doc: &ParsedDocument) -> ValidationReport {
    let mut report = ValidationReport::default();

    for diag in &doc.diagnostics {
        let code = match diag.kind {
            ParserDiagnosticKind::InvalidHeader => WarningCode::InvalidSection,
            ParserDiagnosticKind::InvalidAssignment
            | ParserDiagnosticKind::InvalidValue
            | ParserDiagnosticKind::UnterminatedString => WarningCode::InvalidValue,
        };
        report.warnings.push(
            WarningEvent::new(
                code,
                &diag.message,
                &doc.source_path,
                diag.section.clone(),
                None,
            )
            .with_position(
                diag.line,
                Some(diag.column),
                Some(diag.line_content.clone()),
            ),
        );
    }

    // Parsing is resilient, so validation keeps scanning every section to apply
    // every valid key even if unrelated lines already produced diagnostics.
    for section in &doc.sections {
        validate_section(section, &doc.source_path, &mut report);
    }
    report
}

/// Merge one validated report into an aggregate load result.
fn merge_reports(target: &mut ValidationReport, mut other: ValidationReport) {
    for (server_name, value) in other.settings.server_settings.drain() {
        let target_value = target
            .settings
            .server_settings
            .entry(server_name)
            .or_insert_with(|| object! {});
        deep_merge_json(target_value, value);
    }
    target.include_paths.append(&mut other.include_paths);
    target.warnings.append(&mut other.warnings);
}

/// Validate one parsed section and update the report with accepted values.
fn validate_section(section: &ParsedSection, source_path: &Path, report: &mut ValidationReport) {
    if section.name == "root" {
        validate_root_section(section, source_path, report);
        return;
    }
    if section.name == "include" {
        validate_include_section(section, source_path, report);
        return;
    }
    let Some(server_name) = section.name.strip_prefix("lsp.") else {
        report.warnings.push(section_warning(
            source_path,
            section,
            WarningCode::UnknownKey,
            format!("Unknown section `{}` ignored", section.name),
            None,
        ));
        return;
    };
    if server_name.is_empty() {
        report.warnings.push(section_warning(
            source_path,
            section,
            WarningCode::InvalidSection,
            "LSP section names must look like `lsp.<server-name>`".to_string(),
            None,
        ));
        return;
    }
    if !is_known_server_display_name(server_name) {
        report.warnings.push(section_warning(
            source_path,
            section,
            WarningCode::UnknownKey,
            format!("Unknown LSP server `{server_name}`; section ignored"),
            None,
        ));
        return;
    }

    // LSP values are keyed with dotted paths that map to nested JSON objects,
    // so each assignment expands just its own branch under that server name.
    for item in &section.items {
        validate_server_setting(item, server_name, source_path, report);
    }
}

/// Validate unknown top-level assignments outside named sections.
fn validate_root_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    for item in &section.items {
        report.warnings.push(
            WarningEvent::new(
                WarningCode::UnknownKey,
                "Unknown top-level setting ignored; use [lsp.<server-name>] sections",
                source_path,
                Some("root".to_string()),
                Some(item.key.clone()),
            )
            .with_position(item.line, None, Some(item.line_content.clone())),
        );
    }
}

/// Validate one include section and collect loadable include paths.
fn validate_include_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    for item in &section.items {
        match &item.value {
            ParsedValue::String(value) if !value.trim().is_empty() => {
                report.include_paths.push(IncludePathEntry {
                    path: value.clone(),
                    line: item.line,
                    line_content: item.line_content.clone(),
                });
            }
            _ => report.warnings.push(
                WarningEvent::new(
                    WarningCode::InvalidValue,
                    "Include values must be non-empty strings",
                    source_path,
                    Some(section.name.clone()),
                    Some(item.key.clone()),
                )
                .with_position(item.line, None, Some(item.line_content.clone())),
            ),
        }
    }
}

/// Validate and apply one `[lsp.<server>]` assignment into the merged JSON tree.
fn validate_server_setting(
    item: &ParsedItem,
    server_name: &str,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    let Some(path_segments) = validated_key_segments(&item.key) else {
        report.warnings.push(
            WarningEvent::new(
                WarningCode::InvalidValue,
                "LSP setting keys must use dot-separated non-empty path segments",
                source_path,
                Some(format!("lsp.{server_name}")),
                Some(item.key.clone()),
            )
            .with_position(item.line, None, Some(item.line_content.clone())),
        );
        return;
    };
    let value = parsed_value_to_json(&item.value);
    let server_root = report
        .settings
        .server_settings
        .entry(server_name.to_string())
        .or_insert_with(|| object! {});
    set_nested_value(server_root, &path_segments, value);
}

/// Validate and split a dotted setting key into path segments.
fn validated_key_segments(key: &str) -> Option<Vec<&str>> {
    let segments = key.split('.').collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }
    if segments
        .iter()
        .any(|segment| segment.trim().is_empty() || !segment.chars().all(valid_key_char))
    {
        return None;
    }
    Some(segments)
}

/// Return whether one character is valid inside one dotted LSP key segment.
fn valid_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

/// Convert one parsed scalar value into a JSON value.
fn parsed_value_to_json(value: &ParsedValue) -> JsonValue {
    match value {
        ParsedValue::String(v) => JsonValue::String(v.clone()),
        ParsedValue::StringArray(values) => {
            JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect())
        }
        ParsedValue::Integer(v) => JsonValue::Number((*v).into()),
        ParsedValue::Boolean(v) => JsonValue::Boolean(*v),
    }
}

/// Assign one nested value under `root`, creating intermediate objects as needed.
fn set_nested_value(root: &mut JsonValue, path_segments: &[&str], value: JsonValue) {
    if path_segments.is_empty() {
        *root = value;
        return;
    }

    // Dotted keys map to object nesting, so non-object intermediates are replaced
    // with objects to keep later path assignments deterministic and override-safe.
    let mut cursor = root;
    for segment in &path_segments[..path_segments.len().saturating_sub(1)] {
        if !cursor.is_object() {
            *cursor = object! {};
        }
        if cursor[*segment].is_null() || !cursor[*segment].is_object() {
            cursor[*segment] = object! {};
        }
        cursor = &mut cursor[*segment];
    }
    cursor[path_segments[path_segments.len() - 1]] = value;
}

/// Recursively merge one JSON value into another.
fn deep_merge_json(target: &mut JsonValue, incoming: JsonValue) {
    match (target, incoming) {
        (JsonValue::Object(target_obj), JsonValue::Object(incoming_obj)) => {
            // Object merges recurse per key so include files can override only
            // selected nested fields without replacing whole server trees.
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

/// Build one section-level warning anchored to header or first assignment line.
fn section_warning(
    source_path: &Path,
    section: &ParsedSection,
    code: WarningCode,
    message: String,
    key: Option<String>,
) -> WarningEvent {
    let warning = WarningEvent::new(code, message, source_path, Some(section.name.clone()), key);
    if let Some(line) = section.header_line {
        return warning.with_position(line, None, section.header_line_content.clone());
    }
    if let Some(item) = section.items.first() {
        return warning.with_position(item.line, None, Some(item.line_content.clone()));
    }
    warning
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempTree;

    /// Verify dotted keys map to nested JSON values for one known server name.
    #[test]
    fn test_load_lsp_config_parses_dotted_server_keys() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "dotted.cfg",
            r#"
[lsp.rust-analyzer]
check.command = "clippy"
checkOnSave = false
"#,
        )
        .expect("write config");
        let path = tree.path().join("dotted.cfg");

        let outcome = load_lsp_config(&path);

        assert!(outcome.warnings.is_empty());
        assert_eq!(
            outcome.settings.server_settings["rust-analyzer"]["check"]["command"].as_str(),
            Some("clippy")
        );
        assert_eq!(
            outcome.settings.server_settings["rust-analyzer"]["checkOnSave"].as_bool(),
            Some(false)
        );
    }

    /// Verify include files override main-file values for matching nested keys.
    #[test]
    fn test_load_lsp_config_include_overrides_main_values() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "include_extra.cfg",
            r#"
[lsp.rust-analyzer]
check.command = "clippy"
"#,
        )
        .expect("write include");
        tree.write_file(
            "include_main.cfg",
            r#"
[lsp.rust-analyzer]
check.command = "check"

[include]
extra = "include_extra.cfg"
"#,
        )
        .expect("write main");
        let path = tree.path().join("include_main.cfg");

        let outcome = load_lsp_config(&path);

        assert!(outcome.warnings.is_empty());
        assert_eq!(
            outcome.settings.server_settings["rust-analyzer"]["check"]["command"].as_str(),
            Some("clippy")
        );
    }

    /// Verify unknown LSP server sections are ignored with one warning.
    #[test]
    fn test_load_lsp_config_warns_for_unknown_server() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "unknown_server.cfg",
            r#"
[lsp.unknown-server]
value = true
"#,
        )
        .expect("write config");
        let path = tree.path().join("unknown_server.cfg");

        let outcome = load_lsp_config(&path);

        assert!(outcome.settings.server_settings.is_empty());
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].code, WarningCode::UnknownKey);
    }
}
