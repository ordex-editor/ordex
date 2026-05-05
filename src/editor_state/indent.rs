//! Manual indentation helpers for `EditorState`.

use super::*;
use crate::syntax::profile::{IndentationStyle, ManualIndentConfig};

/// Inclusive logical-line range targeted by one indent command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndentLineRange {
    start_line: usize,
    end_line: usize,
}

impl EditorState {
    /// Reindent the current Visual selection and return to Normal mode.
    pub(super) fn indent_visual_selection(&mut self) {
        let Some((selection, _kind)) = self.normalized_selection() else {
            return;
        };

        self.reindent_selection(selection);
        self.exit_visual_mode();
    }

    /// Reindent one operator-resolved selection range.
    ///
    /// Returns `true` when the current language exposes a manual indentation rule,
    /// and `false` when indentation is unsupported for the active file.
    pub(super) fn reindent_selection(&mut self, selection: SelectionRange) -> bool {
        let Some(profile) = self.active_manual_indent_profile() else {
            self.show_status_message("No manual indent rule for current language");
            return false;
        };
        let config = profile
            .manual_indent()
            .expect("manual indent profile should carry manual indent metadata");
        let line_range = self.indent_line_range(selection);
        let mut changed_any = false;

        // Reindent line-by-line inside one undo transaction so the whole command
        // replays, undoes, and redraws the same way as other editing operators.
        self.with_history_transaction(|editor| {
            for line_idx in line_range.start_line..=line_range.end_line {
                changed_any |= editor.reindent_one_line(line_idx, profile, config);
            }
            editor.move_cursor_to_first_non_blank(line_range.start_line);
        });

        if changed_any {
            self.status_message = None;
        }
        true
    }

    /// Return the active language profile when it exposes manual indent metadata.
    fn active_manual_indent_profile(
        &self,
    ) -> Option<&'static crate::syntax::profile::LanguageProfile> {
        let profile =
            detect_language_details(Some(self.file_path.as_path())).map(|(profile, _)| profile)?;
        profile.manual_indent().map(|_| profile)
    }

    /// Convert one character range into the inclusive logical lines it touches.
    fn indent_line_range(&self, selection: SelectionRange) -> IndentLineRange {
        let start_line = self
            .buffer
            .char_to_line(selection.start.min(self.buffer.chars_count()));

        // End positions are exclusive, so convert them back to the last covered
        // character before asking the buffer for its containing logical line.
        let end_line = if selection.end > selection.start {
            let last_char = selection
                .end
                .saturating_sub(1)
                .min(self.buffer.chars_count().saturating_sub(1));
            self.buffer.char_to_line(last_char)
        } else {
            start_line
        };
        IndentLineRange {
            start_line,
            end_line,
        }
    }

    /// Reindent one logical line according to the active style family.
    ///
    /// Returns `true` when the line's leading indentation changed, and `false`
    /// when the line was blank or already matched the desired indentation.
    fn reindent_one_line(
        &mut self,
        line_idx: usize,
        profile: &crate::syntax::profile::LanguageProfile,
        config: ManualIndentConfig,
    ) -> bool {
        let Some(line) = self.buffer.line_for_display_string(line_idx) else {
            return false;
        };
        if line.trim().is_empty() {
            return false;
        }

        let current_indent_chars = leading_indent_char_count(&line);
        let target_indent_columns = self.target_indent_columns(line_idx, profile, config);
        let (desired_tabs, desired_spaces) = split_indent_prefix(
            target_indent_columns,
            self.settings.indent_width,
            self.settings.indent_with_tabs,
        );
        if indent_prefix_matches(&line, current_indent_chars, desired_tabs, desired_spaces) {
            return false;
        }

        // The replacement only touches the leading indentation span so line
        // contents stay byte-for-byte identical after the prefix is rewritten.
        let line_start = self.buffer.line_to_char(line_idx);
        self.remove_buffer_range(line_start, line_start + current_indent_chars);
        self.insert_indent_prefix(line_start, desired_tabs, desired_spaces);
        true
    }

    /// Compute the target indentation width for one line.
    fn target_indent_columns(
        &self,
        line_idx: usize,
        profile: &crate::syntax::profile::LanguageProfile,
        config: ManualIndentConfig,
    ) -> usize {
        let previous_non_blank = self.previous_non_blank_line(line_idx).and_then(|index| {
            self.buffer
                .line_for_display_string(index)
                .map(|text| (index, text))
        });
        let current_line = self
            .buffer
            .line_for_display_string(line_idx)
            .unwrap_or_default();
        let mut target = previous_non_blank.as_ref().map_or(0, |(_, line)| {
            indent_columns(line, self.settings.indent_width)
        });

        // Each indentation family derives the base indent from the nearest
        // non-blank predecessor, then adjusts the current line relative to that
        // anchor according to the language's opening and closing cues.
        match config.style {
            IndentationStyle::CLike => {
                if previous_non_blank
                    .as_ref()
                    .is_some_and(|(_, line)| opens_c_like_block(line))
                {
                    target = target.saturating_add(self.settings.indent_width);
                }
                if starts_with_c_like_closer(&current_line) {
                    target = target.saturating_sub(self.settings.indent_width);
                }
                target
            }
            IndentationStyle::PythonLike => {
                if previous_non_blank
                    .as_ref()
                    .is_some_and(|(_, line)| opens_python_like_block(line))
                {
                    target = target.saturating_add(self.settings.indent_width);
                }
                if starts_with_python_dedent_keyword(&current_line, profile, config) {
                    target = target.saturating_sub(self.settings.indent_width);
                }
                target
            }
        }
    }

    /// Return the nearest earlier non-blank logical line, if any.
    fn previous_non_blank_line(&self, line_idx: usize) -> Option<usize> {
        // Blank lines do not carry indentation intent, so walk upward until one
        // line with visible content can anchor the current line's target indent.
        (0..line_idx).rev().find(|candidate| {
            self.buffer
                .line_for_display_string(*candidate)
                .is_some_and(|line| !line.trim().is_empty())
        })
    }

    /// Move the cursor to the first non-blank column of `line_idx`.
    fn move_cursor_to_first_non_blank(&mut self, line_idx: usize) {
        self.cursor = Cursor::new(line_idx, 0);
        self.move_first_non_blank();
    }

    /// Insert one normalized indentation prefix at `char_idx`.
    fn insert_indent_prefix(&mut self, char_idx: usize, tabs: usize, spaces: usize) {
        let mut insert_idx = char_idx;

        // The prefix is normalized as tabs followed by spaces so repeated manual
        // indent commands converge on one stable representation.
        if tabs > 0 {
            let text = "\t".repeat(tabs);
            insert_idx += text.chars().count();
            self.insert_buffer_text(char_idx, &text);
        }
        if spaces > 0 {
            let text = " ".repeat(spaces);
            self.insert_buffer_text(insert_idx, &text);
        }
    }
}

