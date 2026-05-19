//! Command-mode regex substitute parsing and replacement planning.

use crate::search::regex_input_for_byte_range;
use crate::text_buffer::TextBuffer;
use regex_cursor::engines::meta::Regex;
use regex_cursor::regex_automata::util::interpolate;

/// Describe which part of the active buffer a substitute command should mutate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubstituteScope {
    /// Replace matches only on the cursor's current logical line.
    CurrentLine,
    /// Replace matches across the entire active buffer.
    WholeFile,
}

impl SubstituteScope {
    /// Return the character-index range covered by this scope.
    pub(crate) fn char_range(self, buffer: &TextBuffer, current_line: usize) -> (usize, usize) {
        match self {
            Self::CurrentLine => {
                // Clamp the requested line first so command execution can safely
                // reuse the current cursor line even after external edits.
                let line = current_line.min(buffer.lines_count().saturating_sub(1));
                let start_char = buffer.line_to_char(line);
                let end_char = if line + 1 < buffer.lines_count() {
                    buffer.line_to_char(line + 1)
                } else {
                    buffer.chars_count()
                };
                (start_char, end_char)
            }
            Self::WholeFile => (0, buffer.chars_count()),
        }
    }
}

/// Parsed substitute command payload ready for execution planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubstituteCommand {
    pub(crate) scope: SubstituteScope,
    pub(crate) pattern: String,
    pub(crate) replacement: String,
}

/// Describe whether command-mode input can drive live substitute preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewSubstituteCommand {
    /// Input does not target `:s` or `:%s`, so substitute preview should stay inactive.
    NotSubstitute,
    /// Input starts like a substitute command but is still incomplete.
    Incomplete,
    /// Input looks like a substitute command but contains one invalid construct.
    Invalid(String),
    /// Input is valid enough to drive substitute preview and final execution planning.
    Ready(SubstituteCommand),
}

/// Describe why substitute parsing could not yet produce one executable command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SubstituteParseError {
    /// The user is still typing a substitute command, so preview should stay off.
    Incomplete(String),
    /// The input is not previewable or executable in its current form.
    Invalid(String),
}

/// One planned buffer edit produced by a substitute command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubstituteEdit {
    pub(crate) start_char: usize,
    pub(crate) end_char: usize,
    pub(crate) replacement: String,
}

/// One compiled substitute plan with stable global edit coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubstitutePlan {
    pattern: String,
    edits: Vec<SubstituteEdit>,
}

impl SubstitutePlan {
    /// Return the regex pattern that produced this plan.
    pub(crate) fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Return the stable edit list for this plan.
    pub(crate) fn edits(&self) -> &[SubstituteEdit] {
        &self.edits
    }

    /// Return the number of substitutions represented by this plan.
    pub(crate) fn substitution_count(&self) -> usize {
        self.edits.len()
    }
}

/// Parse one command-mode substitute input if it uses `:s` or `:%s` syntax.
pub(crate) fn parse_substitute_command(input: &str) -> Option<Result<SubstituteCommand, String>> {
    match parse_substitute_input(input) {
        PreviewSubstituteCommand::NotSubstitute => None,
        PreviewSubstituteCommand::Incomplete => {
            Some(Err(format_substitute_incomplete_error(input)))
        }
        PreviewSubstituteCommand::Invalid(error) => Some(Err(error)),
        PreviewSubstituteCommand::Ready(command) => Some(Ok(command)),
    }
}

/// Parse one command-mode input string for incremental substitute preview.
pub(crate) fn parse_preview_substitute_command(input: &str) -> PreviewSubstituteCommand {
    parse_substitute_input(input)
}

