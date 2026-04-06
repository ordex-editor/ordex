//! Shared line-based TOML-like parser used by Ordex file formats.
//!
//! The parser is intentionally resilient: it keeps collecting sections/items and
//! records diagnostics for malformed lines instead of aborting on first error.

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

/// Parsed scalar values supported by the TOML-like format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedValue {
    String(String),
    StringArray(Vec<String>),
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

/// One named document section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSection {
    pub(crate) name: String,
    pub(crate) header_line: Option<usize>,
    pub(crate) header_line_content: Option<String>,
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

/// Result of parsing one TOML-like source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedDocument {
    pub(crate) source_path: PathBuf,
    pub(crate) sections: Vec<ParsedSection>,
    pub(crate) diagnostics: Vec<ParserDiagnostic>,
}

/// Source metadata recorded for one section header.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionHeader {
    line: Option<usize>,
    line_content: Option<String>,
}

/// One logical assignment assembled from one or more physical input lines.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogicalLine {
    line_no: usize,
    raw_line: String,
    stripped_line: String,
}

/// State machine used while scanning array values across strings and comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayBracketState {
    Code,
    String,
    EscapedString,
    Comment,
}

/// State machine used while stripping line comments without touching quoted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentStripState {
    Code,
    String,
    EscapedString,
    Comment,
}

/// Incremental parser state shared by string and reader-based entry points.
struct ParserState {
    section_items: HashMap<String, Vec<ParsedItem>>,
    section_headers: HashMap<String, SectionHeader>,
    section_order: Vec<String>,
    diagnostics: Vec<ParserDiagnostic>,
    current_section: String,
}

impl ParserState {
    /// Create parser state with the implicit root section already registered.
    fn new() -> Self {
        let current_section = String::from("root");
        let mut section_items = HashMap::new();
        section_items.insert(current_section.clone(), Vec::new());
        let mut section_headers = HashMap::new();
        section_headers.insert(
            current_section.clone(),
            SectionHeader {
                line: None,
                line_content: None,
            },
        );

        Self {
            section_items,
            section_headers,
            section_order: vec![current_section.clone()],
            diagnostics: Vec::new(),
            current_section,
        }
    }

    /// Finalize accumulated parser state into the public parsed document model.
    fn finish(mut self, source_path: &Path) -> ParsedDocument {
        let sections = self
            .section_order
            .into_iter()
            .filter_map(|name| {
                let header = self.section_headers.remove(&name).unwrap_or(SectionHeader {
                    line: None,
                    line_content: None,
                });
                self.section_items.remove(&name).map(|items| ParsedSection {
                    name,
                    header_line: header.line,
                    header_line_content: header.line_content,
                    items,
                })
            })
            .collect();

        ParsedDocument {
            source_path: source_path.to_path_buf(),
            sections,
            diagnostics: self.diagnostics,
        }
    }
}

/// Parse one TOML-like input string into sections, items, and diagnostics.
#[cfg(test)]
pub(crate) fn parse_str(source_path: &Path, input: &str) -> ParsedDocument {
    let mut state = ParserState::new();
    let mut pending = None;
    for (line_idx, raw_line) in input.lines().enumerate() {
        consume_document_line(&mut state, &mut pending, line_idx + 1, raw_line);
    }
    flush_pending_logical_line(&mut state, pending);
    state.finish(source_path)
}

/// Parse one UTF-8 document reader without buffering the full file in memory.
pub(crate) fn parse_reader<R: BufRead>(
    source_path: &Path,
    reader: R,
) -> io::Result<ParsedDocument> {
    let mut state = ParserState::new();
    let mut pending = None;
    for (line_idx, raw_line) in reader.lines().enumerate() {
        consume_document_line(&mut state, &mut pending, line_idx + 1, &raw_line?);
    }
    flush_pending_logical_line(&mut state, pending);
    Ok(state.finish(source_path))
}