/// Return the number of leading indentation characters in `line`.
fn leading_indent_char_count(line: &str) -> usize {
    line.chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count()
}

/// Return the visual indentation width of the leading whitespace in `line`.
fn indent_columns(line: &str, indent_width: usize) -> usize {
    let mut columns = 0;

    // Leading tabs advance to the next configured indentation stop while spaces
    // advance by exactly one column.
    for ch in line.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => {
                let remainder = columns % indent_width;
                columns += if remainder == 0 {
                    indent_width
                } else {
                    indent_width - remainder
                };
            }
            _ => break,
        }
    }
    columns
}

/// Split one indentation width into normalized tab and space counts.
fn split_indent_prefix(
    columns: usize,
    indent_width: usize,
    indent_with_tabs: bool,
) -> (usize, usize) {
    if indent_with_tabs {
        return (columns / indent_width, columns % indent_width);
    }
    (0, columns)
}

/// Return whether the current leading whitespace already matches one normalized prefix.
fn indent_prefix_matches(line: &str, indent_chars: usize, tabs: usize, spaces: usize) -> bool {
    if indent_chars != tabs + spaces {
        return false;
    }
    let mut chars = line.chars().take(indent_chars);
    for _ in 0..tabs {
        if chars.next() != Some('\t') {
            return false;
        }
    }
    for _ in 0..spaces {
        if chars.next() != Some(' ') {
            return false;
        }
    }
    true
}

/// Return whether `line` opens one brace-oriented block for the following line.
fn opens_c_like_block(line: &str) -> bool {
    line.trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '{' | '[' | '('))
}

/// Return whether `line` begins with one closing brace-oriented delimiter.
fn starts_with_c_like_closer(line: &str) -> bool {
    line.trim_start_matches([' ', '\t'])
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '}' | ']' | ')'))
}

/// Return whether `line` opens one colon-oriented block for the following line.
fn opens_python_like_block(line: &str) -> bool {
    line.trim_end().ends_with(':')
}

/// Return whether `line` should outdent relative to the preceding Python block.
fn starts_with_python_dedent_keyword(
    line: &str,
    profile: &crate::syntax::profile::LanguageProfile,
    config: ManualIndentConfig,
) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    config
        .dedent_keywords
        .iter()
        .any(|keyword| starts_with_keyword(trimmed, keyword, profile))
}

/// Return whether `line` starts with `keyword` as a standalone token.
fn starts_with_keyword(
    line: &str,
    keyword: &str,
    profile: &crate::syntax::profile::LanguageProfile,
) -> bool {
    let Some(remainder) = line.strip_prefix(keyword) else {
        return false;
    };

    let Some(pattern) = profile.identifier else {
        return false;
    };
    remainder
        .chars()
        .next()
        .is_none_or(|ch| !identifier_can_continue(pattern, ch))
}
