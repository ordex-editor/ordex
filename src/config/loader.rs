//! End-to-end configuration loading orchestrator.

use crate::config::include_loader::{parse_config_file, resolve_include_path};
use crate::config::keymap_merge::{dedupe_bindings, dedupe_sequence_bindings};
use crate::config::validator::{
    ConfigSettings, ValidationReport, merge_validation_reports, validate_document,
};
use crate::config::warnings::{WarningCode, WarningEvent};
use std::path::Path;

/// Aggregated load report returned to startup code.
#[derive(Debug, Clone)]
pub(crate) struct LoadResultReport {
    pub(crate) startup_allowed: bool,
    pub(crate) applied_sections: Vec<String>,
    pub(crate) skipped_sections: Vec<String>,
    pub(crate) defaulted_keys: Vec<String>,
    pub(crate) ignored_unknown_keys: Vec<String>,
    pub(crate) warnings: Vec<WarningEvent>,
}

/// Final config settings and load report for one startup attempt.
#[derive(Debug, Clone)]
pub(crate) struct ConfigLoadOutcome {
    pub(crate) settings: ConfigSettings,
    pub(crate) report: LoadResultReport,
}

/// Load one main config file, process includes, and merge valid settings.
pub(crate) fn load_config(path: &Path) -> ConfigLoadOutcome {
    let mut aggregate = ValidationReport::default();

    let main_doc = match parse_config_file(path) {
        Ok(document) => document,
        Err(error) => {
            let warning = WarningEvent::new(
                WarningCode::InvalidSection,
                format!(
                    "Could not read config file `{}`; defaults used ({})",
                    path.display(),
                    error
                ),
                path,
                None,
                None,
            );
            return ConfigLoadOutcome {
                settings: ConfigSettings::default(),
                report: LoadResultReport {
                    startup_allowed: true,
                    applied_sections: Vec::new(),
                    skipped_sections: Vec::new(),
                    defaulted_keys: Vec::new(),
                    ignored_unknown_keys: Vec::new(),
                    warnings: vec![warning],
                },
            };
        }
    };

    let main_report = validate_document(&main_doc);
    let include_paths = main_report.settings.include_paths.clone();
    merge_validation_reports(&mut aggregate, main_report);

    // Includes are loaded after the main file so they can extend or override
    // settings while preserving recoverable startup on read failures.
    for include in include_paths {
        let include_path = resolve_include_path(path, &include.path);
        match parse_config_file(&include_path) {
            Ok(include_doc) => {
                let include_report = validate_document(&include_doc);
                merge_validation_reports(&mut aggregate, include_report);
            }
            Err(error) => {
                aggregate.warnings.push(
                    WarningEvent::new(
                        WarningCode::MissingInclude,
                        format!("Missing include `{}` ({})", include_path.display(), error),
                        &include_path,
                        Some("include".to_string()),
                        None,
                    )
                    .with_position(
                        include.line,
                        None,
                        Some(include.line_content.clone()),
                    ),
                );
                if !aggregate.skipped_sections.iter().any(|s| s == "include") {
                    aggregate.skipped_sections.push("include".to_string());
                }
            }
        }
    }

    let (deduped_bindings, dedupe_warnings) =
        dedupe_bindings(&aggregate.settings.key_bindings, path);
    aggregate.settings.key_bindings = deduped_bindings;
    aggregate.warnings.extend(dedupe_warnings);
    let (deduped_sequences, sequence_warnings) =
        dedupe_sequence_bindings(&aggregate.settings.sequence_bindings, path);
    aggregate.settings.sequence_bindings = deduped_sequences;
    aggregate.warnings.extend(sequence_warnings);

    ConfigLoadOutcome {
        settings: aggregate.settings.clone(),
        report: LoadResultReport {
            startup_allowed: true,
            applied_sections: aggregate.applied_sections,
            skipped_sections: aggregate.skipped_sections,
            defaulted_keys: aggregate.defaulted_keys,
            ignored_unknown_keys: aggregate.ignored_unknown_keys,
            warnings: aggregate.warnings,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ordex_cfg_test_{}_{}", std::process::id(), name))
    }

    #[test]
    fn missing_include_is_recoverable() {
        let path = temp_path("missing_include.cfg");
        fs::write(
            &path,
            r#"
[include]
extra = "does-not-exist.cfg"
"#,
        )
        .expect("write");
        let outcome = load_config(&path);
        assert!(outcome.report.startup_allowed);
        assert!(
            outcome
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == WarningCode::MissingInclude)
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn include_values_override_main_file_values() {
        let path = temp_path("include_override_main.cfg");
        let include_path = temp_path("include_override_extra.cfg");
        let include_name = include_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("include name");
        fs::write(
            &path,
            format!(
                r#"
[editor]
scroll_margin = 1

[include]
extra = "{include_name}"
"#
            ),
        )
        .expect("write main config");
        fs::write(
            &include_path,
            r#"
[editor]
scroll_margin = 4
"#,
        )
        .expect("write include config");

        let outcome = load_config(&path);
        assert_eq!(outcome.settings.scroll_margin, Some(4));

        let _ = fs::remove_file(path);
        let _ = fs::remove_file(include_path);
    }
}