/// Parse one command-mode substitute input into preview-friendly state.
fn parse_substitute_input(input: &str) -> PreviewSubstituteCommand {
    let trimmed = input.trim();
    let (scope, body) = if let Some(body) = trimmed.strip_prefix("%s") {
        (SubstituteScope::WholeFile, body)
    } else if let Some(body) = trimmed.strip_prefix('s') {
        (SubstituteScope::CurrentLine, body)
    } else {
        return PreviewSubstituteCommand::NotSubstitute;
    };
    if body.is_empty() {
        return PreviewSubstituteCommand::Incomplete;
    }
    if body.chars().next().is_some_and(|delimiter| {
        delimiter.is_ascii_alphanumeric() || delimiter.is_ascii_whitespace()
    }) {
        return PreviewSubstituteCommand::NotSubstitute;
    }

    match parse_substitute_body(scope, body) {
        Ok(command) => PreviewSubstituteCommand::Ready(command),
        Err(SubstituteParseError::Incomplete(_)) => PreviewSubstituteCommand::Incomplete,
        Err(SubstituteParseError::Invalid(error)) => {
            PreviewSubstituteCommand::Invalid(format!("Invalid substitute: {error}"))
        }
    }
}

/// Build one substitute plan against the active buffer and current cursor line.
pub(crate) fn build_substitute_plan(
    command: &SubstituteCommand,
    buffer: &TextBuffer,
    current_line: usize,
) -> Result<SubstitutePlan, String> {
    let regex = Regex::new(&command.pattern).map_err(|error| format!("Invalid regex:\n{error}"))?;
    let (start_char, end_char) = command.scope.char_range(buffer, current_line);
    let scope_start_byte = buffer.char_to_byte(start_char);
    let scope_end_byte = buffer.char_to_byte(end_char);
    let mut edits = Vec::new();

    // Substitute scans the requested rope-backed buffer range directly, then
    // materializes only the capture spans referenced by the replacement
    // template for each individual match.
    for captures in regex.captures_iter(regex_input_for_byte_range(
        buffer,
        scope_start_byte,
        scope_end_byte,
    )) {
        let Some(found) = captures.get_match() else {
            continue;
        };
        let replacement = build_replacement_text(buffer, &command.replacement, &captures);
        edits.push(SubstituteEdit {
            start_char: buffer.byte_to_char(found.start()),
            end_char: buffer.byte_to_char(found.end()),
            replacement,
        });
    }

    Ok(SubstitutePlan {
        pattern: command.pattern.clone(),
        edits,
    })
}

/// Parse one substitute body after the `s` or `:%s` prefix.
fn parse_substitute_body(
    scope: SubstituteScope,
    body: &str,
) -> Result<SubstituteCommand, SubstituteParseError> {
    let mut chars = body.chars();
    let Some(delimiter) = chars.next() else {
        return Err(SubstituteParseError::Incomplete(
            "missing delimiter".to_string(),
        ));
    };
    if delimiter.is_ascii_alphanumeric() || delimiter.is_ascii_whitespace() {
        return Err(SubstituteParseError::Invalid(format!(
            "unsupported delimiter `{delimiter}`"
        )));
    }

    let body = &body[delimiter.len_utf8()..];
    let (pattern, remainder) = parse_substitute_segment(body, delimiter, true)?;
    let (replacement, remainder) = parse_substitute_segment(remainder, delimiter, false)?;
    if !remainder.is_empty() {
        return Err(SubstituteParseError::Invalid(format!(
            "unsupported suffix `{remainder}`"
        )));
    }

    Ok(SubstituteCommand {
        scope,
        pattern,
        replacement,
    })
}

/// Parse one substitute segment and optionally require a trailing delimiter.
fn parse_substitute_segment(
    input: &str,
    delimiter: char,
    require_closing_delimiter: bool,
) -> Result<(String, &str), SubstituteParseError> {
    let mut segment = String::new();
    let mut chars = input.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if ch == delimiter {
            return Ok((segment, &input[index + ch.len_utf8()..]));
        }
        if ch == '\\' {
            // Preserve regex and replacement escapes verbatim, except for the
            // substitute delimiter itself, which should lose the escape marker.
            if let Some((_, next)) = chars.next() {
                if next == delimiter {
                    segment.push(next);
                } else {
                    segment.push(ch);
                    segment.push(next);
                }
            } else {
                segment.push(ch);
            }
            continue;
        }
        segment.push(ch);
    }

    if require_closing_delimiter {
        Err(SubstituteParseError::Incomplete(format!(
            "missing closing delimiter `{delimiter}`"
        )))
    } else {
        Ok((segment, ""))
    }
}

