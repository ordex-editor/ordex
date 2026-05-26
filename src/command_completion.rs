//! Command-mode completion modeling and candidate collection.

use crate::session::default_sessions_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

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
    /// Canonical command name shown and inserted by command completion.
    pub(crate) completion_name: &'static str,
    /// All accepted spellings for this command, including aliases.
    pub(crate) names: &'static [&'static str],
    pub(crate) argument_completer: CommandArgumentCompleter,
}

impl CommandSpec {
    /// Return whether `name` matches any accepted spelling of this command.
    ///
    /// Returns `true` when `name` is one of the canonical command spellings or
    /// aliases accepted by parsing, and `false` when it refers to another command.
    fn matches_name(&self, name: &str) -> bool {
        self.names.contains(&name)
    }
}

/// One completion candidate available in command mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionCandidate {
    /// Text inserted into the prompt when this candidate is previewed or selected.
    pub(crate) insert_text: String,
    /// Visible label rendered in the popup for this candidate.
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
    /// Prompt column where the active token starts.
    ///
    /// This is the first character column replaced when a previewed completion is
    /// written into the command prompt. For `:e st`, the file-path token starts
    /// after `e `, so this points at the `s` in `st`.
    replace_start_column: usize,
    /// Prompt column where the originally typed token ended when this session started.
    ///
    /// This marks the end of the user-typed replacement span before any preview
    /// text is applied. It stays fixed even when a preview grows longer so the
    /// session can restore the original token exactly when selection returns to
    /// `None`.
    replace_end_column: usize,
    /// Raw token text typed by the user before any completion preview was applied.
    original_text: String,
    /// Currently highlighted candidate index, or `None` when the raw token is shown.
    selected_index: Option<usize>,
    /// Candidate list available for the current command token.
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
        CommandCompletionPopup { entries }
    }
}

/// Stable request snapshot for one command-completion refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionRequest {
    input: String,
    cursor_column: usize,
    explicit: bool,
    context: CommandCompletionContext,
}

impl CommandCompletionRequest {
    /// Return whether this request still matches the supplied prompt state.
    ///
    /// Returns `true` when rebuilding command completion for the current prompt
    /// produces the same request snapshot, and `false` when typing, cursor
    /// movement, or explicit triggering changed the target token.
    pub(crate) fn matches_prompt_state(
        &self,
        input: &str,
        cursor_column: usize,
        command_specs: &[CommandSpec],
    ) -> bool {
        build_command_completion_request(input, cursor_column, self.explicit, command_specs)
            .is_some_and(|request| request == *self)
    }

    /// Return the context extracted from the request prompt.
    pub(crate) fn context(&self) -> &CommandCompletionContext {
        &self.context
    }

    /// Return whether this request should scan the filesystem on a background worker.
    ///
    /// Returns `true` when candidate collection may touch the filesystem for path
    /// or session discovery, and `false` when command-name completion can be
    /// resolved immediately from in-memory command metadata.
    pub(crate) fn requires_async_scan(&self) -> bool {
        self.context.requires_async_scan()
    }
}

/// Build one command-completion session for the current prompt state.
#[cfg(test)]
pub(crate) fn build_command_completion_session(
    input: &str,
    cursor_column: usize,
    explicit: bool,
    command_specs: &[CommandSpec],
) -> Option<CommandCompletionSession> {
    let request = build_command_completion_request(input, cursor_column, explicit, command_specs)?;
    if !request.requires_async_scan() {
        return build_command_completion_session_for_request(&request, command_specs);
    }

    let cancel = AtomicBool::new(false);
    // Tests sometimes need the fully collected session after bypassing the
    // background worker, so they collect the async-backed candidates inline here.
    let candidates = match request.context().kind {
        CommandCompletionKind::CommandName => {
            collect_command_name_candidates(&request.context().original_text, command_specs)
        }
        CommandCompletionKind::FilePath => {
            collect_file_path_candidates_with_cancel(&request.context().original_text, &cancel)
        }
        CommandCompletionKind::SessionName => {
            collect_session_name_candidates_with_cancel(&request.context().original_text, &cancel)
        }
    };
    build_command_completion_session_from_candidates(request.context(), candidates)
}

