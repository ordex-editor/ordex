//! Command-mode completion modeling and candidate collection.

use crate::session::default_sessions_dir;
use std::fs;
use std::path::{Path, PathBuf};

/// Argument completer kinds supported by built-in command metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandArgumentCompleter {
    None,
    FilePath,
    SessionName,
}

/// Declarative metadata for one ex-command plus its aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) names: &'static [&'static str],
    pub(crate) argument_completer: CommandArgumentCompleter,
}

/// One completion candidate available in command mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionCandidate {
    pub(crate) insert_text: String,
    pub(crate) label: String,
}

/// One rendered command-completion entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionPopupEntry {
    pub(crate) label: String,
    pub(crate) selected: bool,
}

/// Render-facing command-completion popup model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionPopup {
    pub(crate) anchor_column: usize,
    pub(crate) entries: Vec<CommandCompletionPopupEntry>,
}

/// Cycling direction for command completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandCompletionDirection {
    Forward,
    Backward,
}

/// Active command-completion session plus live-preview state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionSession {
    replace_start_column: usize,
    replace_end_column: usize,
    anchor_column: usize,
    original_text: String,
    selected_index: Option<usize>,
    candidates: Vec<CommandCompletionCandidate>,
}

impl CommandCompletionSession {
    /// Build one active session from the matched candidates and replacement span.
    pub(crate) fn new(
        replace_start_column: usize,
        replace_end_column: usize,
        original_text: String,
        candidates: Vec<CommandCompletionCandidate>,
    ) -> Self {
        Self {
            replace_start_column,
            replace_end_column,
            anchor_column: replace_start_column,
            original_text,
            selected_index: None,
            candidates,
        }
    }

    /// Return the prompt column where replacement starts.
    pub(crate) fn replace_start_column(&self) -> usize {
        self.replace_start_column
    }

    /// Return the text that should currently appear in the prompt.
    pub(crate) fn current_text(&self) -> &str {
        self.selected_index
            .and_then(|index| self.candidates.get(index))
            .map(|candidate| candidate.insert_text.as_str())
            .unwrap_or(self.original_text.as_str())
    }

    /// Return the prompt column immediately after the visible preview text.
    pub(crate) fn replacement_end_column(&self) -> usize {
        self.replace_start_column + self.current_text().chars().count()
    }

    /// Move the active selection forward or backward through the candidate list.
    pub(crate) fn move_selection(&mut self, direction: CommandCompletionDirection) {
        if self.candidates.is_empty() {
            self.selected_index = None;
            return;
        }

        self.selected_index = match (direction, self.selected_index) {
            (CommandCompletionDirection::Forward, None) => Some(0),
            (CommandCompletionDirection::Forward, Some(index))
                if index + 1 < self.candidates.len() =>
            {
                Some(index + 1)
            }
            (CommandCompletionDirection::Forward, Some(_)) => None,
            (CommandCompletionDirection::Backward, None) => {
                Some(self.candidates.len().saturating_sub(1))
            }
            (CommandCompletionDirection::Backward, Some(index)) if index > 0 => Some(index - 1),
            (CommandCompletionDirection::Backward, Some(_)) => None,
        };
    }

    /// Build one render-facing popup model for this session.
    pub(crate) fn popup(&self) -> CommandCompletionPopup {
        let entries = self
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| CommandCompletionPopupEntry {
                label: candidate.label.clone(),
                selected: self.selected_index == Some(index),
            })
            .collect();
        CommandCompletionPopup {
            anchor_column: self.anchor_column,
            entries,
        }
    }
}

/// Build one command-completion session for the current prompt state.
pub(crate) fn build_command_completion_session(
    input: &str,
    cursor_column: usize,
    explicit: bool,
    command_specs: &[CommandSpec],
) -> Option<CommandCompletionSession> {
    let context = command_completion_context(input, cursor_column, explicit, command_specs)?;
    let candidates = collect_command_completion_candidates(&context, command_specs);
    if candidates.is_empty() {
        return None;
    }

    Some(CommandCompletionSession::new(
        context.replace_start_column,
        context.replace_end_column,
        context.original_text,
        candidates,
    ))
}

/// Internal completion target extracted from one command prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandCompletionContext {
    replace_start_column: usize,
    replace_end_column: usize,
    original_text: String,
    kind: CommandCompletionKind,
}

/// Command-mode token kinds that completion can target.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandCompletionKind {
    CommandName,
    FilePath,
    SessionName,
}