/// Format the command-mode error used when Enter submits an incomplete substitute.
fn format_substitute_incomplete_error(input: &str) -> String {
    match parse_substitute_body_from_input(input) {
        Some(SubstituteParseError::Incomplete(error)) => format!("Invalid substitute: {error}"),
        Some(SubstituteParseError::Invalid(error)) => format!("Invalid substitute: {error}"),
        None => "Invalid substitute: missing delimiter".to_string(),
    }
}

/// Re-run substitute parsing to recover the specific incomplete error message.
fn parse_substitute_body_from_input(input: &str) -> Option<SubstituteParseError> {
    let trimmed = input.trim();
    let (_, body) = if let Some(body) = trimmed.strip_prefix("%s") {
        (SubstituteScope::WholeFile, body)
    } else if let Some(body) = trimmed.strip_prefix('s') {
        (SubstituteScope::CurrentLine, body)
    } else {
        return None;
    };
    if body.is_empty() {
        return Some(SubstituteParseError::Incomplete(
            "missing delimiter".to_string(),
        ));
    }
    if body.chars().next().is_some_and(|delimiter| {
        delimiter.is_ascii_alphanumeric() || delimiter.is_ascii_whitespace()
    }) {
        return None;
    }

    let scope = if trimmed.starts_with("%s") {
        SubstituteScope::WholeFile
    } else {
        SubstituteScope::CurrentLine
    };
    parse_substitute_body(scope, body).err()
}

