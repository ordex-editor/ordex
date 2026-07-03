//! File-target parsing helpers for go-to-file style motions.

use crate::text_buffer::TextBuffer;
use std::path::{Path, PathBuf};

/// One parsed file target resolved from text under the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileTarget {
    /// Path text extracted from the token under the cursor.
    pub(crate) path_text: String,
    /// Optional one-based target line parsed from a `:line[:column]` suffix.
    pub(crate) line: Option<usize>,
    /// Optional one-based target column parsed from a `:line:column` suffix.
    pub(crate) column: Option<usize>,
}

/// Detailed resolution result for one parsed file target path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileTargetPathResolution {
    Resolved(PathBuf),
    MissingPath,
    MissingHomeDirectory,
    MissingWorkingDirectory,
}

/// Parse one filename-like token under `cursor_char_idx`.
///
/// Returns `Some(FileTarget)` when the cursor sits on or immediately after one
/// token that looks like a filename, and `None` when no such token is present.
pub(crate) fn find_file_target(
    buffer: &TextBuffer,
    cursor_char_idx: usize,
    allow_position_suffix: bool,
) -> Option<FileTarget> {
    let anchor = find_token_anchor(buffer, cursor_char_idx)?;
    let start = expand_token_start(buffer, anchor);
    let end = expand_token_end(buffer, anchor);
    let token = buffer.slice_string(start, end + 1);
    if token.is_empty() {
        return None;
    }

    if allow_position_suffix {
        return Some(parse_file_target_suffix(&token));
    }
    Some(FileTarget {
        path_text: token,
        line: None,
        column: None,
    })
}

/// Resolve one parsed file target and preserve failure context for UI messaging.
pub(crate) fn resolve_file_target_path_detailed(
    active_file_path: Option<&Path>,
    path_text: &str,
) -> FileTargetPathResolution {
    if path_text.is_empty() {
        return FileTargetPathResolution::MissingPath;
    }
    if path_text.starts_with('/') {
        return FileTargetPathResolution::Resolved(PathBuf::from(path_text));
    }
    if let Some(home_relative) = path_text.strip_prefix("~/") {
        let Some(home) = std::env::home_dir() else {
            return FileTargetPathResolution::MissingHomeDirectory;
        };
        return FileTargetPathResolution::Resolved(home.join(home_relative));
    }

    let Some(current_dir) = std::env::current_dir().ok() else {
        return FileTargetPathResolution::MissingWorkingDirectory;
    };
    // Relative paths follow the active buffer directory when available, and
    // otherwise fall back to the current process directory for unnamed buffers.
    let base_directory = active_file_path
        .filter(|path| !path.as_os_str().is_empty())
        .and_then(|path| {
            if path.is_absolute() {
                path.parent().map(Path::to_path_buf)
            } else {
                current_dir.join(path).parent().map(Path::to_path_buf)
            }
        })
        .unwrap_or(current_dir);
    FileTargetPathResolution::Resolved(base_directory.join(path_text))
}

/// Return whether `c` belongs to one filename-like token.
///
/// Returns `true` for non-whitespace characters that commonly appear in paths,
/// and `false` for surrounding delimiters that should stop token expansion.
fn is_file_target_char(c: char) -> bool {
    !c.is_whitespace()
        && !matches!(
            c,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';'
        )
}

/// Find the character index that should anchor token expansion.
///
/// Returns `Some(idx)` when the cursor sits on or just after a filename-like
/// token character, and `None` when neither side is part of a token.
fn find_token_anchor(buffer: &TextBuffer, cursor_char_idx: usize) -> Option<usize> {
    if buffer
        .char_at(cursor_char_idx)
        .is_some_and(is_file_target_char)
    {
        return Some(cursor_char_idx);
    }
    cursor_char_idx
        .checked_sub(1)
        .filter(|idx| buffer.char_at(*idx).is_some_and(is_file_target_char))
}

/// Expand left to the first character in the token that contains `anchor`.
fn expand_token_start(buffer: &TextBuffer, anchor: usize) -> usize {
    let mut start = anchor;
    // Scan left until the token boundary is reached so punctuation outside the
    // path, such as quotes or commas, stays out of the extracted target.
    while start > 0 && buffer.char_at(start - 1).is_some_and(is_file_target_char) {
        start -= 1;
    }
    start
}

