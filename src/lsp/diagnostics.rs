//! Normalized LSP diagnostics used by transport, state, and rendering code.

use super::protocol::LspRange;
use std::path::PathBuf;

/// One normalized LSP diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum LspDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

impl LspDiagnosticSeverity {
    /// Return the stable lowercase label used in UI surfaces.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "info",
            Self::Hint => "hint",
        }
    }

    /// Return the gutter marker shown for this severity.
    pub(crate) fn gutter_marker(self) -> char {
        match self {
            Self::Error => '!',
            Self::Warning => '^',
            Self::Information => 'i',
            Self::Hint => '~',
        }
    }

    /// Return the sort rank where smaller values are more severe.
    pub(crate) fn sort_rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Information => 2,
            Self::Hint => 3,
        }
    }
}

/// One normalized diagnostic for one document range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspDiagnostic {
    /// Zero-based diagnostic range in LSP coordinates.
    pub(crate) range: LspRange,
    /// Normalized severity used for sorting and rendering.
    pub(crate) severity: LspDiagnosticSeverity,
    /// Main human-readable diagnostic message.
    pub(crate) message: String,
    /// Optional language-server diagnostic source such as `rustc`.
    pub(crate) source: Option<String>,
    /// Optional machine-readable diagnostic code.
    pub(crate) code: Option<String>,
}

impl LspDiagnostic {
    /// Return the stable display label used by the diagnostics picker.
    pub(crate) fn display_label(&self) -> String {
        let mut label = format!(
            "{}:{} [{}] {}",
            self.range.start.line.saturating_add(1),
            self.range.start.character.saturating_add(1),
            self.severity.label(),
            self.message
        );
        if let Some(source) = &self.source {
            label.push_str(" · ");
            label.push_str(source);
        }
        if let Some(code) = &self.code {
            label.push_str(" (");
            label.push_str(code);
            label.push(')');
        }
        label
    }

    /// Return whether this diagnostic covers `line` and `character`.
    ///
    /// Returns `true` when the supplied zero-based LSP position falls within the
    /// diagnostic range, and `false` when the position lies outside the range.
    pub(crate) fn covers_position(&self, line: usize, character: usize) -> bool {
        let start = self.range.start;
        let end = self.range.end;
        if line < start.line || line > end.line {
            return false;
        }
        if start.line == end.line {
            let end_character = end.character.max(start.character.saturating_add(1));
            return line == start.line && (start.character..end_character).contains(&character);
        }
        if line == start.line {
            return character >= start.character;
        }
        if line == end.line {
            return character < end.character.max(1);
        }
        true
    }
}

/// One document-local diagnostics snapshot published by the language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspFileDiagnostics {
    /// Canonical filesystem path for the document.
    pub(crate) file_path: PathBuf,
    /// Optional document version attached to the publish event.
    pub(crate) version: Option<i32>,
    /// Ordered diagnostics for the document.
    pub(crate) diagnostics: Vec<LspDiagnostic>,
}

impl LspFileDiagnostics {
    /// Create one sorted diagnostics snapshot for a file.
    pub(crate) fn new(
        file_path: PathBuf,
        version: Option<i32>,
        mut diagnostics: Vec<LspDiagnostic>,
    ) -> Self {
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.range.start.line,
                diagnostic.range.start.character,
                diagnostic.severity.sort_rank(),
                diagnostic.message.clone(),
            )
        });
        Self {
            file_path,
            version,
            diagnostics,
        }
    }

    /// Return whether this file currently has no diagnostics.
    ///
    /// Returns `true` when the diagnostics list is empty, and `false` when the
    /// file still has at least one active diagnostic.
    pub(crate) fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Return the most severe diagnostic that starts on `line`, if any.
    pub(crate) fn line_severity(&self, line: usize) -> Option<LspDiagnosticSeverity> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.start.line == line)
            .map(|diagnostic| diagnostic.severity)
            .min()
    }

    /// Return the most severe diagnostic covering `line` and `character`, if any.
    pub(crate) fn severity_at_position(
        &self,
        line: usize,
        character: usize,
    ) -> Option<LspDiagnosticSeverity> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.covers_position(line, character))
            .map(|diagnostic| diagnostic.severity)
            .min()
    }

    /// Return the next diagnostic index after `line` and `character`, if any.
    pub(crate) fn next_index_after(&self, line: usize, character: usize) -> Option<usize> {
        self.diagnostics.iter().position(|diagnostic| {
            diagnostic.range.start.line > line
                || (diagnostic.range.start.line == line
                    && diagnostic.range.start.character > character)
        })
    }

    /// Return the previous diagnostic index before `line` and `character`, if any.
    pub(crate) fn previous_index_before(&self, line: usize, character: usize) -> Option<usize> {
        self.diagnostics
            .iter()
            .enumerate()
            .rev()
            .find(|(_, diagnostic)| {
                diagnostic.range.start.line < line
                    || (diagnostic.range.start.line == line
                        && diagnostic.range.start.character < character)
            })
            .map(|(index, _)| index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one diagnostic with the supplied start and end coordinates.
    fn diagnostic(
        line: usize,
        start: usize,
        end: usize,
        severity: LspDiagnosticSeverity,
    ) -> LspDiagnostic {
        LspDiagnostic {
            range: LspRange {
                start: super::super::protocol::LspPosition {
                    line,
                    character: start,
                },
                end: super::super::protocol::LspPosition {
                    line,
                    character: end,
                },
            },
            severity,
            message: format!("diagnostic-{line}-{start}"),
            source: None,
            code: None,
        }
    }

    /// Verify coverage queries respect the diagnostic range and severity ordering.
    #[test]
    fn test_file_diagnostics_report_line_and_position_severity() {
        let diagnostics = LspFileDiagnostics::new(
            PathBuf::from("/tmp/main.rs"),
            None,
            vec![
                diagnostic(1, 4, 9, LspDiagnosticSeverity::Warning),
                diagnostic(1, 4, 7, LspDiagnosticSeverity::Error),
            ],
        );

        assert_eq!(
            diagnostics.line_severity(1),
            Some(LspDiagnosticSeverity::Error)
        );
        assert_eq!(
            diagnostics.severity_at_position(1, 5),
            Some(LspDiagnosticSeverity::Error)
        );
        assert_eq!(
            diagnostics.severity_at_position(1, 8),
            Some(LspDiagnosticSeverity::Warning)
        );
        assert_eq!(diagnostics.severity_at_position(1, 12), None);
    }

    /// Verify next and previous navigation use the sorted diagnostic order.
    #[test]
    fn test_file_diagnostics_find_next_and_previous_indices() {
        let diagnostics = LspFileDiagnostics::new(
            PathBuf::from("/tmp/main.rs"),
            None,
            vec![
                diagnostic(2, 3, 8, LspDiagnosticSeverity::Hint),
                diagnostic(0, 5, 8, LspDiagnosticSeverity::Error),
                diagnostic(1, 2, 6, LspDiagnosticSeverity::Warning),
            ],
        );

        assert_eq!(diagnostics.next_index_after(0, 0), Some(0));
        assert_eq!(diagnostics.next_index_after(0, 5), Some(1));
        assert_eq!(diagnostics.next_index_after(2, 3), None);
        assert_eq!(diagnostics.previous_index_before(2, 3), Some(1));
        assert_eq!(diagnostics.previous_index_before(1, 2), Some(0));
        assert_eq!(diagnostics.previous_index_before(0, 5), None);
    }
}