/// Interpolate one replacement template against the current regex captures.
fn build_replacement_text(
    buffer: &TextBuffer,
    replacement: &str,
    captures: &regex_cursor::regex_automata::util::captures::Captures,
) -> String {
    let mut expanded = String::new();
    let Some(pattern_id) = captures.pattern() else {
        return expanded;
    };

    // The interpolation helper tokenizes the replacement template. The first
    // closure appends the text for a numeric capture index to the destination
    // string, while the second closure resolves a named capture to its index.
    interpolate::string(
        replacement,
        |index, dst| {
            let Some(span) = captures.get_group(index) else {
                return;
            };
            let start_char = buffer.byte_to_char(span.start);
            let end_char = buffer.byte_to_char(span.end);
            dst.push_str(&buffer.slice_string(start_char, end_char));
        },
        |name| captures.group_info().to_index(pattern_id, name),
        &mut expanded,
    );
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse current-line substitute commands with slash delimiters.
    #[test]
    fn test_parse_substitute_command_parses_current_line_scope() {
        assert_eq!(
            parse_substitute_command("s/foo/bar/"),
            Some(Ok(SubstituteCommand {
                scope: SubstituteScope::CurrentLine,
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
            }))
        );
    }

    /// Parse whole-file substitute commands with alternate delimiters and escapes.
    #[test]
    fn test_parse_substitute_command_parses_whole_file_scope_with_escaped_delimiter() {
        assert_eq!(
            parse_substitute_command(r"%s#foo\/bar#$1/#"),
            Some(Ok(SubstituteCommand {
                scope: SubstituteScope::WholeFile,
                pattern: r"foo\/bar".to_string(),
                replacement: "$1/".to_string(),
            }))
        );
    }

    /// Accept replacement segments when the trailing delimiter is omitted.
    #[test]
    fn test_parse_substitute_command_accepts_missing_final_delimiter() {
        assert_eq!(
            parse_substitute_command("s/foo/bar"),
            Some(Ok(SubstituteCommand {
                scope: SubstituteScope::CurrentLine,
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
            }))
        );
    }

    /// Treat missing closing delimiters as incomplete during preview parsing.
    #[test]
    fn test_parse_preview_substitute_command_marks_incomplete_input() {
        assert_eq!(
            parse_preview_substitute_command("s/foo"),
            PreviewSubstituteCommand::Incomplete
        );
        assert_eq!(
            parse_preview_substitute_command("%s"),
            PreviewSubstituteCommand::Incomplete
        );
    }

    /// Surface unsupported suffixes as invalid substitute preview input.
    #[test]
    fn test_parse_preview_substitute_command_rejects_invalid_suffix() {
        assert_eq!(
            parse_preview_substitute_command("s/foo/bar/g"),
            PreviewSubstituteCommand::Invalid(
                "Invalid substitute: unsupported suffix `g`".to_string()
            )
        );
    }

    /// Reject trailing flags because the first version intentionally omits them.
    #[test]
    fn test_parse_substitute_command_rejects_unsupported_suffix() {
        assert_eq!(
            parse_substitute_command("s/foo/bar/g"),
            Some(Err("Invalid substitute: unsupported suffix `g`".to_string()))
        );
    }

    /// Plan current-line substitutions without spilling into later lines.
    #[test]
    fn test_build_substitute_plan_limits_current_line_scope() {
        let buffer = TextBuffer::from_str("foo foo\nfoo\n");
        let command = SubstituteCommand {
            scope: SubstituteScope::CurrentLine,
            pattern: "foo".to_string(),
            replacement: "bar".to_string(),
        };

        let plan = build_substitute_plan(&command, &buffer, 0).expect("build substitute plan");

        assert_eq!(plan.substitution_count(), 2);
        assert_eq!(
            plan.edits(),
            &[
                SubstituteEdit {
                    start_char: 0,
                    end_char: 3,
                    replacement: "bar".to_string(),
                },
                SubstituteEdit {
                    start_char: 4,
                    end_char: 7,
                    replacement: "bar".to_string(),
                },
            ]
        );
    }

    /// Current-line scope should keep global coordinates correct away from line zero.
    #[test]
    fn test_build_substitute_plan_uses_global_offsets_for_later_lines() {
        let buffer = TextBuffer::from_str("skip\nfoo foo\n");
        let command = SubstituteCommand {
            scope: SubstituteScope::CurrentLine,
            pattern: "foo".to_string(),
            replacement: "bar".to_string(),
        };

        let plan = build_substitute_plan(&command, &buffer, 1).expect("build substitute plan");

        assert_eq!(plan.substitution_count(), 2);
        assert_eq!(plan.edits()[0].start_char, 5);
        assert_eq!(plan.edits()[1].start_char, 9);
    }

    /// Expand capture references in replacement text while planning edits.
    #[test]
    fn test_build_substitute_plan_expands_capture_references() {
        let buffer = TextBuffer::from_str("alpha-12\nbeta-7\n");
        let command = SubstituteCommand {
            scope: SubstituteScope::WholeFile,
            pattern: r"([a-z]+)-(\d+)".to_string(),
            replacement: "$2:$1".to_string(),
        };

        let plan = build_substitute_plan(&command, &buffer, 0).expect("build substitute plan");

        assert_eq!(plan.substitution_count(), 2);
        assert_eq!(plan.edits()[0].replacement, "12:alpha");
        assert_eq!(plan.edits()[1].replacement, "7:beta");
    }

    /// Expand named capture references without allocating the full scope text.
    #[test]
    fn test_build_substitute_plan_expands_named_capture_references() {
        let buffer = TextBuffer::from_str("alpha-12\n");
        let command = SubstituteCommand {
            scope: SubstituteScope::WholeFile,
            pattern: r"(?P<word>[a-z]+)-(?P<num>\d+)".to_string(),
            replacement: "${num}:${word}".to_string(),
        };

        let plan = build_substitute_plan(&command, &buffer, 0).expect("build substitute plan");

        assert_eq!(plan.substitution_count(), 1);
        assert_eq!(plan.edits()[0].replacement, "12:alpha");
    }
}