/// Resolve the active completion target from one command prompt.
fn command_completion_context(
    input: &str,
    cursor_column: usize,
    explicit: bool,
    command_specs: &[CommandSpec],
) -> Option<CommandCompletionContext> {
    let cursor_column = cursor_column.min(input.chars().count());
    let first_whitespace = input.chars().position(char::is_whitespace);

    if first_whitespace.is_none() || cursor_column <= first_whitespace.unwrap_or(0) {
        return command_name_completion_context(input, cursor_column, explicit);
    }

    // Command arguments only activate once the command token resolves exactly.
    let split_column = first_whitespace.expect("checked above");
    let command_name = slice_chars(input, 0, split_column);
    let completer = command_argument_completer(&command_name, command_specs)?;
    if completer == CommandArgumentCompleter::None {
        return None;
    }

    let argument_start = input
        .chars()
        .take(cursor_column.max(split_column))
        .enumerate()
        .skip(split_column)
        .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index))
        .unwrap_or(input.chars().count());
    if !explicit && argument_start == input.chars().count() {
        return None;
    }

    // The current command grammar only supports one free-form trailing argument,
    // so the replacement span can safely cover the whole remainder of the prompt.
    let original_text = slice_chars(input, argument_start, input.chars().count());
    if !explicit && original_text.is_empty() {
        return None;
    }

    Some(CommandCompletionContext {
        replace_start_column: argument_start,
        replace_end_column: input.chars().count(),
        original_text,
        kind: match completer {
            CommandArgumentCompleter::FilePath => CommandCompletionKind::FilePath,
            CommandArgumentCompleter::SessionName => CommandCompletionKind::SessionName,
            CommandArgumentCompleter::None => return None,
        },
    })
}

/// Build the command-name completion context when the cursor is in the first token.
fn command_name_completion_context(
    input: &str,
    _cursor_column: usize,
    explicit: bool,
) -> Option<CommandCompletionContext> {
    let replace_end_column = input
        .chars()
        .position(char::is_whitespace)
        .unwrap_or(input.chars().count());
    let original_text = slice_chars(input, 0, replace_end_column);
    if !original_text.is_empty() && original_text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if !explicit && original_text.is_empty() {
        return None;
    }

    Some(CommandCompletionContext {
        replace_start_column: 0,
        replace_end_column,
        original_text,
        kind: CommandCompletionKind::CommandName,
    })
}

/// Return the argument completer for one exactly matched command token.
fn command_argument_completer(
    command_name: &str,
    command_specs: &[CommandSpec],
) -> Option<CommandArgumentCompleter> {
    // Aliases share the same completer kind, so look through every declared name.
    command_specs.iter().find_map(|spec| {
        spec.names
            .contains(&command_name)
            .then_some(spec.argument_completer)
    })
}

/// Collect all completion candidates for one resolved context.
fn collect_command_completion_candidates(
    context: &CommandCompletionContext,
    command_specs: &[CommandSpec],
) -> Vec<CommandCompletionCandidate> {
    match context.kind {
        CommandCompletionKind::CommandName => {
            collect_command_name_candidates(&context.original_text, command_specs)
        }
        CommandCompletionKind::FilePath => collect_file_path_candidates(&context.original_text),
        CommandCompletionKind::SessionName => {
            collect_session_name_candidates(&context.original_text)
        }
    }
}

/// Collect command-name candidates that extend the current prefix.
fn collect_command_name_candidates(
    prefix: &str,
    command_specs: &[CommandSpec],
) -> Vec<CommandCompletionCandidate> {
    let normalized_prefix = normalize_text(prefix);
    let mut candidates = Vec::new();

    // Preserve the declared command order so common short aliases stay early.
    for spec in command_specs {
        for name in spec.names {
            if !normalize_text(name).starts_with(&normalized_prefix) {
                continue;
            }
            if name.chars().count() <= prefix.chars().count() {
                continue;
            }
            candidates.push(CommandCompletionCandidate {
                insert_text: (*name).to_string(),
                label: (*name).to_string(),
            });
        }
    }

    candidates
}

/// Collect file-path candidates rooted at the current working directory.
fn collect_file_path_candidates(prefix: &str) -> Vec<CommandCompletionCandidate> {
    let Some((resolved_directory, display_prefix, basename_prefix)) = path_completion_parts(prefix)
    else {
        return Vec::new();
    };
    let normalized_prefix = normalize_text(&basename_prefix);
    let mut candidates = Vec::new();
    let mut entries = read_sorted_directory_entries(&resolved_directory);

    // Hidden files stay hidden until the user starts the basename with `.`.
    entries.retain(|entry| basename_prefix.starts_with('.') || !entry.name.starts_with('.'));
    for entry in entries {
        if !normalize_text(&entry.name).starts_with(&normalized_prefix) {
            continue;
        }
        let insert_text = format!("{display_prefix}{}", entry.name);
        if insert_text.chars().count() <= prefix.chars().count() {
            continue;
        }
        candidates.push(CommandCompletionCandidate {
            insert_text,
            label: if entry.is_directory {
                format!("{}/", entry.name)
            } else {
                entry.name
            },
        });
    }

    candidates
}

