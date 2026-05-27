//! Validation and normalization for parsed configuration documents.
//!
//! This module keeps validation section-scoped so valid key mappings can still
//! be applied even when other sections contain invalid values.

use crate::config::warnings::{WarningCode, WarningEvent};
use crate::keybindings::{
    ActionBinding, Binding, KeyInput, ModeContext, OperatorBinding, ReplayBinding,
    ReplayParseError, parse_action, parse_key_input, parse_key_sequence, parse_mode_context,
    parse_operator_binding, parse_replay_sequence,
};
use crate::themes;
use crate::toml_like_parser::{
    ParsedDocument, ParsedItem, ParsedSection, ParsedValue, ParserDiagnosticKind,
};
use std::path::Path;

/// A key binding parsed from configuration and ready to apply at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredBinding {
    pub(crate) mode: ModeContext,
    pub(crate) key: KeyInput,
    pub(crate) binding: Binding,
    pub(crate) source: String,
}

/// A multi-key sequence binding parsed from configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredSequenceBinding {
    pub(crate) mode: ModeContext,
    pub(crate) keys: Vec<KeyInput>,
    pub(crate) binding: Binding,
    pub(crate) source: String,
}

/// One operator-pending key binding parsed from `[keymap.operator]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredOperatorBinding {
    pub(crate) key: KeyInput,
    pub(crate) binding: OperatorBinding,
    pub(crate) source: String,
}

/// Include path entry with source location metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncludePathEntry {
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) line_content: String,
}

/// Runtime settings resolved from one or more configuration files.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ConfigSettings {
    pub(crate) scroll_margin: Option<usize>,
    pub(crate) horizontal_scroll_margin: Option<usize>,
    pub(crate) relative_line_numbers: Option<bool>,
    pub(crate) soft_wrap: Option<bool>,
    pub(crate) auto_reload_external_changes: Option<bool>,
    pub(crate) indent_width: Option<usize>,
    pub(crate) indent_with_tabs: Option<bool>,
    pub(crate) file_picker_max_files: Option<usize>,
    pub(crate) sequence_discovery_popup: Option<bool>,
    pub(crate) theme: Option<String>,
    pub(crate) swap_exclude_patterns: Option<Vec<String>>,
    pub(crate) include_paths: Vec<IncludePathEntry>,
    pub(crate) key_bindings: Vec<ConfiguredBinding>,
    pub(crate) sequence_bindings: Vec<ConfiguredSequenceBinding>,
    pub(crate) operator_bindings: Vec<ConfiguredOperatorBinding>,
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

/// Shared metadata for validating one named setting assignment.
struct SettingContext<'a> {
    section_name: &'a str,
    item: &'a ParsedItem,
    source_path: &'a Path,
}

impl<'a> SettingContext<'a> {
    /// Capture source metadata for one setting assignment.
    fn new(section: &'a ParsedSection, item: &'a ParsedItem, source_path: &'a Path) -> Self {
        Self {
            section_name: &section.name,
            item,
            source_path,
        }
    }

    /// Return the fully qualified setting name used in reports and warnings.
    fn qualified_key(&self) -> String {
        format!("{}.{}", self.section_name, self.item.key)
    }

    /// Build a warning anchored to the setting's source line.
    fn warning(&self, code: WarningCode, message: impl Into<String>) -> WarningEvent {
        WarningEvent::new(
            code,
            message,
            self.source_path,
            Some(self.section_name.to_string()),
            Some(self.item.key.clone()),
        )
        .with_position(self.item.line, None, Some(self.item.line_content.clone()))
    }
}

