//! Warning model and startup warning rendering for configuration loading.

use std::path::{Path, PathBuf};

/// Warning category emitted during parsing, validation, or include loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarningCode {
    UnknownKey,
    InvalidValue,
    InvalidSection,
    DuplicateKeymap,
    MissingInclude,
}

impl WarningCode {
    fn as_str(self) -> &'static str {
        match self {
            WarningCode::UnknownKey => "unknown-key",
            WarningCode::InvalidValue => "invalid-value",
            WarningCode::InvalidSection => "invalid-section",
            WarningCode::DuplicateKeymap => "duplicate-keymap",
            WarningCode::MissingInclude => "missing-include",
        }
    }
}

/// One warning event with source metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WarningEvent {
    pub(crate) code: WarningCode,
    pub(crate) message: String,
    pub(crate) source_path: PathBuf,
    pub(crate) section: Option<String>,
    pub(crate) key: Option<String>,
}

impl WarningEvent {
    /// Build a warning event with consistent source metadata.
    pub(crate) fn new(
        code: WarningCode,
        message: impl Into<String>,
        source_path: &Path,
        section: Option<String>,
        key: Option<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source_path: source_path.to_path_buf(),
            section,
            key,
        }
    }
}

/// Print startup warnings to stderr in a stable, human-readable format.
pub(crate) fn emit_startup_warnings(warnings: &[WarningEvent]) {
    for warning in warnings {
        let section = warning
            .section
            .as_deref()
            .map(|value| format!(" section={}", value))
            .unwrap_or_default();
        let key = warning
            .key
            .as_deref()
            .map(|value| format!(" key={}", value))
            .unwrap_or_default();
        eprintln!(
            "Config warning [{}] {} (source: {}{}{})",
            warning.code.as_str(),
            warning.message,
            warning.source_path.display(),
            section,
            key
        );
    }
}