/// Expand right to the last character in the token that contains `anchor`.
fn expand_token_end(buffer: &TextBuffer, anchor: usize) -> usize {
    let mut end = anchor;
    // Scan right for the same token boundary used by the left expansion so the
    // parsed target stays symmetric around the cursor position.
    while buffer.char_at(end + 1).is_some_and(is_file_target_char) {
        end += 1;
    }
    end
}

/// Parse one optional trailing `:line[:column]` suffix from `token`.
fn parse_file_target_suffix(token: &str) -> FileTarget {
    let Some((path_and_line, trailing_value)) = split_numeric_suffix(token) else {
        return FileTarget {
            path_text: token.to_string(),
            line: None,
            column: None,
        };
    };
    let Some((path_text, line)) = split_numeric_suffix(path_and_line) else {
        return FileTarget {
            path_text: path_and_line.to_string(),
            line: Some(trailing_value),
            column: None,
        };
    };
    if path_text.is_empty() {
        return FileTarget {
            path_text: token.to_string(),
            line: None,
            column: None,
        };
    }

    FileTarget {
        path_text: path_text.to_string(),
        line: Some(line),
        column: Some(trailing_value),
    }
}

/// Split one trailing `:<digits>` suffix from `text`.
///
/// Returns `Some((prefix, value))` when the suffix is present and non-zero, and
/// `None` when the trailing segment is missing or not numeric.
fn split_numeric_suffix(text: &str) -> Option<(&str, usize)> {
    let (prefix, suffix) = text.rsplit_once(':')?;
    let value = suffix.parse::<usize>().ok()?;
    (value > 0).then_some((prefix, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// File targets should be detected when the cursor sits on a bare filename.
    fn test_find_file_target_accepts_bare_filename() {
        let buffer = TextBuffer::from_str("include helper.rs here");

        let target = find_file_target(&buffer, 10, false).expect("target should parse");

        assert_eq!(
            target,
            FileTarget {
                path_text: "helper.rs".to_string(),
                line: None,
                column: None,
            }
        );
    }

    #[test]
    /// File targets should treat the previous character as the anchor at token boundaries.
    fn test_find_file_target_accepts_cursor_after_token() {
        let buffer = TextBuffer::from_str("./src/main.rs ");

        let target = find_file_target(&buffer, 13, false).expect("target should parse");

        assert_eq!(target.path_text, "./src/main.rs");
    }

    #[test]
    /// `gF` parsing should extract one-based line and column suffixes.
    fn test_find_file_target_parses_line_and_column_suffix() {
        let buffer = TextBuffer::from_str("open src/lib.rs:12:4");

        let target = find_file_target(&buffer, 10, true).expect("target should parse");

        assert_eq!(
            target,
            FileTarget {
                path_text: "src/lib.rs".to_string(),
                line: Some(12),
                column: Some(4),
            }
        );
    }

    #[test]
    /// `gF` parsing should also accept one-based line-only suffixes.
    fn test_find_file_target_parses_line_only_suffix() {
        let buffer = TextBuffer::from_str("open src/lib.rs:12");

        let target = find_file_target(&buffer, 10, true).expect("target should parse");

        assert_eq!(
            target,
            FileTarget {
                path_text: "src/lib.rs".to_string(),
                line: Some(12),
                column: None,
            }
        );
    }

    #[test]
    /// `gF` parsing should leave non-numeric suffixes attached to the filename.
    fn test_find_file_target_keeps_non_numeric_suffixes_literal() {
        let buffer = TextBuffer::from_str("open file.rs:main");

        let target = find_file_target(&buffer, 7, true).expect("target should parse");

        assert_eq!(target.path_text, "file.rs:main");
        assert_eq!(target.line, None);
        assert_eq!(target.column, None);
    }

    #[test]
    /// Relative file targets should resolve from the active buffer directory.
    fn test_resolve_file_target_path_uses_active_buffer_directory() {
        let resolved =
            resolve_file_target_path_detailed(Some(Path::new("/tmp/src/main.rs")), "lib.rs");

        assert_eq!(
            resolved,
            FileTargetPathResolution::Resolved(PathBuf::from("/tmp/src/lib.rs"))
        );
    }
}