/// Build one command-completion session from a previously captured request.
pub(crate) fn build_command_completion_session_for_request(
    request: &CommandCompletionRequest,
    command_specs: &[CommandSpec],
) -> Option<CommandCompletionSession> {
    if request.requires_async_scan() {
        return None;
    }
    let candidates = collect_sync_command_completion_candidates(request.context(), command_specs);
    build_command_completion_session_from_candidates(request.context(), candidates)
}

/// Build one stable command-completion request for the current prompt state.
pub(crate) fn build_command_completion_request(
    input: &str,
    cursor_column: usize,
    explicit: bool,
    command_specs: &[CommandSpec],
) -> Option<CommandCompletionRequest> {
    let context = command_completion_context(input, cursor_column, command_specs)?;
    Some(CommandCompletionRequest {
        input: input.to_string(),
        cursor_column,
        explicit,
        context,
    })
}

/// Build one command-completion session from the resolved context and candidates.
pub(crate) fn build_command_completion_session_from_candidates(
    context: &CommandCompletionContext,
    candidates: Vec<CommandCompletionCandidate>,
) -> Option<CommandCompletionSession> {
    if candidates.is_empty() {
        return None;
    }

    Some(CommandCompletionSession::new(
        context.replace_start_column,
        context.replace_end_column,
        context.original_text.clone(),
        candidates,
    ))
}

/// Reuse still-matching async candidates while a refreshed request is running.
pub(crate) fn retained_async_command_completion_session(
    session: &CommandCompletionSession,
    request: &CommandCompletionRequest,
) -> Option<CommandCompletionSession> {
    if !request.requires_async_scan() {
        return None;
    }
    let context = request.context();
    if session.replace_start_column != context.replace_start_column {
        return None;
    }
    if context.kind == CommandCompletionKind::CommandName {
        return None;
    }

    let normalized_prefix = normalize_text(&context.original_text);
    // Re-filter the current popup against the new prefix so the popup stays
    // visible during background scans without showing entries from another token.
    let candidates = session
        .candidates
        .iter()
        .filter(|candidate| {
            normalized_prefix.is_empty()
                || normalize_text(&candidate.insert_text).starts_with(&normalized_prefix)
        })
        .cloned()
        .collect();
    build_command_completion_session_from_candidates(context, candidates)
}

/// Parsed description of the token that command completion should currently replace.
///
/// This represents the active prompt slice after command parsing decides whether
/// completion targets the command name or one supported trailing argument. It is
/// the bridge between raw prompt text and candidate collection because it stores
/// both the replacement bounds and the semantic completion kind for that token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionContext {
    replace_start_column: usize,
    replace_end_column: usize,
    original_text: String,
    kind: CommandCompletionKind,
}

