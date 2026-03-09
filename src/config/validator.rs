//! Validation and normalization for parsed configuration documents.
//!
//! This module keeps validation section-scoped so valid key mappings can still
//! be applied even when other sections contain invalid values.

use crate::config::parser::{ParsedDocument, ParsedSection, ParsedValue, ParserDiagnosticKind};
use crate::config::warnings::{WarningCode, WarningEvent};
use crate::keybindings::{
    ActionBinding, KeyInput, ModeContext, parse_action, parse_key_input, parse_key_sequence,
    parse_mode_context,
};
use std::path::Path;

/// A key binding parsed from configuration and ready to apply at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredBinding {
    pub(crate) mode: ModeContext,
    pub(crate) key: KeyInput,
    pub(crate) actions: ActionBinding,
    pub(crate) source: String,
}

/// A multi-key sequence binding parsed from configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredSequenceBinding {
    pub(crate) mode: ModeContext,
    pub(crate) keys: Vec<KeyInput>,
    pub(crate) actions: ActionBinding,
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
    pub(crate) include_paths: Vec<IncludePathEntry>,
    pub(crate) key_bindings: Vec<ConfiguredBinding>,
    pub(crate) sequence_bindings: Vec<ConfiguredSequenceBinding>,
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
            if let Some(item) = section.items.first() {
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
                    report.warnings.push(
                        WarningEvent::new(
                            WarningCode::InvalidValue,
                            "editor.scroll_margin must be a non-negative integer",
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
                    report.warnings.push(
                        WarningEvent::new(
                            WarningCode::InvalidValue,
                            "editor.horizontal_scroll_margin must be a non-negative integer",
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
                }
            },
            "relative_line_numbers" => match item.value {
                ParsedValue::Boolean(value) => {
                    report.settings.relative_line_numbers = Some(value);
                }
                _ => {
                    push_unique(
                        &mut report.defaulted_keys,
                        format!("{}.{}", section.name, item.key),
                    );
                    report.warnings.push(
                        WarningEvent::new(
                            WarningCode::InvalidValue,
                            "editor.relative_line_numbers must be a boolean",
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
                }
            },
            "soft_wrap" => match item.value {
                ParsedValue::Boolean(value) => {
                    report.settings.soft_wrap = Some(value);
                }
                _ => {
                    push_unique(
                        &mut report.defaulted_keys,
                        format!("{}.{}", section.name, item.key),
                    );
                    report.warnings.push(
                        WarningEvent::new(
                            WarningCode::InvalidValue,
                            "editor.soft_wrap must be a boolean",
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
                }
            },
            _ => {
                push_unique(
                    &mut report.ignored_unknown_keys,
                    format!("{}.{}", section.name, item.key),
                );
                report.warnings.push(
                    WarningEvent::new(
                        WarningCode::UnknownKey,
                        "Unknown editor setting ignored",
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
        let actions = match parse_action_binding(&item.value) {
            Ok(actions) => actions,
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
                actions,
                source: format!("{}:{}:{}", source_path.display(), section.name, item.key),
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
                    actions,
                    source: format!("{}:{}:{}", source_path.display(), section.name, item.key),
                });
        }
    }
}

/// Why parsing a keymap action value failed.
enum ActionBindingParseError<'a> {
    InvalidType,
    EmptyArray,
    UnknownAction(&'a str),
}

/// Parse keymap action values into the runtime representation, preserving array order.
fn parse_action_binding(value: &ParsedValue) -> Result<ActionBinding, ActionBindingParseError<'_>> {
    match value {
        ParsedValue::String(value) => parse_action(value)
            .map(ActionBinding::single)
            .ok_or(ActionBindingParseError::UnknownAction(value)),
        ParsedValue::StringArray(values) => {
            if values.is_empty() {
                return Err(ActionBindingParseError::EmptyArray);
            }
            let mut actions = Vec::with_capacity(values.len());
            for value in values {
                let action =
                    parse_action(value).ok_or(ActionBindingParseError::UnknownAction(value))?;
                actions.push(action);
            }
            Ok(ActionBinding::from_actions(actions).expect("validated actions must not be empty"))
        }
        _ => Err(ActionBindingParseError::InvalidType),
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
    use crate::keybindings::Action;
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
                && binding.actions == ActionBinding::Single(crate::keybindings::Action::PageDown)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Alt('b')
                && binding.actions
                    == ActionBinding::Single(crate::keybindings::Action::MoveWordBackward)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Home
                && binding.actions
                    == ActionBinding::Single(crate::keybindings::Action::MoveLineStart)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::CtrlHome
                && binding.actions
                    == ActionBinding::Single(crate::keybindings::Action::MoveToLastLine)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Delete
                && binding.actions
                    == ActionBinding::Single(crate::keybindings::Action::DeleteCharAtCursor)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Char(' ')
                && binding.actions
                    == ActionBinding::Single(crate::keybindings::Action::SaveCurrentFile)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::PageUp
                && binding.actions == ActionBinding::Single(crate::keybindings::Action::PageUp)
        }));
        assert!(bindings.iter().any(|binding| {
            binding.key == KeyInput::Char('é')
                && binding.actions == ActionBinding::Single(crate::keybindings::Action::MoveRight)
        }));
        assert!(report.settings.sequence_bindings.iter().any(|binding| {
            binding.keys == vec![KeyInput::Char('z'), KeyInput::Char('u')]
                && binding.actions == ActionBinding::Single(crate::keybindings::Action::MoveDown)
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
                    && binding.actions
                        == ActionBinding::Single(crate::keybindings::Action::MoveRight))
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
                && binding.actions
                    == ActionBinding::Multiple(vec![Action::MoveDown, Action::MoveRight])
        }));
        assert!(report.settings.sequence_bindings.iter().any(|binding| {
            binding.keys == vec![KeyInput::Char('z'), KeyInput::Char('u')]
                && binding.actions
                    == ActionBinding::Multiple(vec![Action::MoveDown, Action::MoveRight])
        }));
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
}
