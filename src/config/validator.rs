//! Validation and normalization for parsed configuration documents.
//!
//! This module keeps validation section-scoped so valid key mappings can still
//! be applied even when other sections contain invalid values.

use crate::config::parser::{ParsedDocument, ParsedSection, ParsedValue, ParserDiagnosticKind};
use crate::config::warnings::{WarningCode, WarningEvent};
use crate::keybindings::{
    Action, KeyInput, ModeContext, parse_action, parse_key_input, parse_mode_context,
};
use std::collections::HashSet;
use std::path::Path;

/// A key binding parsed from configuration and ready to apply at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredBinding {
    pub(crate) mode: ModeContext,
    pub(crate) key: KeyInput,
    pub(crate) action: Action,
    pub(crate) source: String,
}

/// Runtime settings resolved from one or more configuration files.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ConfigSettings {
    pub(crate) scroll_margin: Option<usize>,
    pub(crate) horizontal_scroll_margin: Option<usize>,
    pub(crate) include_paths: Vec<String>,
    pub(crate) key_bindings: Vec<ConfiguredBinding>,
}

/// Validation output including resolved settings and non-fatal warnings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ValidationReport {
    pub(crate) settings: ConfigSettings,
    pub(crate) applied_sections: Vec<String>,
    pub(crate) skipped_sections: Vec<String>,
    pub(crate) defaulted_keys: Vec<String>,
    pub(crate) ignored_unknown_keys: Vec<String>,
    pub(crate) warnings: Vec<WarningEvent>,
}

/// Validate a parsed config document and collect resilient warnings.
pub(crate) fn validate_document(doc: &ParsedDocument) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut invalid_sections = HashSet::new();

    for diag in &doc.diagnostics {
        let code = match diag.kind {
            ParserDiagnosticKind::InvalidHeader => WarningCode::InvalidSection,
            ParserDiagnosticKind::InvalidAssignment
            | ParserDiagnosticKind::InvalidValue
            | ParserDiagnosticKind::UnterminatedString => WarningCode::InvalidValue,
        };
        if let Some(section) = diag.section.clone() {
            invalid_sections.insert(section.clone());
            push_unique(&mut report.skipped_sections, section.clone());
            report.warnings.push(WarningEvent::new(
                code,
                &diag.message,
                &doc.source_path,
                Some(section),
                None,
            ));
        } else {
            report.warnings.push(WarningEvent::new(
                code,
                &diag.message,
                &doc.source_path,
                None,
                None,
            ));
        }
    }

    for section in &doc.sections {
        if invalid_sections.contains(&section.name) {
            continue;
        }
        validate_section(section, &doc.source_path, &mut report);
    }

    report
}

/// Merge validation results from multiple documents (main + includes).
pub(crate) fn merge_validation_reports(target: &mut ValidationReport, mut other: ValidationReport) {
    if let Some(value) = other.settings.scroll_margin.take() {
        target.settings.scroll_margin = Some(value);
    }
    if let Some(value) = other.settings.horizontal_scroll_margin.take() {
        target.settings.horizontal_scroll_margin = Some(value);
    }
    target
        .settings
        .include_paths
        .append(&mut other.settings.include_paths);
    target
        .settings
        .key_bindings
        .append(&mut other.settings.key_bindings);

    merge_unique(&mut target.applied_sections, other.applied_sections);
    merge_unique(&mut target.skipped_sections, other.skipped_sections);
    merge_unique(&mut target.defaulted_keys, other.defaulted_keys);
    merge_unique(&mut target.ignored_unknown_keys, other.ignored_unknown_keys);
    target.warnings.append(&mut other.warnings);
}