/// Validate a parsed config document and collect resilient warnings.
pub(crate) fn validate_document(doc: &ParsedDocument) -> ValidationReport {
    let mut report = ValidationReport::default();

    for diag in &doc.diagnostics {
        let code = match diag.kind {
            ParserDiagnosticKind::InvalidHeader => WarningCode::InvalidSection,
            ParserDiagnosticKind::InvalidAssignment
            | ParserDiagnosticKind::InvalidValue
            | ParserDiagnosticKind::UnterminatedString => WarningCode::InvalidValue,
        };
        if let Some(section) = diag.section.clone() {
            report.warnings.push(
                WarningEvent::new(code, &diag.message, &doc.source_path, Some(section), None)
                    .with_position(
                        diag.line,
                        Some(diag.column),
                        Some(diag.line_content.clone()),
                    ),
            );
        } else {
            report.warnings.push(
                WarningEvent::new(code, &diag.message, &doc.source_path, None, None).with_position(
                    diag.line,
                    Some(diag.column),
                    Some(diag.line_content.clone()),
                ),
            );
        }
    }

    // Parse diagnostics are attached to individual lines, so we keep validating
    // all parsed items in each section to retain as many usable settings as possible.
    for section in &doc.sections {
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
    if let Some(value) = other.settings.relative_line_numbers.take() {
        target.settings.relative_line_numbers = Some(value);
    }
    if let Some(value) = other.settings.soft_wrap.take() {
        target.settings.soft_wrap = Some(value);
    }
    if let Some(value) = other.settings.auto_reload_external_changes.take() {
        target.settings.auto_reload_external_changes = Some(value);
    }
    if let Some(value) = other.settings.indent_width.take() {
        target.settings.indent_width = Some(value);
    }
    if let Some(value) = other.settings.indent_with_tabs.take() {
        target.settings.indent_with_tabs = Some(value);
    }
    if let Some(value) = other.settings.file_picker_max_files.take() {
        target.settings.file_picker_max_files = Some(value);
    }
    if let Some(value) = other.settings.sequence_discovery_popup.take() {
        target.settings.sequence_discovery_popup = Some(value);
    }
    if let Some(value) = other.settings.theme.take() {
        target.settings.theme = Some(value);
    }
    if let Some(value) = other.settings.swap_exclude_patterns.take() {
        target.settings.swap_exclude_patterns = Some(value);
    }
    target
        .settings
        .include_paths
        .append(&mut other.settings.include_paths);
    target
        .settings
        .key_bindings
        .append(&mut other.settings.key_bindings);
    target
        .settings
        .sequence_bindings
        .append(&mut other.settings.sequence_bindings);
    target
        .settings
        .operator_bindings
        .append(&mut other.settings.operator_bindings);

    merge_unique(&mut target.applied_sections, other.applied_sections);
    merge_unique(&mut target.skipped_sections, other.skipped_sections);
    merge_unique(&mut target.defaulted_keys, other.defaulted_keys);
    merge_unique(&mut target.ignored_unknown_keys, other.ignored_unknown_keys);
    target.warnings.append(&mut other.warnings);
}

/// Validate one section and dispatch to section-specific validation.
fn validate_section(section: &ParsedSection, source_path: &Path, report: &mut ValidationReport) {
    if section.name == "root" && section.items.is_empty() {
        return;
    }
    match section.name.as_str() {
        "editor" => {
            validate_editor_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        "include" => {
            validate_include_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        "swap" => {
            validate_swap_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        "keymap.operator" => {
            validate_operator_keymap_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        name if name.starts_with("keymap.") => {
            validate_keymap_section(section, source_path, report);
            push_unique(&mut report.applied_sections, section.name.clone());
        }
        "root" => {
            for item in &section.items {
                push_unique(
                    &mut report.ignored_unknown_keys,
                    format!("root.{}", item.key),
                );
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::UnknownKey,
                        "Unknown top-level setting ignored; place settings under a section",
                        source_path,
                        Some("root".to_string()),
                        Some(item.key.clone()),
                    )
                    .with_position(
                        item.line,
                        None,
                        Some(item.line_content.clone()),
                    ),
                );
            }
        }
        _ => {
            push_unique(&mut report.ignored_unknown_keys, section.name.clone());
            let warning = WarningEvent::new(
                WarningCode::UnknownKey,
                format!("Unknown section `{}` ignored", section.name),
                source_path,
                Some(section.name.clone()),
                None,
            );
            if let Some(line) = section.header_line {
                report.warnings.push(warning.with_position(
                    line,
                    None,
                    section.header_line_content.clone(),
                ));
            } else if let Some(item) = section.items.first() {
                report.warnings.push(warning.with_position(
                    item.line,
                    None,
                    Some(item.line_content.clone()),
                ));
            } else {
                report.warnings.push(warning);
            }
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
        let context = SettingContext::new(section, item, source_path);

        // Each branch only declares its domain-specific validator. Shared helpers
        // own the default tracking and source-aware warning emission.
        match item.key.as_str() {
            "scroll_margin" => {
                if let Some(value) = validate_non_negative_integer_setting(report, &context) {
                    report.settings.scroll_margin = Some(value);
                }
            }
            "horizontal_scroll_margin" => {
                if let Some(value) = validate_non_negative_integer_setting(report, &context) {
                    report.settings.horizontal_scroll_margin = Some(value);
                }
            }
            "relative_line_numbers" => {
                if let Some(value) = validate_boolean_setting(report, &context) {
                    report.settings.relative_line_numbers = Some(value);
                }
            }
            "soft_wrap" => {
                if let Some(value) = validate_boolean_setting(report, &context) {
                    report.settings.soft_wrap = Some(value);
                }
            }
            "auto_reload_external_changes" => {
                if let Some(value) = validate_boolean_setting(report, &context) {
                    report.settings.auto_reload_external_changes = Some(value);
                }
            }
            "indent_width" => {
                if let Some(value) = validate_positive_integer_setting(report, &context) {
                    report.settings.indent_width = Some(value);
                }
            }
            "indent_with_tabs" => {
                if let Some(value) = validate_boolean_setting(report, &context) {
                    report.settings.indent_with_tabs = Some(value);
                }
            }
            "file_picker_max_files" => {
                if let Some(value) = validate_positive_integer_setting(report, &context) {
                    report.settings.file_picker_max_files = Some(value);
                }
            }
            "sequence_discovery_popup" => {
                if let Some(value) = validate_boolean_setting(report, &context) {
                    report.settings.sequence_discovery_popup = Some(value);
                }
            }
            "theme" => {
                if let Some(value) = validate_theme_setting(report, &context) {
                    report.settings.theme = Some(value);
                }
            }
            _ => {
                record_unknown_setting(report, &context, "Unknown editor setting ignored");
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
                report.settings.include_paths.push(IncludePathEntry {
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

/// Validate values in the `[swap]` section.
fn validate_swap_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    for item in &section.items {
        let context = SettingContext::new(section, item, source_path);
        match item.key.as_str() {
            "exclude" => {
                if let Some(patterns) = validate_swap_exclude_patterns(report, &context) {
                    report.settings.swap_exclude_patterns = Some(patterns);
                }
            }
            _ => {
                record_unknown_setting(report, &context, "Unknown swap setting ignored");
            }
        }
    }
}

/// Validate values in one `[keymap.<mode>]` section.
fn validate_keymap_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    // Keymap sections are mode-specific, so we resolve the mode once up front and
    // skip the whole section if the suffix doesn't map to a runtime mode.
    let Some(mode_name) = section.name.strip_prefix("keymap.") else {
        return;
    };
    let Some(mode) = parse_mode_context(mode_name) else {
        let warning = WarningEvent::new(
            WarningCode::InvalidSection,
            format!("Unknown keymap mode `{}`", mode_name),
            source_path,
            Some(section.name.clone()),
            None,
        );
        if let Some(item) = section.items.first() {
            report.warnings.push(warning.with_position(
                item.line,
                None,
                Some(item.line_content.clone()),
            ));
        } else {
            report.warnings.push(warning);
        }
        push_unique(&mut report.skipped_sections, section.name.clone());
        return;
    };

    for item in &section.items {
        // First validate the value shape and action names. We reject the whole
        // binding on any invalid action so partial multi-action arrays never
        // produce surprising half-applied mappings.
        let source = format!("{}:{}:{}", source_path.display(), section.name, item.key);
        let binding = match parse_keymap_binding(mode, item, source_path, &source) {
            Ok(binding) => binding,
            Err(ActionBindingParseError::EmptyArray) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        "Keymap action array must not be empty",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(
                        item.line,
                        None,
                        Some(item.line_content.clone()),
                    ),
                );
                continue;
            }
            Err(ActionBindingParseError::EmptyReplay) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        "Key replay must not be empty after `@`",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(
                        item.line,
                        None,
                        Some(item.line_content.clone()),
                    ),
                );
                continue;
            }
            Err(ActionBindingParseError::UnknownAction(invalid_name)) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        format!(
                            "Unknown keymap action `{}`; use case-sensitive kebab-case names like `move-down`",
                            invalid_name
                        ),
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(item.line, None, Some(item.line_content.clone())),
                );
                continue;
            }
            Err(ActionBindingParseError::InvalidReplayToken(token)) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        format!(
                            "Invalid key replay token `<{}>`; use supported names like `<Enter>`, `<Tab>`, or `<Ctrl-Home>`",
                            token
                        ),
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(item.line, None, Some(item.line_content.clone())),
                );
                continue;
            }
            Err(ActionBindingParseError::UnterminatedReplayToken) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        "Unterminated key replay token; close angle-bracket keys like `<Enter>` with `>`",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(item.line, None, Some(item.line_content.clone())),
                );
                continue;
            }
            Err(ActionBindingParseError::InvalidType) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        "Keymap action must be a string or array of strings",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(
                        item.line,
                        None,
                        Some(item.line_content.clone()),
                    ),
                );
                continue;
            }
        };

        // Then decide whether the left-hand side is a direct key or a multi-key
        // sequence. Both share the same validated action payload and source metadata.
        if let Some(key) = parse_key_input(&item.key) {
            report.settings.key_bindings.push(ConfiguredBinding {
                mode,
                key,
                binding,
                source,
            });
        } else {
            let Some(keys) = parse_key_sequence(&item.key) else {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        "Invalid keymap key",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(
                        item.line,
                        None,
                        Some(item.line_content.clone()),
                    ),
                );
                continue;
            };
            report
                .settings
                .sequence_bindings
                .push(ConfiguredSequenceBinding {
                    mode,
                    keys,
                    binding,
                    source,
                });
        }
    }
}

