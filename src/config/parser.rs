//! Line-based TOML-like parser for Ordex configuration files.
//!
//! The parser is intentionally resilient: it keeps collecting sections/items and
//! records diagnostics for malformed lines instead of aborting on first error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed scalar values supported by the configuration format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedValue {
    String(String),
    Integer(i64),
    Boolean(bool),
}

/// One parsed key/value assignment inside a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedItem {
    pub(crate) key: String,
    pub(crate) value: ParsedValue,
    pub(crate) line: usize,
    pub(crate) line_content: String,
}

/// One named configuration section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSection {
    pub(crate) name: String,
    pub(crate) items: Vec<ParsedItem>,
}

/// Parser diagnostic categories emitted while processing input lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParserDiagnosticKind {
    InvalidHeader,
    InvalidAssignment,
    InvalidValue,
    UnterminatedString,
}

/// Location-aware parser diagnostic for invalid lines or values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserDiagnostic {
    pub(crate) kind: ParserDiagnosticKind,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) section: Option<String>,
    pub(crate) message: String,
    pub(crate) line_content: String,
}

/// Result of parsing one config source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedDocument {
    pub(crate) source_path: PathBuf,
    pub(crate) sections: Vec<ParsedSection>,
    pub(crate) diagnostics: Vec<ParserDiagnostic>,
}

/// Parse one TOML-like input string into sections/items and diagnostics.
pub(crate) fn parse_str(source_path: &Path, input: &str) -> ParsedDocument {
    let mut section_items: HashMap<String, Vec<ParsedItem>> = HashMap::new();
    let mut section_order: Vec<String> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current_section = String::from("root");
    section_order.push(current_section.clone());
    section_items.insert(current_section.clone(), Vec::new());

    for (line_idx, raw_line) in input.lines().enumerate() {
        let line_no = line_idx + 1;
        let without_comments = strip_comments(raw_line);
        let trimmed = without_comments.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('[') {
            match parse_header(trimmed) {
                Some(section_name) => {
                    current_section = section_name;
                    if !section_items.contains_key(&current_section) {
                        section_order.push(current_section.clone());
                        section_items.insert(current_section.clone(), Vec::new());
                    }
                }
                None => diagnostics.push(ParserDiagnostic {
                    kind: ParserDiagnosticKind::InvalidHeader,
                    line: line_no,
                    column: 1,
                    section: Some(current_section.clone()),
                    message: "Invalid section header".to_string(),
                    line_content: raw_line.to_string(),
                }),
            }
            continue;
        }

        let (key, value_raw, value_col) = match split_assignment(trimmed) {
            Ok(parts) => parts,
            Err(error) => {
                diagnostics.push(ParserDiagnostic {
                    kind: ParserDiagnosticKind::InvalidAssignment,
                    line: line_no,
                    column: error.column,
                    section: Some(current_section.clone()),
                    message: error.message,
                    line_content: raw_line.to_string(),
                });
                continue;
            }
        };

        match parse_value(value_raw) {
            Ok(value) => {
                if let Some(items) = section_items.get_mut(&current_section) {
                    items.push(ParsedItem {
                        key: key.to_string(),
                        value,
                        line: line_no,
                        line_content: raw_line.to_string(),
                    });
                }
            }
            Err(kind) => diagnostics.push(ParserDiagnostic {
                kind,
                line: line_no,
                column: value_col,
                section: Some(current_section.clone()),
                message: format!("Invalid value for key `{}`", key),
                line_content: raw_line.to_string(),
            }),
        }
    }

    let sections = section_order
        .into_iter()
        .filter_map(|name| {
            section_items
                .remove(&name)
                .map(|items| ParsedSection { name, items })
        })
        .collect();

    ParsedDocument {
        source_path: source_path.to_path_buf(),
        sections,
        diagnostics,
    }
}

fn parse_header(line: &str) -> Option<String> {
    if !(line.starts_with('[') && line.ends_with(']')) {
        return None;
    }
    let inner = line[1..line.len().saturating_sub(1)].trim();
    if inner.is_empty()
        || !inner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return None;
    }
    Some(inner.to_string())
}

#[derive(Debug)]
struct AssignmentError {
    column: usize,
    message: String,
}