/// Parse one logical document line and merge its effects into the shared state.
fn parse_line(state: &mut ParserState, line_no: usize, raw_line: &str, stripped_line: &str) {
    let trimmed = stripped_line.trim();
    if trimmed.is_empty() {
        return;
    }

    if trimmed.starts_with('[') {
        match parse_header(trimmed) {
            Some(section_name) => {
                state.current_section = section_name;
                if !state.section_items.contains_key(&state.current_section) {
                    state.section_order.push(state.current_section.clone());
                    state
                        .section_items
                        .insert(state.current_section.clone(), Vec::new());
                    // Keep the header location so later validation can point to
                    // the section declaration instead of the first item inside it.
                    state.section_headers.insert(
                        state.current_section.clone(),
                        SectionHeader {
                            line: Some(line_no),
                            line_content: Some(raw_line.to_string()),
                        },
                    );
                }
            }
            None => state.diagnostics.push(ParserDiagnostic {
                kind: ParserDiagnosticKind::InvalidHeader,
                line: line_no,
                column: 1,
                section: Some(state.current_section.clone()),
                message: "Invalid section header".to_string(),
                line_content: raw_line.to_string(),
            }),
        }
    } else {
        // Only an unquoted `=` splits the assignment, so quoted strings can
        // legitimately contain `=` characters in their value.
        let (key, value_raw, value_col) = match split_assignment(trimmed) {
            Ok(parts) => parts,
            Err(error) => {
                state.diagnostics.push(ParserDiagnostic {
                    kind: ParserDiagnosticKind::InvalidAssignment,
                    line: line_no,
                    column: error.column,
                    section: Some(state.current_section.clone()),
                    message: error.message,
                    line_content: raw_line.to_string(),
                });
                return;
            }
        };

        match parse_value(value_raw) {
            Ok(value) => {
                let section_name = state.current_section.clone();
                let items = state
                    .section_items
                    .entry(section_name.clone())
                    .or_insert_with(|| {
                        state.section_order.push(section_name.clone());
                        // Re-create the current section on demand so a parsed
                        // value is never silently dropped if the section map
                        // gets out of sync.
                        state.section_headers.entry(section_name.clone()).or_insert(
                            SectionHeader {
                                line: None,
                                line_content: None,
                            },
                        );
                        Vec::new()
                    });
                items.push(ParsedItem {
                    key: key.to_string(),
                    value,
                    line: line_no,
                    line_content: raw_line.to_string(),
                });
            }
            Err(kind) => {
                let message = match kind {
                    ParserDiagnosticKind::UnterminatedString => {
                        format!("Missing closing `\"` for string value of key `{}`", key)
                    }
                    _ => format!("Invalid value for key `{}`", key),
                };
                state.diagnostics.push(ParserDiagnostic {
                    kind,
                    line: line_no,
                    column: value_col,
                    section: Some(state.current_section.clone()),
                    message,
                    line_content: raw_line.to_string(),
                });
            }
        }
    }
}

/// Merge one physical input line into the current logical line buffer.
fn consume_document_line(
    state: &mut ParserState,
    pending: &mut Option<LogicalLine>,
    line_no: usize,
    raw_line: &str,
) {
    if let Some(logical_line) = pending.as_mut() {
        logical_line.raw_line.push('\n');
        logical_line.raw_line.push_str(raw_line);
        logical_line.stripped_line.push('\n');
        append_line_without_comments(&mut logical_line.stripped_line, raw_line);
        if logical_line_is_complete(&logical_line.stripped_line) {
            let logical_line = pending.take().expect("pending logical line");
            parse_line(
                state,
                logical_line.line_no,
                &logical_line.raw_line,
                &logical_line.stripped_line,
            );
        }
        return;
    }

    let mut stripped_line = String::new();
    append_line_without_comments(&mut stripped_line, raw_line);
    if line_starts_multiline_array(&stripped_line) {
        *pending = Some(LogicalLine {
            line_no,
            raw_line: raw_line.to_string(),
            stripped_line,
        });
        return;
    }

    parse_line(state, line_no, raw_line, &stripped_line);
}

/// Flush one unfinished logical line at end of input.
fn flush_pending_logical_line(state: &mut ParserState, pending: Option<LogicalLine>) {
    if let Some(logical_line) = pending {
        parse_line(
            state,
            logical_line.line_no,
            &logical_line.raw_line,
            &logical_line.stripped_line,
        );
    }
}

/// Return whether `stripped_line` starts one multiline array assignment.
///
/// Returns `true` when the line opens an array assignment whose closing bracket
/// has not appeared yet, and `false` for every other line shape.
fn line_starts_multiline_array(stripped_line: &str) -> bool {
    let trimmed = stripped_line.trim();
    let Ok((_, value_raw, _)) = split_assignment(trimmed) else {
        return false;
    };
    value_raw.starts_with('[') && !logical_line_is_complete(stripped_line)
}

