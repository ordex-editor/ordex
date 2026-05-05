//! Command-mode regex substitute parsing and replacement planning.

use crate::text_buffer::TextBuffer;
use regex_cursor::Input as RegexInput;
use regex_cursor::engines::meta::Regex;

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
    let trimmed = input.trim();
    let (scope, body) = if let Some(body) = trimmed.strip_prefix("%s") {
        (SubstituteScope::WholeFile, body)
    } else if let Some(body) = trimmed.strip_prefix('s') {
        (SubstituteScope::CurrentLine, body)
    } else {
        return None;
    };
    if body.is_empty() {
        return Some(Err("Invalid substitute: missing delimiter".to_string()));
    }
    if body.chars().next().is_some_and(|delimiter| {
        delimiter.is_ascii_alphanumeric() || delimiter.is_ascii_whitespace()
    }) {
        return None;
    }

    Some(parse_substitute_body(scope, body).map_err(|error| format!("Invalid substitute: {error}")))
}

/// Build one substitute plan against the active buffer and current cursor line.
pub(crate) fn build_substitute_plan(
    command: &SubstituteCommand,
    buffer: &TextBuffer,
    current_line: usize,
) -> Result<SubstitutePlan, String> {
    let regex = Regex::new(&command.pattern).map_err(|error| format!("Invalid regex:\n{error}"))?;
    let (start_char, end_char) = command.scope.char_range(buffer, current_line);
    let scope_text = buffer.slice_string(start_char, end_char);
    let scope_start_byte = buffer.char_to_byte(start_char);
    let mut edits = Vec::new();

    // Capture iteration gives stable non-overlapping byte spans relative to the
    // contiguous scope snapshot. Convert each span back into global buffer
    // character coordinates before the caller mutates the rope.
    for captures in regex.captures_iter(RegexInput::new(scope_text.as_str())) {
        let Some(found) = captures.get_match() else {
            continue;
        };
        let mut replacement = String::new();
        captures.interpolate_string_into(
            scope_text.as_str(),
            &command.replacement,
            &mut replacement,
        );
        edits.push(SubstituteEdit {
            start_char: buffer.byte_to_char(scope_start_byte + found.start()),
            end_char: buffer.byte_to_char(scope_start_byte + found.end()),
            replacement,
        });
    }

    Ok(SubstitutePlan {
        pattern: command.pattern.clone(),
        edits,
    })
}

/// Parse one substitute body after the `s` or `:%s` prefix.
fn parse_substitute_body(scope: SubstituteScope, body: &str) -> Result<SubstituteCommand, String> {
    let mut chars = body.chars();
    let Some(delimiter) = chars.next() else {
        return Err("missing delimiter".to_string());
    };
    if delimiter.is_ascii_alphanumeric() || delimiter.is_ascii_whitespace() {
        return Err(format!("unsupported delimiter `{delimiter}`"));
    }

    let body = &body[delimiter.len_utf8()..];
    let (pattern, remainder) = parse_substitute_segment(body, delimiter)?;
    let (replacement, remainder) = parse_substitute_segment(remainder, delimiter)?;
    if !remainder.is_empty() {
        return Err(format!("unsupported suffix `{remainder}`"));
    }

    Ok(SubstituteCommand {
        scope,
        pattern,
        replacement,
    })
}

/// Parse one delimiter-terminated substitute segment.
fn parse_substitute_segment(input: &str, delimiter: char) -> Result<(String, &str), String> {
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

    Err(format!("missing closing delimiter `{delimiter}`"))
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
}