/// Why parsing one operator binding value failed.
enum OperatorBindingParseError<'a> {
    InvalidType,
    UnknownBinding(&'a str),
}

/// Validate values in `[keymap.operator]`.
fn validate_operator_keymap_section(
    section: &ParsedSection,
    source_path: &Path,
    report: &mut ValidationReport,
) {
    for item in &section.items {
        let Some(key) = parse_key_input(&item.key) else {
            report.warnings.push(
                WarningEvent::new(
                    WarningCode::InvalidValue,
                    "Operator keymap keys must be single keys",
                    source_path,
                    Some(section.name.clone()),
                    Some(item.key.clone()),
                )
                .with_position(item.line, None, Some(item.line_content.clone())),
            );
            continue;
        };

        let binding = match parse_operator_binding_value(&item.value) {
            Ok(binding) => binding,
            Err(OperatorBindingParseError::UnknownBinding(invalid_name)) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        format!(
                            "Unknown operator keymap action `{}`; use kebab-case names like `word-forward`",
                            invalid_name
                        ),
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(item.line, None, Some(item.line_content.clone())),
                );
                continue;
            }
            Err(OperatorBindingParseError::InvalidType) => {
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::InvalidValue,
                        "Operator keymap action must be a string",
                        source_path,
                        Some(section.name.clone()),
                        Some(item.key.clone()),
                    )
                    .with_position(
                        item.line,
                        None,
                        Some(item.line_content.clone()),
                    ),
                );
                continue;
            }
        };

        report
            .settings
            .operator_bindings
            .push(ConfiguredOperatorBinding {
                key,
                binding,
                source: format!("{}:{}:{}", source_path.display(), section.name, item.key),
            });
    }
}