impl CommandCompletionContext {
    /// Return whether collecting candidates for this context may block on filesystem work.
    ///
    /// Returns `true` for file-path and session-name completion contexts, and
    /// `false` for pure in-memory command-name completion.
    pub(crate) fn requires_async_scan(&self) -> bool {
        matches!(
            self.kind,
            CommandCompletionKind::FilePath | CommandCompletionKind::SessionName
        )
    }
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
    command_specs: &[CommandSpec],
) -> Option<CommandCompletionContext> {
    let cursor_column = cursor_column.min(input.chars().count());
    let first_whitespace = input.chars().position(char::is_whitespace);

    if first_whitespace.is_none() || cursor_column <= first_whitespace.unwrap_or(0) {
        return command_name_completion_context(input);
    }

    // Command arguments only activate once the command token resolves exactly.
    let Some(split_column) = first_whitespace else {
        return command_name_completion_context(input);
    };
    let command_name = slice_chars(input, 0, split_column);
    let completer = command_argument_completer(command_name, command_specs)?;
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

    // The current command grammar only supports one free-form trailing argument,
    // so the replacement span can safely cover the whole remainder of the prompt.
    let original_text = slice_chars(input, argument_start, input.chars().count()).to_string();

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
fn command_name_completion_context(input: &str) -> Option<CommandCompletionContext> {
    let replace_end_column = input
        .chars()
        .position(char::is_whitespace)
        .unwrap_or(input.chars().count());
    let original_text = slice_chars(input, 0, replace_end_column);
    // Purely numeric command prompts are line-jump commands, so command-name
    // completion must stay hidden and let the numeric prompt semantics win.
    if !original_text.is_empty() && original_text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(CommandCompletionContext {
        replace_start_column: 0,
        replace_end_column,
        original_text: original_text.to_string(),
        kind: CommandCompletionKind::CommandName,
    })
}

/// Return the argument completer for one exactly matched command token.
fn command_argument_completer(
    command_name: &str,
    command_specs: &[CommandSpec],
) -> Option<CommandArgumentCompleter> {
    // Aliases share the same completer kind, so look through every accepted name.
    command_specs.iter().find_map(|spec| {
        spec.matches_name(command_name)
            .then_some(spec.argument_completer)
    })
}

/// Collect sync-safe completion candidates for one resolved context.
fn collect_sync_command_completion_candidates(
    context: &CommandCompletionContext,
    command_specs: &[CommandSpec],
) -> Vec<CommandCompletionCandidate> {
    match context.kind {
        CommandCompletionKind::CommandName => {
            collect_command_name_candidates(&context.original_text, command_specs)
        }
        CommandCompletionKind::FilePath | CommandCompletionKind::SessionName => Vec::new(),
    }
}

/// Collect command-name candidates that extend the current prefix.
fn collect_command_name_candidates(
    prefix: &str,
    command_specs: &[CommandSpec],
) -> Vec<CommandCompletionCandidate> {
    let normalized_prefix = normalize_text(prefix);
    let mut candidates = Vec::new();

    // Preserve the declared command order so completion stays stable while still
    // showing the canonical command spelling instead of every accepted alias.
    for spec in command_specs {
        let name = spec.completion_name;
        if !normalize_text(name).starts_with(&normalized_prefix) {
            continue;
        }
        if name.len() <= prefix.len() {
            continue;
        }
        candidates.push(CommandCompletionCandidate {
            insert_text: name.to_string(),
            label: name.to_string(),
        });
    }

    candidates
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

/// Return one case-folded text value for case-insensitive prefix matching.
fn normalize_text(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Return the substring covering the half-open character range `[start, end)`.
fn slice_chars(text: &str, start: usize, end: usize) -> &str {
    let start = start.min(text.chars().count());
    let end = end.min(text.chars().count()).max(start);
    let start_byte = char_to_byte_idx(text, start);
    let end_byte = char_to_byte_idx(text, end);
    &text[start_byte..end_byte]
}

/// Convert one character index into its corresponding UTF-8 byte offset.
fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }

    text.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(text.len())
}

/// One in-flight asynchronous command-completion request.
#[derive(Debug)]
pub(crate) struct PendingCommandCompletion {
    request: CommandCompletionRequest,
    task: AsyncCommandCompletionTask,
}

/// One asynchronous command-completion worker hidden behind a request wrapper.
#[derive(Debug)]
enum AsyncCommandCompletionTask {
    FilePath(CommandFilePathCompletionScan),
    SessionName(CommandSessionCompletionScan),
}

impl PendingCommandCompletion {
    /// Spawn one background worker for `request` when the request scans the filesystem.
    pub(crate) fn spawn(request: CommandCompletionRequest) -> Option<Self> {
        let task = match request.context.kind {
            CommandCompletionKind::FilePath => AsyncCommandCompletionTask::FilePath(
                CommandFilePathCompletionScan::spawn(request.context.clone()),
            ),
            CommandCompletionKind::SessionName => AsyncCommandCompletionTask::SessionName(
                CommandSessionCompletionScan::spawn(request.context.clone()),
            ),
            CommandCompletionKind::CommandName => return None,
        };
        Some(Self { request, task })
    }

    /// Return the request owned by this background worker.
    pub(crate) fn request(&self) -> &CommandCompletionRequest {
        &self.request
    }

    /// Cancel this worker so later polls can ignore its results.
    pub(crate) fn cancel(&mut self) {
        match &mut self.task {
            AsyncCommandCompletionTask::FilePath(scan) => scan.cancel(),
            AsyncCommandCompletionTask::SessionName(scan) => scan.cancel(),
        }
    }

    /// Drain this worker and return any finished candidate set.
    pub(crate) fn poll(&mut self) -> CommandCompletionPollResult {
        match &mut self.task {
            AsyncCommandCompletionTask::FilePath(scan) => scan.poll(),
            AsyncCommandCompletionTask::SessionName(scan) => scan.poll(),
        }
    }
}

/// Final poll state for one asynchronous command-completion worker.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionPollResult {
    /// Whether the worker finished and no further polling is required.
    pub(crate) finished: bool,
    /// Completed candidates returned by the worker, when available.
    pub(crate) candidates: Option<Vec<CommandCompletionCandidate>>,
}