/// Return whether the current logical line has a complete array value.
///
/// Returns `true` when all array brackets are balanced outside strings/comments,
/// and `false` when the parser still expects later lines to close the array.
fn logical_line_is_complete(stripped_line: &str) -> bool {
    let trimmed = stripped_line.trim();
    let Ok((_, value_raw, _)) = split_assignment(trimmed) else {
        return true;
    };
    if !value_raw.starts_with('[') {
        return true;
    }
    array_bracket_depth(value_raw) == 0
}

/// Count unmatched array brackets while ignoring quoted strings and comments.
fn array_bracket_depth(value_raw: &str) -> usize {
    let mut depth = 0_usize;
    let mut state = ArrayBracketState::Code;

    // Multiline arrays keep line breaks inside the value, so comment handling
    // must reset at each newline instead of treating the whole value as one line.
    for c in value_raw.chars() {
        state = match (state, c) {
            (ArrayBracketState::Comment, '\n') => ArrayBracketState::Code,
            (ArrayBracketState::Comment, _) => ArrayBracketState::Comment,
            (ArrayBracketState::Code, '"') => ArrayBracketState::String,
            (ArrayBracketState::Code, '#') => ArrayBracketState::Comment,
            (ArrayBracketState::Code, '[') => {
                depth += 1;
                ArrayBracketState::Code
            }
            (ArrayBracketState::Code, ']') => {
                depth = depth.saturating_sub(1);
                ArrayBracketState::Code
            }
            (ArrayBracketState::Code, _) => ArrayBracketState::Code,
            (ArrayBracketState::String, '\\') => ArrayBracketState::EscapedString,
            (ArrayBracketState::String, '"') => ArrayBracketState::Code,
            (ArrayBracketState::String, _) => ArrayBracketState::String,
            (ArrayBracketState::EscapedString, _) => ArrayBracketState::String,
        };
    }
    depth
}

/// Append `raw_line` without comments to `target`.
fn append_line_without_comments(target: &mut String, raw_line: &str) {
    let mut state = CommentStripState::Code;
    for c in raw_line.chars() {
        state = match (state, c) {
            (CommentStripState::Comment, _) => CommentStripState::Comment,
            (CommentStripState::Code, '#') => CommentStripState::Comment,
            (CommentStripState::Code, '"') => {
                target.push(c);
                CommentStripState::String
            }
            (CommentStripState::Code, _) => {
                target.push(c);
                CommentStripState::Code
            }
            (CommentStripState::String, '\\') => {
                target.push(c);
                CommentStripState::EscapedString
            }
            (CommentStripState::String, '"') => {
                target.push(c);
                CommentStripState::Code
            }
            (CommentStripState::String, _) => {
                target.push(c);
                CommentStripState::String
            }
            (CommentStripState::EscapedString, _) => {
                target.push(c);
                CommentStripState::String
            }
        };
    }
}

/// Parse a section header like `[editor]` into its normalized section name.
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