/// Why parsing a keymap action value failed.
enum ActionBindingParseError {
    InvalidType,
    EmptyArray,
    EmptyReplay,
    UnknownAction(String),
    InvalidReplayToken(String),
    UnterminatedReplayToken,
}

/// Parse one `[keymap.<mode>]` value into the runtime binding payload.
fn parse_keymap_binding(
    mode: ModeContext,
    item: &ParsedItem,
    source_path: &Path,
    source: &str,
) -> Result<Binding, ActionBindingParseError> {
    parse_action_binding(mode, &item.key, &item.value, source_path, source)
}

/// Parse keymap values into the runtime representation, preserving array order.
fn parse_action_binding(
    _mode: ModeContext,
    trigger: &str,
    value: &ParsedValue,
    _source_path: &Path,
    source: &str,
) -> Result<Binding, ActionBindingParseError> {
    match value {
        ParsedValue::String(value) if value.trim_start().starts_with('@') => {
            let syntax = value.trim();
            let replay_syntax = syntax
                .strip_prefix('@')
                .expect("checked replay prefix")
                .to_string();
            let keys = match parse_replay_sequence(&replay_syntax) {
                Ok(keys) => keys,
                Err(ReplayParseError::Empty) => {
                    return Err(ActionBindingParseError::EmptyReplay);
                }
                Err(ReplayParseError::InvalidToken(token)) => {
                    return Err(ActionBindingParseError::InvalidReplayToken(token));
                }
                Err(ReplayParseError::UnterminatedToken) => {
                    return Err(ActionBindingParseError::UnterminatedReplayToken);
                }
            };
            Ok(Binding::Replay(ReplayBinding::new(
                keys,
                replay_syntax,
                trigger.to_string(),
                source.to_string(),
            )))
        }
        ParsedValue::String(value) => parse_action(value)
            .map(ActionBinding::single)
            .map(Binding::actions)
            .ok_or_else(|| ActionBindingParseError::UnknownAction(value.clone())),
        ParsedValue::StringArray(values) => {
            if values.is_empty() {
                return Err(ActionBindingParseError::EmptyArray);
            }
            let mut actions = Vec::with_capacity(values.len());
            for value in values {
                let action = parse_action(value)
                    .ok_or_else(|| ActionBindingParseError::UnknownAction(value.clone()))?;
                actions.push(action);
            }
            Ok(Binding::actions(
                ActionBinding::from_actions(actions).expect("validated actions must not be empty"),
            ))
        }
        _ => Err(ActionBindingParseError::InvalidType),
    }
}