/// One background file-path scan for command completion plus its cancellation handle.
#[derive(Debug)]
struct CommandFilePathCompletionScan {
    receiver: Receiver<Vec<CommandCompletionCandidate>>,
    cancel: Arc<AtomicBool>,
}

impl CommandFilePathCompletionScan {
    /// Spawn one background file-path scan for `context`.
    fn spawn(context: CommandCompletionContext) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            let candidates =
                collect_file_path_candidates_with_cancel(&context.original_text, &worker_cancel);
            let _ = sender.send(candidates);
        });
        Self { receiver, cancel }
    }

    /// Cancel this file-path scan.
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Drain this file-path scan and return any finished candidates.
    fn poll(&mut self) -> CommandCompletionPollResult {
        match self.receiver.try_recv() {
            Ok(candidates) => CommandCompletionPollResult {
                finished: true,
                candidates: Some(candidates),
            },
            Err(TryRecvError::Empty) => CommandCompletionPollResult::default(),
            Err(TryRecvError::Disconnected) => CommandCompletionPollResult {
                finished: true,
                candidates: None,
            },
        }
    }
}

/// One background session-name scan plus its cancellation handle.
#[derive(Debug)]
struct CommandSessionCompletionScan {
    receiver: Receiver<Vec<CommandCompletionCandidate>>,
    cancel: Arc<AtomicBool>,
}

impl CommandSessionCompletionScan {
    /// Spawn one background session-name scan for `context`.
    fn spawn(context: CommandCompletionContext) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            let candidates =
                collect_session_name_candidates_with_cancel(&context.original_text, &worker_cancel);
            let _ = sender.send(candidates);
        });
        Self { receiver, cancel }
    }

    /// Cancel this session-name scan.
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Drain this session-name scan and return any finished candidates.
    fn poll(&mut self) -> CommandCompletionPollResult {
        match self.receiver.try_recv() {
            Ok(candidates) => CommandCompletionPollResult {
                finished: true,
                candidates: Some(candidates),
            },
            Err(TryRecvError::Empty) => CommandCompletionPollResult::default(),
            Err(TryRecvError::Disconnected) => CommandCompletionPollResult {
                finished: true,
                candidates: None,
            },
        }
    }
}