/// Validate one section and dispatch to section-specific validation.
fn validate_section(section: &ParsedSection, source_path: &Path, report: &mut ValidationReport) {
    match section.name.as_str() {
        "editor" => {
            validate_editor_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        "include" => {
            validate_include_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        name if name.starts_with("keymap.") => {
            validate_keymap_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        _ => {
            push_unique(&mut report.ignored_unknown_keys, section.name.clone());
            report.warnings.push(WarningEvent::new(
                WarningCode::UnknownKey,
                format!("Unknown section `{}` ignored", section.name),
                source_path,
                Some(section.name.clone()),
                None,
            ));
        }
    }
}

/// Validate values in the `[editor]` section.
fn validate_editor_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    for item in &section.items {
        match item.key.as_str() {
            "scroll_margin" => match item.value {
                ParsedValue::Integer(value) if value >= 0 => {
                    report.settings.scroll_margin = Some(value as usize);
                }
                _ => {
                    push_unique(
                        &mut report.defaulted_keys,
                        format!("{}.{}", section.name, item.key),
                    );
                    report.warnings.push(WarningEvent::new(
                        WarningCode::InvalidValue,
                        "editor.scroll_margin must be a non-negative integer",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    ));
                }
            },
            "horizontal_scroll_margin" => match item.value {
                ParsedValue::Integer(value) if value >= 0 => {
                    report.settings.horizontal_scroll_margin = Some(value as usize);
                }
                _ => {
                    push_unique(
                        &mut report.defaulted_keys,
                        format!("{}.{}", section.name, item.key),
                    );
                    report.warnings.push(WarningEvent::new(
                        WarningCode::InvalidValue,
                        "editor.horizontal_scroll_margin must be a non-negative integer",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    ));
                }
            },
            _ => {
                push_unique(
                    &mut report.ignored_unknown_keys,
                    format!("{}.{}", section.name, item.key),
                );
                report.warnings.push(WarningEvent::new(
                    WarningCode::UnknownKey,
                    "Unknown editor setting ignored",
                    source_path,
                    Some(section.name.clone()),
                    Some(item.key.clone()),
                ));
            }
        }
    }
}

/// Validate values in the `[include]` section.
fn validate_include_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    for item in &section.items {
        match &item.value {
            ParsedValue::String(value) if !value.trim().is_empty() => {
                report.settings.include_paths.push(value.clone());
            }
            _ => report.warnings.push(WarningEvent::new(
                WarningCode::InvalidValue,
                "Include values must be non-empty strings",
                source_path,
                Some(section.name.clone()),
                Some(item.key.clone()),
            )),
        }
    }
}

/// Validate values in one `[keymap.<mode>]` section.
fn validate_keymap_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    let Some(mode_name) = section.name.strip_prefix("keymap.") else {
        return;
    };
    let Some(mode) = parse_mode_context(mode_name) else {
        report.warnings.push(WarningEvent::new(
            WarningCode::InvalidSection,
            format!("Unknown keymap mode `{}`", mode_name),
            source_path,
            Some(section.name.clone()),
            None,
        ));
        push_unique(&mut report.skipped_sections, section.name.clone());
        return;
    };

    for item in &section.items {
        let Some(action_name) = value_as_string(&item.value) else {
            report.warnings.push(WarningEvent::new(
                WarningCode::InvalidValue,
                "Keymap action must be a string",
                source_path,
                Some(section.name.clone()),
                Some(item.key.clone()),
            ));
            continue;
        };

        let Some(key) = parse_key_input(&item.key) else {
            report.warnings.push(WarningEvent::new(
                WarningCode::InvalidValue,
                "Invalid keymap key",
                source_path,
                Some(section.name.clone()),
                Some(item.key.clone()),
            ));
            continue;
        };

        let Some(action) = parse_action(action_name) else {
            report.warnings.push(WarningEvent::new(
                WarningCode::InvalidValue,
                format!("Unknown keymap action `{}`", action_name),
                source_path,
                Some(section.name.clone()),
                Some(item.key.clone()),
            ));
            continue;
        };

        report.settings.key_bindings.push(ConfiguredBinding {
            mode,
            key,
            action,
            source: format!("{}:{}:{}", source_path.display(), section.name, item.key),
        });
    }
}

/// Convert a parsed value to string when the value type is compatible.
fn value_as_string(value: &ParsedValue) -> Option<&str> {
    match value {
        ParsedValue::String(value) => Some(value),
        _ => None,
    }
}

/// Push a string value to the list only if it is not already present.
fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

/// Merge an incoming list into `values`, keeping only unique entries.
fn merge_unique(values: &mut Vec<String>, incoming: Vec<String>) {
    for value in incoming {
        push_unique(values, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parser::parse_str;
    use std::path::Path;

    #[test]
    fn parses_complex_key_bindings() {
        let input = r#"
[keymap.normal]
ctrl-f = "PageDown"
alt-b = "MoveWordBackward"
home = "MoveLineStart"
delete = "DeleteCharAtCursor"
space = "SaveCurrentFile"
pageup = "PageUp"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        let bindings = &report.settings.key_bindings;
        assert!(bindings.iter().any(
            |binding| binding.key == KeyInput::Ctrl('f') && binding.action == Action::PageDown
        ));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Alt('b') && binding.action == Action::MoveWordBackward
        }));
        assert!(bindings.iter().any(
            |binding| binding.key == KeyInput::Home && binding.action == Action::MoveLineStart
        ));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Delete && binding.action == Action::DeleteCharAtCursor
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Char(' ') && binding.action == Action::SaveCurrentFile
        }));
        assert!(
            bindings
                .iter()
                .any(|binding| binding.key == KeyInput::PageUp && binding.action == Action::PageUp)
        );
    }
}