/// Parse one operator keymap value.
fn parse_operator_binding_value(
    value: &ParsedValue,
) -> Result<OperatorBinding, OperatorBindingParseError<'_>> {
    match value {
        ParsedValue::String(value) => {
            parse_operator_binding(value).ok_or(OperatorBindingParseError::UnknownBinding(value))
        }
        _ => Err(OperatorBindingParseError::InvalidType),
    }
}

/// Record that a setting kept its default because validation failed.
fn record_defaulted_invalid_value(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
    message: impl Into<String>,
) {
    push_unique(&mut report.defaulted_keys, context.qualified_key());
    report
        .warnings
        .push(context.warning(WarningCode::InvalidValue, message));
}

/// Record that a setting key is unknown and was ignored.
fn record_unknown_setting(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
    message: impl Into<String>,
) {
    push_unique(&mut report.ignored_unknown_keys, context.qualified_key());
    report
        .warnings
        .push(context.warning(WarningCode::UnknownKey, message));
}

/// Validate one setting value with shared default tracking and warning emission.
fn validate_setting_value<T, F>(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
    message: impl Into<String>,
    parse: F,
) -> Option<T>
where
    F: FnOnce(&ParsedValue) -> Option<T>,
{
    match parse(&context.item.value) {
        Some(value) => Some(value),
        None => {
            // Validation failures all follow the same defaulting path.
            record_defaulted_invalid_value(report, context, message);
            None
        }
    }
}