fn split_assignment(line: &str) -> Result<(&str, &str, usize), AssignmentError> {
    let mut in_string = false;
    let mut escape = false;
    for (idx, c) in line.char_indices() {
        if c == '"' && !escape {
            in_string = !in_string;
        }
        escape = c == '\\' && !escape;
        if c == '=' && !in_string {
            let key = line[..idx].trim();
            let value = line[idx + 1..].trim();
            if key.is_empty() {
                return Err(AssignmentError {
                    column: 1,
                    message: "Missing key name before `=`".to_string(),
                });
            }
            if value.is_empty() {
                return Err(AssignmentError {
                    column: idx + 2,
                    message: "Missing value after `=`".to_string(),
                });
            }
            if let Some(quote_idx) = key.find('"') {
                return Err(AssignmentError {
                    column: quote_idx + 1,
                    message: "Unexpected `\"` in key name; keys must not be quoted".to_string(),
                });
            }
            if !key
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err(AssignmentError {
                    column: 1,
                    message: "Invalid key name; use letters, digits, `_`, or `-`".to_string(),
                });
            }
            return Ok((key, value, idx + 2));
        }
    }
    Err(AssignmentError {
        column: 1,
        message: "Missing `=` between key and value".to_string(),
    })
}

fn parse_value(value_raw: &str) -> Result<ParsedValue, ParserDiagnosticKind> {
    if value_raw.starts_with('"') {
        if value_raw.len() < 2 || !value_raw.ends_with('"') {
            return Err(ParserDiagnosticKind::UnterminatedString);
        }
        let mut out = String::new();
        let inner = &value_raw[1..value_raw.len().saturating_sub(1)];
        let mut escape = false;
        for c in inner.chars() {
            if escape {
                out.push(match c {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else {
                out.push(c);
            }
        }
        if escape {
            return Err(ParserDiagnosticKind::UnterminatedString);
        }
        return Ok(ParsedValue::String(out));
    }

    if value_raw.eq_ignore_ascii_case("true") {
        return Ok(ParsedValue::Boolean(true));
    }
    if value_raw.eq_ignore_ascii_case("false") {
        return Ok(ParsedValue::Boolean(false));
    }

    value_raw
        .parse::<i64>()
        .map(ParsedValue::Integer)
        .map_err(|_| ParserDiagnosticKind::InvalidValue)
}

/// Strip `#` comments when the marker is outside of quoted strings.
fn strip_comments(line: &str) -> &str {
    let mut in_string = false;
    let mut escape = false;
    for (idx, c) in line.char_indices() {
        if c == '"' && !escape {
            in_string = !in_string;
        }
        escape = c == '\\' && !escape;
        if c == '#' && !in_string {
            return &line[..idx];
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_values() {
        let input = r#"
[editor]
scroll_margin = 2
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let editor = doc
            .sections
            .iter()
            .find(|section| section.name == "editor")
            .expect("editor section");
        assert_eq!(editor.items.len(), 1);
    }

    #[test]
    fn ignores_hash_comments_outside_strings() {
        let input = r#"
[editor]
scroll_margin = 1 # trailing comment
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let editor = doc
            .sections
            .iter()
            .find(|section| section.name == "editor")
            .expect("editor section");
        assert_eq!(editor.items.len(), 1);
    }

    #[test]
    fn reports_invalid_assignment() {
        let input = r#"
[editor]
scroll_margin 3
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(
            doc.diagnostics[0].kind,
            ParserDiagnosticKind::InvalidAssignment
        );
        assert_eq!(
            doc.diagnostics[0].message,
            "Missing `=` between key and value"
        );
    }

    #[test]
    fn keeps_hash_inside_quoted_string() {
        let input = r#"
[editor]
title = "value # not a comment" # trailing comment
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let editor = doc
            .sections
            .iter()
            .find(|section| section.name == "editor")
            .expect("editor section");
        assert_eq!(editor.items.len(), 1);
        assert_eq!(
            editor.items[0].value,
            ParsedValue::String("value # not a comment".to_string())
        );
    }

    #[test]
    fn ignores_comment_only_lines() {
        let input = r#"
# top-level comment

[editor]
# section comment
scroll_margin = 3
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let editor = doc
            .sections
            .iter()
            .find(|section| section.name == "editor")
            .expect("editor section");
        assert_eq!(editor.items.len(), 1);
    }

    #[test]
    fn accepts_unicode_key_names() {
        let input = r#"
[keymap.normal]
é = "MoveRight"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let keymap = doc
            .sections
            .iter()
            .find(|section| section.name == "keymap.normal")
            .expect("keymap section");
        assert_eq!(keymap.items.len(), 1);
        assert_eq!(keymap.items[0].key, "é");
    }

    #[test]
    fn reports_quoted_key_name_error() {
        let input = r#"
[keymap.normal]
"é" = "MoveWordForward"
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(
            doc.diagnostics[0].message,
            "Unexpected `\"` in key name; keys must not be quoted"
        );
    }
}