/// Collect file-path candidates while allowing a background worker to cancel early.
fn collect_file_path_candidates_with_cancel(
    prefix: &str,
    cancel: &AtomicBool,
) -> Vec<CommandCompletionCandidate> {
    let Some((resolved_directory, display_prefix, basename_prefix)) = path_completion_parts(prefix)
    else {
        return Vec::new();
    };
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let normalized_prefix = normalize_text(&basename_prefix);
    let mut entries = read_sorted_directory_entries_with_cancel(&resolved_directory, cancel);
    let mut candidates = Vec::new();

    // Hidden files stay hidden until the user starts the basename with `.`.
    entries.retain(|entry| basename_prefix.starts_with('.') || !entry.name.starts_with('.'));
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        if !normalize_text(&entry.name).starts_with(&normalized_prefix) {
            continue;
        }
        let insert_text = format!("{display_prefix}{}", entry.name);
        if insert_text.len() <= prefix.len() {
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

/// Collect session-name candidates while allowing a background worker to cancel early.
fn collect_session_name_candidates_with_cancel(
    prefix: &str,
    cancel: &AtomicBool,
) -> Vec<CommandCompletionCandidate> {
    let normalized_prefix = normalize_text(prefix);
    let Ok(sessions_dir) = default_sessions_dir() else {
        return Vec::new();
    };
    let Ok(read_dir) = fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();

    for entry in read_dir.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Session files use a `.toml` storage suffix even though completion
        // should present only the user-visible session name.
        let visible_name = name
            .strip_suffix(".toml")
            .unwrap_or(name.as_str())
            .to_string();
        if !normalize_text(&visible_name).starts_with(&normalized_prefix) {
            continue;
        }
        if visible_name.len() <= prefix.len() {
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

/// Read and sort one directory while allowing a background worker to cancel early.
fn read_sorted_directory_entries_with_cancel(
    directory: &Path,
    cancel: &AtomicBool,
) -> Vec<DirectoryEntry> {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut entries = Vec::new();

    for entry in read_dir.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
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

    // Sorting first by directory-ness and then by normalized name keeps the
    // popup order stable while still listing directories ahead of files.
    entries.sort_by_key(|entry| {
        (
            !entry.is_directory,
            normalize_text(&entry.name),
            entry.name.clone(),
        )
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_state::ex_commands::command_specs;
    use std::sync::atomic::AtomicBool;
    use test_utils::TempTree;

    /// Confirm automatic completion on `:` lists built-in command names.
    #[test]
    fn test_build_command_completion_session_lists_commands_for_empty_prompt() {
        let session =
            build_command_completion_session("", 0, false, command_specs()).expect("session");

        assert!(
            session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "write")
        );
        assert!(
            session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "quit")
        );
        assert!(
            !session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "w")
        );
        assert!(
            !session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "q")
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
                .any(|entry| entry.label == "write")
        );
        assert!(
            session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "save-session")
        );
        assert!(
            !session
                .popup()
                .entries
                .iter()
                .any(|entry| entry.label == "e")
        );
    }

    /// Confirm file-path completion lists matching directories with trailing `/`.
    #[test]
    fn test_collect_file_path_candidates_with_cancel_lists_matching_directories() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/lib.rs", "pub fn demo() {}\n")
            .expect("write source");
        tree.write_file("state/file.txt", "demo\n")
            .expect("write directory");
        let cancel = AtomicBool::new(false);
        let prefix = format!("{}/s", tree.path().display());

        let labels = collect_file_path_candidates_with_cancel(&prefix, &cancel)
            .into_iter()
            .map(|candidate| candidate.label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["src/".to_string(), "state/".to_string()]);
    }

    /// Confirm supported path completions open immediately after one separating space.
    #[test]
    fn test_collect_file_path_candidates_with_cancel_lists_entries_for_empty_prefix() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("state/file.txt", "demo\n")
            .expect("write directory");
        let cancel = AtomicBool::new(false);
        let prefix = format!("{}/", tree.path().display());

        let labels = collect_file_path_candidates_with_cancel(&prefix, &cancel)
            .into_iter()
            .map(|candidate| candidate.label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["state/".to_string()]);
    }

    /// Confirm session completion strips the on-disk `.toml` suffix from suggestions.
    #[test]
    fn test_collect_session_name_candidates_with_cancel_strips_toml_suffix() {
        let sessions_dir = default_sessions_dir().expect("sessions dir");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::write(sessions_dir.join("alpha.toml"), "").expect("write session");
        fs::write(sessions_dir.join("beta.toml"), "").expect("write session");
        let cancel = AtomicBool::new(false);

        let labels = collect_session_name_candidates_with_cancel("a", &cancel)
            .into_iter()
            .map(|candidate| candidate.label)
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["alpha".to_string()]);
    }

    /// Confirm supported session completions open immediately after one separating space.
    #[test]
    fn test_collect_session_name_candidates_with_cancel_lists_entries_for_empty_prefix() {
        let sessions_dir = default_sessions_dir().expect("sessions dir");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::write(sessions_dir.join("alpha.toml"), "").expect("write session");
        let cancel = AtomicBool::new(false);

        let labels = collect_session_name_candidates_with_cancel("", &cancel)
            .into_iter()
            .map(|candidate| candidate.label)
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
