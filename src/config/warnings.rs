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
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
    pub(crate) line_content: Option<String>,
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
            line: None,
            column: None,
            line_content: None,
        }
    }

    /// Add line/column context for source-aware warning output.
    pub(crate) fn with_position(
        mut self,
        line: usize,
        column: Option<usize>,
        line_content: Option<String>,
    ) -> Self {
        self.line = Some(line);
        self.column = column;
        self.line_content = line_content;
        self
    }
}

/// Print startup warnings to stderr in a stable, human-readable format.
pub(crate) fn emit_startup_warnings(warnings: &[WarningEvent]) {
    for warning in warnings {
        let location = match (warning.line, warning.column) {
            (Some(line), Some(column)) => {
                format!("{}:{line}:{column}", warning.source_path.display())
            }
            (Some(line), None) => format!("{}:{line}", warning.source_path.display()),
            (None, _) => warning.source_path.display().to_string(),
        };
        let mut detail = String::new();
        if let Some(section) = warning.section.as_deref() {
            detail.push_str(&format!(" section `{section}`"));
        }
        if let Some(key) = warning.key.as_deref() {
            detail.push_str(&format!(" key `{key}`"));
        }
        eprintln!(
            "Configuration warning [{}]\n  {}\n  at {}{}",
            warning.code.as_str(),
            warning.message,
            location,
            detail
        );
        if let (Some(line), Some(line_content)) = (warning.line, warning.line_content.as_deref()) {
            eprintln!("  {line:>4} | {}", line_content.trim_end());
        }
    }
}