/// Extract a boolean from a parsed value.
fn parse_boolean_value(value: &ParsedValue) -> Option<bool> {
    match value {
        ParsedValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

/// Extract a non-negative integer from a parsed value.
fn parse_non_negative_usize_value(value: &ParsedValue) -> Option<usize> {
    match value {
        ParsedValue::Integer(value) if *value >= 0 => usize::try_from(*value).ok(),
        _ => None,
    }
}

/// Extract a positive integer from a parsed value.
fn parse_positive_usize_value(value: &ParsedValue) -> Option<usize> {
    match value {
        ParsedValue::Integer(value) if *value > 0 => usize::try_from(*value).ok(),
        _ => None,
    }
}

/// Extract a string from a parsed value.
fn parse_string_value(value: &ParsedValue) -> Option<String> {
    match value {
        ParsedValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

/// Extract a string-array value from one parsed setting.
fn parse_string_array_value(value: &ParsedValue) -> Option<Vec<String>> {
    match value {
        ParsedValue::StringArray(values) => Some(values.clone()),
        _ => None,
    }
}

/// Validate a boolean editor setting.
fn validate_boolean_setting(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
) -> Option<bool> {
    let setting_name = context.qualified_key();
    validate_setting_value(
        report,
        context,
        format!("{setting_name} must be a boolean"),
        parse_boolean_value,
    )
}

/// Validate a non-negative integer editor setting.
fn validate_non_negative_integer_setting(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
) -> Option<usize> {
    let setting_name = context.qualified_key();
    validate_setting_value(
        report,
        context,
        format!("{setting_name} must be a non-negative integer"),
        parse_non_negative_usize_value,
    )
}

/// Validate a positive integer editor setting.
fn validate_positive_integer_setting(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
) -> Option<usize> {
    let setting_name = context.qualified_key();
    validate_setting_value(
        report,
        context,
        format!("{setting_name} must be a positive integer"),
        parse_positive_usize_value,
    )
}

/// Validate a string editor setting.
fn validate_string_setting(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
) -> Option<String> {
    let setting_name = context.qualified_key();
    validate_setting_value(
        report,
        context,
        format!("{setting_name} must be a string"),
        parse_string_value,
    )
}

/// Validate an editor theme name against the registered themes.
fn validate_theme_setting(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
) -> Option<String> {
    let theme_name = validate_string_setting(report, context)?;
    if themes::find(&theme_name).is_some() {
        return Some(theme_name);
    }

    // Theme membership is checked after the shared string validation so the
    // warning can list the supported names without duplicating type checks.
    record_defaulted_invalid_value(
        report,
        context,
        format!(
            "{} must be one of: {}",
            context.qualified_key(),
            themes::names().join(", ")
        ),
    );
    None
}

/// Validate `[swap] exclude` as a string array and drop empty patterns with warnings.
fn validate_swap_exclude_patterns(
    report: &mut ValidationReport,
    context: &SettingContext<'_>,
) -> Option<Vec<String>> {
    let mut patterns = validate_setting_value(
        report,
        context,
        format!("{} must be an array of strings", context.qualified_key()),
        parse_string_array_value,
    )?;

    // Empty entries are ignored one-by-one so a single bad pattern does not
    // discard the rest of the exclusion list.
    let original_len = patterns.len();
    patterns.retain(|pattern| !pattern.trim().is_empty());
    if patterns.len() != original_len {
        report.warnings.push(context.warning(
            WarningCode::InvalidValue,
            format!("{} ignores empty string patterns", context.qualified_key()),
        ));
    }
    Some(patterns)
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
    use crate::keybindings::{Action, Binding, OperatorBinding};
    use crate::toml_like_parser::parse_str;
    use std::path::Path;

    #[test]
    fn parses_complex_key_bindings() {
        let input = r#"
[keymap.normal]
ctrl-f = "page-down"
alt-b = "move-word-backward"
home = "move-line-start"
ctrl-home = "move-to-last-line"
delete = "delete-char-at-cursor"
space = "save-current-file"
pageup = "page-up"
é = "move-right"
zu = "move-down"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        let bindings = &report.settings.key_bindings;
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Ctrl('f')
                && binding.binding
                    == Binding::actions(ActionBinding::Single(crate::keybindings::Action::PageDown))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Alt('b')
                && binding.binding
                    == Binding::actions(ActionBinding::Single(
                        crate::keybindings::Action::MoveWordBackward,
                    ))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Home
                && binding.binding
                    == Binding::actions(ActionBinding::Single(
                        crate::keybindings::Action::MoveLineStart,
                    ))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::CtrlHome
                && binding.binding
                    == Binding::actions(ActionBinding::Single(
                        crate::keybindings::Action::MoveToLastLine,
                    ))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Delete
                && binding.binding
                    == Binding::actions(ActionBinding::Single(
                        crate::keybindings::Action::DeleteCharAtCursor,
                    ))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Char(' ')
                && binding.binding
                    == Binding::actions(ActionBinding::Single(
                        crate::keybindings::Action::SaveCurrentFile,
                    ))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::PageUp
                && binding.binding
                    == Binding::actions(ActionBinding::Single(crate::keybindings::Action::PageUp))
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Char('é')
                && binding.binding
                    == Binding::actions(ActionBinding::Single(
                        crate::keybindings::Action::MoveRight,
                    ))
        }));
        assert!(report.settings.sequence_bindings.iter().any(|binding| {
            binding.keys == vec![KeyInput::Char('z'), KeyInput::Char('u')]
                && binding.binding
                    == Binding::actions(ActionBinding::Single(crate::keybindings::Action::MoveDown))
        }));
    }

    #[test]
    fn keeps_valid_items_when_one_line_is_invalid() {
        let input = r#"
[editor]
bad ??? 9
horizontal_scroll_margin = 4
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.horizontal_scroll_margin, Some(4));
        assert!(!report.warnings.is_empty());
    }

    #[test]
    fn accepts_relative_line_numbers_boolean() {
        let input = r#"
[editor]
relative_line_numbers = true
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.relative_line_numbers, Some(true));
        assert!(report.warnings.is_empty());
    }

    #[test]
    /// Accept non-negative scroll margin values.
    fn accepts_non_negative_scroll_margin() {
        let input = r#"
[editor]
scroll_margin = 0
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.scroll_margin, Some(0));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_soft_wrap_boolean() {
        let input = r#"
[editor]
soft_wrap = false
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.soft_wrap, Some(false));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_positive_indent_width() {
        let input = r#"
[editor]
indent_width = 2
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.indent_width, Some(2));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_indent_with_tabs_boolean() {
        let input = r#"
[editor]
indent_with_tabs = true
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.indent_with_tabs, Some(true));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_positive_file_picker_max_files() {
        let input = r#"
[editor]
file_picker_max_files = 512
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.file_picker_max_files, Some(512));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_sequence_discovery_popup_boolean() {
        let input = r#"
[editor]
sequence_discovery_popup = false
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.sequence_discovery_popup, Some(false));
        assert!(report.warnings.is_empty());
    }

    #[test]
    /// Accept the external-change auto-reload toggle when it is a boolean.
    fn accepts_auto_reload_external_changes_boolean() {
        let input = r#"
[editor]
auto_reload_external_changes = false
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.auto_reload_external_changes, Some(false));
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_known_theme_name() {
        let input = r#"
[editor]
theme = "nord"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.theme.as_deref(), Some("nord"));
        assert!(report.warnings.is_empty());
    }

    #[test]
    /// Reject negative scroll margin values.
    fn rejects_negative_scroll_margin() {
        let input = r#"
[editor]
scroll_margin = -1
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.scroll_margin, None);
        assert_eq!(report.defaulted_keys, vec!["editor.scroll_margin"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.scroll_margin must be a non-negative integer"
        );
    }

    #[test]
    fn rejects_non_boolean_relative_line_numbers() {
        let input = r#"
[editor]
relative_line_numbers = 1
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.relative_line_numbers, None);
        assert_eq!(report.defaulted_keys, vec!["editor.relative_line_numbers"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.relative_line_numbers must be a boolean"
        );
    }

    #[test]
    fn rejects_non_boolean_soft_wrap() {
        let input = r#"
[editor]
soft_wrap = 1
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.soft_wrap, None);
        assert_eq!(report.defaulted_keys, vec!["editor.soft_wrap"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.soft_wrap must be a boolean"
        );
    }

    #[test]
    fn rejects_non_positive_indent_width() {
        let input = r#"
[editor]
indent_width = 0
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.indent_width, None);
        assert_eq!(report.defaulted_keys, vec!["editor.indent_width"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.indent_width must be a positive integer"
        );
    }

    #[test]
    fn rejects_non_boolean_indent_with_tabs() {
        let input = r#"
[editor]
indent_with_tabs = 1
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.indent_with_tabs, None);
        assert_eq!(report.defaulted_keys, vec!["editor.indent_with_tabs"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.indent_with_tabs must be a boolean"
        );
    }

    #[test]
    fn rejects_non_boolean_sequence_discovery_popup() {
        let input = r#"
[editor]
sequence_discovery_popup = 1
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.sequence_discovery_popup, None);
        assert_eq!(
            report.defaulted_keys,
            vec!["editor.sequence_discovery_popup"]
        );
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.sequence_discovery_popup must be a boolean"
        );
    }

    #[test]
    /// Reject non-boolean external-change auto-reload values.
    fn rejects_non_boolean_auto_reload_external_changes() {
        let input = r#"
[editor]
auto_reload_external_changes = 1
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.auto_reload_external_changes, None);
        assert_eq!(
            report.defaulted_keys,
            vec!["editor.auto_reload_external_changes"]
        );
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.auto_reload_external_changes must be a boolean"
        );
    }

    #[test]
    fn rejects_non_positive_file_picker_max_files() {
        let input = r#"
[editor]
file_picker_max_files = 0
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.file_picker_max_files, None);
        assert_eq!(report.defaulted_keys, vec!["editor.file_picker_max_files"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "editor.file_picker_max_files must be a positive integer"
        );
    }

    #[test]
    /// Reject non-string theme values before checking theme membership.
    fn rejects_non_string_theme_value() {
        let input = r#"
[editor]
theme = true
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.theme, None);
        assert_eq!(report.defaulted_keys, vec!["editor.theme"]);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].message, "editor.theme must be a string");
    }

    #[test]
    fn rejects_unknown_theme_name() {
        let input = r#"
[editor]
theme = "missing-theme"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.theme, None);
        assert_eq!(report.defaulted_keys, vec!["editor.theme"]);
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0]
                .message
                .contains("editor.theme must be one of:")
        );
    }

    #[test]
    fn adversarial_invalid_prelude_keeps_following_sections() {
        let input = r#"
keymap.command]
value [= "test"

[keymap.normal]
r = "move-right"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(
            report
                .settings
                .key_bindings
                .iter()
                .any(|binding| binding.key == KeyInput::Char('r')
                    && binding.binding
                        == Binding::actions(ActionBinding::Single(
                            crate::keybindings::Action::MoveRight,
                        )))
        );
        assert!(!report.warnings.is_empty());
    }

    #[test]
    fn empty_config_does_not_emit_root_warning() {
        let doc = parse_str(Path::new("test.cfg"), "");
        let report = validate_document(&doc);
        assert!(report.warnings.is_empty());
        assert!(report.ignored_unknown_keys.is_empty());
    }

    #[test]
    fn unknown_section_warning_uses_header_line() {
        let input = r#"
[unknown_section]
foo = "bar"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].line, Some(2));
        assert_eq!(
            report.warnings[0].line_content.as_deref(),
            Some("[unknown_section]")
        );
    }

    #[test]
    fn rejects_non_kebab_case_action_names() {
        let input = r#"
[keymap.normal]
z = "MoveDown"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.key_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Unknown keymap action `MoveDown`; use case-sensitive kebab-case names like `move-down`"
        );
    }

    #[test]
    fn parses_multi_action_bindings() {
        let input = r#"
[keymap.normal]
z = ["move-down", "move-right"]
zu = ["move-down", "move-right"]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.warnings.is_empty());
        assert!(report.settings.key_bindings.iter().any(|binding| {
            binding.key == KeyInput::Char('z')
                && binding.binding
                    == Binding::actions(ActionBinding::Multiple(vec![
                        Action::MoveDown,
                        Action::MoveRight,
                    ]))
        }));
        assert!(report.settings.sequence_bindings.iter().any(|binding| {
            binding.keys == vec![KeyInput::Char('z'), KeyInput::Char('u')]
                && binding.binding
                    == Binding::actions(ActionBinding::Multiple(vec![
                        Action::MoveDown,
                        Action::MoveRight,
                    ]))
        }));
    }

    #[test]
    fn parses_replay_bindings() {
        let input = r#"
[keymap.normal]
c = "@diw<Enter><Tab>"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.warnings.is_empty());
        assert!(report.settings.key_bindings.iter().any(|binding| {
            binding.key == KeyInput::Char('c')
                && matches!(
                    &binding.binding,
                    Binding::Replay(replay)
                        if replay.keys
                            == vec![
                                KeyInput::Char('d'),
                                KeyInput::Char('i'),
                                KeyInput::Char('w'),
                                KeyInput::Char('\n'),
                                KeyInput::Ctrl('i'),
                            ]
                            && replay.syntax == "diw<Enter><Tab>"
                            && replay.trigger == "c"
                )
        }));
    }

    #[test]
    fn rejects_empty_replay_bindings() {
        let input = r#"
[keymap.normal]
c = "@"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.key_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Key replay must not be empty after `@`"
        );
    }

    #[test]
    fn rejects_invalid_replay_tokens() {
        let input = r#"
[keymap.normal]
c = "@diw<Nope>"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.key_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Invalid key replay token `<Nope>`; use supported names like `<Enter>`, `<Tab>`, or `<Ctrl-Home>`"
        );
    }

    #[test]
    fn parses_operator_bindings() {
        let input = r#"
[keymap.operator]
é = "word-forward"
g = "paragraph-forward"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.warnings.is_empty());
        assert!(report.settings.operator_bindings.iter().any(|binding| {
            binding.key == KeyInput::Char('é') && binding.binding == OperatorBinding::WordForward
        }));
        assert!(report.settings.operator_bindings.iter().any(|binding| {
            binding.key == KeyInput::Char('g')
                && binding.binding == OperatorBinding::ParagraphForward
        }));
    }

    #[test]
    fn rejects_invalid_operator_binding_name() {
        let input = r#"
[keymap.operator]
w = "move-word-forward"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.operator_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Unknown operator keymap action `move-word-forward`; use kebab-case names like `word-forward`"
        );
    }

    #[test]
    fn rejects_operator_binding_arrays() {
        let input = r#"
[keymap.operator]
g = ["paragraph-forward"]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.operator_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Operator keymap action must be a string"
        );
    }

    #[test]
    fn rejects_multi_action_binding_when_one_action_is_invalid() {
        let input = r#"
[keymap.normal]
z = ["move-down", "MoveRight"]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.key_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Unknown keymap action `MoveRight`; use case-sensitive kebab-case names like `move-down`"
        );
    }

    #[test]
    fn rejects_empty_action_arrays() {
        let input = r#"
[keymap.normal]
z = []
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert!(report.settings.key_bindings.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "Keymap action array must not be empty"
        );
    }

    #[test]
    fn accepts_swap_exclude_patterns() {
        let input = r#"
[swap]
exclude = ["/dev/shm/gopass*", "*.gpg"]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(
            report.settings.swap_exclude_patterns,
            Some(vec!["/dev/shm/gopass*".to_string(), "*.gpg".to_string()])
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn accepts_multiline_swap_exclude_patterns() {
        let input = r#"
[swap]
exclude = [
  "/dev/shm/gopass*",
  "*.gpg",
]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(
            report.settings.swap_exclude_patterns,
            Some(vec!["/dev/shm/gopass*".to_string(), "*.gpg".to_string()])
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn rejects_non_array_swap_exclude_value() {
        let input = r#"
[swap]
exclude = "*.gpg"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(report.settings.swap_exclude_patterns, None);
        assert_eq!(report.defaulted_keys, vec!["swap.exclude"]);
        assert_eq!(
            report.warnings[0].message,
            "swap.exclude must be an array of strings"
        );
    }

    #[test]
    fn ignores_empty_swap_exclude_entries() {
        let input = r#"
[swap]
exclude = ["", "*.gpg", "   "]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        let report = validate_document(&doc);
        assert_eq!(
            report.settings.swap_exclude_patterns,
            Some(vec!["*.gpg".to_string()])
        );
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].message,
            "swap.exclude ignores empty string patterns"
        );
    }
}