/// Collect saved-session name candidates from the default session directory.
fn collect_session_name_candidates(prefix: &str) -> Vec<CommandCompletionCandidate> {
    let normalized_prefix = normalize_text(prefix);
    let Ok(sessions_dir) = default_sessions_dir() else {
        return Vec::new();
    };
    let mut names = Vec::new();

    // Session names come from on-disk TOML files, so strip the storage suffix.
    let Ok(read_dir) = fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let visible_name = name
            .strip_suffix(".toml")
            .unwrap_or(name.as_str())
            .to_string();
        if !normalize_text(&visible_name).starts_with(&normalized_prefix) {
            continue;
        }
        if visible_name.chars().count() <= prefix.chars().count() {
            continue;
        }
        names.push(visible_name);
    }
    names.sort_by_key(|name| normalize_text(name));
    names.dedup();
    names
        .into_iter()
        .map(|name| CommandCompletionCandidate {
            insert_text: name.clone(),
            label: name,
        })
        .collect()
}

/// One directory entry considered for command file-path completion.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryEntry {
    name: String,
    is_directory: bool,
}

/// Resolve the current directory, display prefix, and basename prefix for path completion.
fn path_completion_parts(prefix: &str) -> Option<(PathBuf, String, String)> {
    let current_directory = std::env::current_dir().ok()?;
    if let Some(separator_index) = prefix.rfind('/') {
        let directory_prefix = prefix[..=separator_index].to_string();
        let basename_prefix = prefix[separator_index + 1..].to_string();
        let resolved_directory = if Path::new(&directory_prefix).is_absolute() {
            PathBuf::from(&directory_prefix)
        } else {
            current_directory.join(&directory_prefix)
        };
        return Some((resolved_directory, directory_prefix, basename_prefix));
    }

    Some((current_directory, String::new(), prefix.to_string()))
}

/// Read and sort one directory so popup order stays stable across refreshes.
fn read_sorted_directory_entries(directory: &Path) -> Vec<DirectoryEntry> {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut entries = Vec::new();

    // Sort directories first, then case-insensitive names, so repeated typing
    // keeps the popup order stable even when filesystem iteration differs.
    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        entries.push(DirectoryEntry {
            name,
            is_directory: file_type.is_dir(),
        });
    }
    entries.sort_by_key(|entry| {
        (
            !entry.is_directory,
            normalize_text(&entry.name),
            entry.name.clone(),
        )
    });
    entries
}

/// Return one case-folded text value for case-insensitive prefix matching.
fn normalize_text(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Return the substring covering the half-open character range `[start, end)`.
fn slice_chars(text: &str, start: usize, end: usize) -> String {
    let start = start.min(text.chars().count());
    let end = end.min(text.chars().count()).max(start);
    text.chars().skip(start).take(end - start).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_state::ex_commands::command_specs;
    use test_utils::{CurrentDirectoryGuard, TempTree};

    /// Confirm automatic completion stays hidden for an empty command prompt.
    #[test]
    fn test_build_command_completion_session_skips_empty_prompt_without_explicit_trigger() {
        assert_eq!(
            build_command_completion_session("", 0, false, command_specs()),
            None
        );
    }

    /// Confirm explicit completion on an empty prompt lists built-in command names.
    #[test]
    fn test_build_command_completion_session_lists_commands_for_explicit_empty_prompt() {
        let session =
            build_command_completion_session("", 0, true, command_specs()).expect("session");

        assert!(
            session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "w")
        );
        assert!(
            session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "save-session")
        );
    }

    /// Confirm path completion resolves relative entries from the current directory.
    #[test]
    fn test_build_command_completion_session_lists_file_path_arguments() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/lib.rs", "pub fn demo() {}\n")
            .expect("write source");
        tree.write_file("state/file.txt", "demo\n")
            .expect("write directory");
        let _guard = CurrentDirectoryGuard::change_to(tree.path());

        let session =
            build_command_completion_session("e s", 3, false, command_specs()).expect("session");
        let labels = session
            .popup()
            .entries
            .into_iter()
            .map(|entry| entry.label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["src/".to_string(), "state/".to_string()]);
    }

    /// Confirm session completion strips the on-disk `.toml` suffix from suggestions.
    #[test]
    fn test_build_command_completion_session_lists_session_names_without_toml_suffix() {
        let sessions_dir = default_sessions_dir().expect("sessions dir");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::write(sessions_dir.join("alpha.toml"), "").expect("write session");
        fs::write(sessions_dir.join("beta.toml"), "").expect("write session");

        let session =
            build_command_completion_session("os a", 4, false, command_specs()).expect("session");
        let labels = session
            .popup()
            .entries
            .into_iter()
            .map(|entry| entry.label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["alpha".to_string()]);
    }

    /// Confirm cycling through candidates restores the original typed prefix at the end.
    #[test]
    fn test_command_completion_session_restores_original_text_after_last_candidate() {
        let mut session = CommandCompletionSession::new(
            0,
            2,
            "wr".to_string(),
            vec![CommandCompletionCandidate {
                insert_text: "write".to_string(),
                label: "write".to_string(),
            }],
        );

        session.move_selection(CommandCompletionDirection::Forward);
        assert_eq!(session.current_text(), "write");
        session.move_selection(CommandCompletionDirection::Forward);
        assert_eq!(session.current_text(), "wr");
    }
}