/// Split one `key = value` line while ignoring `=` characters inside strings.
fn split_assignment(line: &str) -> Result<(&str, &str, usize), AssignmentError> {
    let mut in_string = false;
    let mut escape = false;
    for (idx, c) in line.char_indices() {
        // Keep track of whether the current character is inside a quoted string
        // so we only split on the assignment operator that belongs to the line.
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

/// Parse one scalar value supported by the TOML-like format.
fn parse_value(value_raw: &str) -> Result<ParsedValue, ParserDiagnosticKind> {
    if value_raw.starts_with('[') {
        return parse_string_array(value_raw).map(ParsedValue::StringArray);
    }
    if value_raw.starts_with('"') {
        return parse_string(value_raw).map(ParsedValue::String);
    }

    // Booleans are intentionally case-sensitive to keep the accepted surface
    // area small and predictable for this TOML-like format.
    if value_raw == "true" {
        return Ok(ParsedValue::Boolean(true));
    }
    if value_raw == "false" {
        return Ok(ParsedValue::Boolean(false));
    }

    // Anything else must parse as an integer; otherwise the value is invalid.
    value_raw
        .parse::<i64>()
        .map(ParsedValue::Integer)
        .map_err(|_| ParserDiagnosticKind::InvalidValue)
}

/// Parse one quoted string literal supported by the TOML-like format.
fn parse_string(value_raw: &str) -> Result<String, ParserDiagnosticKind> {
    if value_raw.len() < 2 || !value_raw.ends_with('"') {
        return Err(ParserDiagnosticKind::UnterminatedString);
    }
    let mut out = String::new();
    let inner = &value_raw[1..value_raw.len().saturating_sub(1)];
    let mut escape = false;
    for c in inner.chars() {
        // Strings support a small escape set; unknown escapes are preserved as
        // their escaped character so the parser stays permissive.
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
    Ok(out)
}

/// Parse an array of quoted strings such as `["move-down", "move-right"]`.
fn parse_string_array(value_raw: &str) -> Result<Vec<String>, ParserDiagnosticKind> {
    if value_raw.len() < 2 || !value_raw.ends_with(']') {
        return Err(ParserDiagnosticKind::InvalidValue);
    }

    let inner = &value_raw[1..value_raw.len().saturating_sub(1)];
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut values = Vec::new();
    let mut in_string = false;
    let mut expect_value = true;
    let mut string_start = None;
    let mut allow_trailing_comma = false;

    for (idx, c) in inner.char_indices() {
        if in_string {
            if c == '"' {
                let start = string_start.expect("string start tracked while parsing array");
                let end = idx + c.len_utf8();
                values.push(parse_string(&inner[start..end])?);
                in_string = false;
                string_start = None;
                expect_value = false;
                allow_trailing_comma = false;
            }
            continue;
        }

        if c.is_whitespace() {
            continue;
        }

        if expect_value {
            if c == '"' {
                in_string = true;
                string_start = Some(idx);
                allow_trailing_comma = false;
            } else {
                return Err(ParserDiagnosticKind::InvalidValue);
            }
        } else if c == ',' {
            expect_value = true;
            allow_trailing_comma = true;
        } else {
            return Err(ParserDiagnosticKind::InvalidValue);
        }
    }

    if in_string {
        return Err(ParserDiagnosticKind::UnterminatedString);
    }
    if expect_value && !allow_trailing_comma {
        return Err(ParserDiagnosticKind::InvalidValue);
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
    fn reports_missing_closing_quote_for_string_values() {
        let input = r#"
[editor]
theme = "nord
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(
            doc.diagnostics[0].kind,
            ParserDiagnosticKind::UnterminatedString
        );
        assert_eq!(
            doc.diagnostics[0].message,
            "Missing closing `\"` for string value of key `theme`"
        );
    }

    #[test]
    fn parses_multiline_string_arrays() {
        let input = r#"
[swap]
exclude = [
  "*.gpg",
  "/dev/shm/gopass*",
]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let swap = doc
            .sections
            .iter()
            .find(|section| section.name == "swap")
            .expect("swap section");
        assert_eq!(
            swap.items,
            vec![ParsedItem {
                key: "exclude".to_string(),
                value: ParsedValue::StringArray(vec![
                    "*.gpg".to_string(),
                    "/dev/shm/gopass*".to_string(),
                ]),
                line: 3,
                line_content: "exclude = [\n  \"*.gpg\",\n  \"/dev/shm/gopass*\",\n]".to_string(),
            }]
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

    #[test]
    fn boolean_values_are_case_sensitive() {
        let input = r#"
[editor]
enabled = TRUE
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(doc.diagnostics[0].kind, ParserDiagnosticKind::InvalidValue);
        assert_eq!(
            doc.diagnostics[0].message,
            "Invalid value for key `enabled`"
        );
    }

    #[test]
    fn parses_string_arrays() {
        let input = r#"
[keymap.normal]
z = ["move-down", "move-right"]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert!(doc.diagnostics.is_empty());
        let keymap = doc
            .sections
            .iter()
            .find(|section| section.name == "keymap.normal")
            .expect("keymap section");
        assert_eq!(
            keymap.items[0].value,
            ParsedValue::StringArray(vec!["move-down".to_string(), "move-right".to_string()])
        );
    }

    #[test]
    fn rejects_malformed_string_arrays() {
        let input = r#"
[keymap.normal]
z = ["move-down", move-right]
"#;
        let doc = parse_str(Path::new("test.cfg"), input);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(doc.diagnostics[0].kind, ParserDiagnosticKind::InvalidValue);
    }

    #[test]
    fn parse_reader_streams_line_by_line() {
        let reader = Cursor::new(
            r#"
[editor]
scroll_margin = 2
"#,
        );
        let doc = parse_reader(Path::new("test.cfg"), reader).expect("parse from reader");
        assert!(doc.diagnostics.is_empty());
        let editor = doc
            .sections
            .iter()
            .find(|section| section.name == "editor")
            .expect("editor section");
        assert_eq!(editor.items.len(), 1);
    }
}
