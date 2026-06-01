//! Application startup and runtime orchestration.

use crate::clipboard::{ClipboardPasteRequest, ClipboardState, ClipboardWriteRequest};
use crate::config;
use crate::editor_state::{DeferredWrite, EditorRequest, EditorState};
use crate::lsp::LspManager;
use crate::render::{
    RenderDecision, RenderSnapshot, TerminalSize, render_editor, render_message_line,
    render_status_cursor, render_vertical_cursor_motion, resize_editor,
};
use crate::session;
use crate::signal::SignalGuard;
use crate::temp_paths;
use crate::themes;
use crate::tui;
use std::env;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::time::{Duration, Instant};
use termion::event::Key;

#[derive(Debug, Default)]
struct CliArgs {
    file_paths: Vec<String>,
    config_path: Option<String>,
}

/// Shared process-owned state borrowed by the interactive event loop.
struct EventLoopContext<'a> {
    lsp_manager: &'a mut LspManager,
    clipboard: &'a mut ClipboardState,
    config_path: Option<&'a str>,
    loaded_session_name: &'a mut Option<String>,
    key_log: &'a mut Option<File>,
}

/// Result produced when the interactive event loop terminates.
#[derive(Debug, Default)]
struct EventLoopOutcome {
    exit_code: i32,
    shutdown_warning: Option<String>,
}

/// Result of quit-time session autosave evaluation.
#[derive(Debug, Eq, PartialEq)]
enum QuitAutosaveOutcome {
    NoSession,
    Saved,
    SkippedMissingWorkingDirectory { session_name: String },
}

/// Final quit decision returned to the event loop.
#[derive(Debug, Eq, PartialEq)]
struct QuitFinalization {
    should_exit: bool,
    shutdown_warning: Option<String>,
}

impl QuitFinalization {
    /// Return one finalization that keeps the editor running after a quit error.
    fn stay_in_editor() -> Self {
        Self {
            should_exit: false,
            shutdown_warning: None,
        }
    }

    /// Return one finalization that allows exit and optionally emits one shell warning.
    fn exit(shutdown_warning: Option<String>) -> Self {
        Self {
            should_exit: true,
            shutdown_warning,
        }
    }
}

/// Launch the application and translate runtime results into process exit behavior.
pub(crate) fn launch() {
    match run() {
        Ok(outcome) => {
            let EventLoopOutcome {
                exit_code,
                shutdown_warning,
            } = outcome;
            if let Some(warning) = shutdown_warning {
                eprintln!("{warning}");
            }
            if exit_code != 0 {
                process::exit(exit_code);
            }
        }
        Err(error) => {
            eprintln!("Error: {error}");
            process::exit(1);
        }
    }
}

/// Execute startup, terminal setup, and the interactive editor runtime.
fn run() -> io::Result<EventLoopOutcome> {
    let args: Vec<String> = env::args().collect();
    let cli_args = parse_cli_args(&args[1..])?;
    let config_outcome = load_startup_config(cli_args.config_path.as_deref())?;

    // Startup warnings must stay on the shell screen before raw mode takes over.
    let outcome = {
        let mut term = tui::Terminal::new()?;
        term.clear_screen()?;

        let terminal_size = TerminalSize::from_termion(termion::terminal_size()?);
        let signals = SignalGuard::install()?;
        let mut editor =
            initialize_editor(&cli_args, config_outcome.as_ref(), terminal_size.height)?;
        let mut lsp_manager = LspManager::new();
        let mut clipboard = ClipboardState::new();
        dispatch_due_lsp_sync(&mut editor, &mut lsp_manager, Instant::now());
        dispatch_due_lsp_completion(&mut editor, &mut lsp_manager);
        dispatch_due_lsp_signature_help(&mut editor, &mut lsp_manager);
        let mut key_log = init_key_log()?;
        let mut loaded_session_name = None;
        let mut event_loop_context = EventLoopContext {
            lsp_manager: &mut lsp_manager,
            clipboard: &mut clipboard,
            config_path: cli_args.config_path.as_deref(),
            loaded_session_name: &mut loaded_session_name,
            key_log: &mut key_log,
        };

        run_event_loop(
            &mut term,
            &signals,
            &mut editor,
            &mut event_loop_context,
            terminal_size,
        )?
    };

    Ok(outcome)
}

/// Load startup configuration and emit any shell-facing warnings before TUI startup.
fn load_startup_config(config_path: Option<&str>) -> io::Result<Option<config::ConfigLoadOutcome>> {
    let outcome = config_path.map(|path| config::load_config(Path::new(path)));

    if let Some(outcome) = &outcome {
        // Config diagnostics belong on stderr before the alternate screen opens.
        config::emit_startup_warnings(&outcome.report.warnings);
        if should_emit_config_summary(outcome) {
            emit_config_summary(outcome);
        }
        if !outcome.report.warnings.is_empty() && should_pause_for_warnings() {
            wait_for_warning_ack()?;
        }
    }

    Ok(outcome)
}

/// Build the initial editor state from CLI arguments, config, and terminal size.
fn initialize_editor(
    cli_args: &CliArgs,
    config_outcome: Option<&config::ConfigLoadOutcome>,
    terminal_height: u16,
) -> io::Result<EditorState> {
    let mut editor = EditorState::new(terminal_height as usize);
    editor.set_color_capability(detect_color_capability());

    // Apply config before loading the file so the first syntax/render pass uses it.
    if let Some(outcome) = config_outcome {
        editor.replace_config(&outcome.settings);
    }
    open_startup_files(&mut editor, &cli_args.file_paths)?;
    if cli_args.file_paths.is_empty() {
        editor.load_startup_swap_state();
    }

    Ok(editor)
}

/// Open every requested startup file or prepare named buffers for missing paths.
fn open_startup_files(editor: &mut EditorState, file_paths: &[String]) -> io::Result<()> {
    let Some((first_path, extra_paths)) = file_paths.split_first() else {
        return Ok(());
    };

    if Path::new(first_path).exists() {
        editor.load_file(first_path)?;
    } else {
        // New buffers inherit the requested file name so syntax detection still works.
        editor.set_startup_path(first_path);
    }
    let first_buffer_id = editor.active_buffer_id();

    for path in extra_paths {
        editor.open_startup_buffer(path)?;
    }
    editor.activate_buffer(first_buffer_id);

    Ok(())
}

/// Drive rendering, input handling, and deferred process-level actions.
fn run_event_loop(
    term: &mut tui::Terminal,
    signals: &SignalGuard,
    editor: &mut EditorState,
    context: &mut EventLoopContext<'_>,
    mut terminal_size: TerminalSize,
) -> io::Result<EventLoopOutcome> {
    const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_millis(50);
    let mut needs_render = true;
    let mut needs_message_render = false;
    let mut needs_cursor_render = false;
    let mut needs_vertical_cursor_render = None;
    let mut shutdown_warning = None;
    // The discovery popup can temporarily hide the terminal cursor when it lands
    // on top of the logical cursor cell. Track that across redraws so we only
    // emit `Show`/`Hide` when the visibility state actually changes.
    let mut cursor_hidden_by_overlay = false;
    signals.mark_resize_pending();
    // Kick off any startup LSP sync immediately so launch-time indexing can surface
    // progress before the user explicitly asks for go-to-definition.
    dispatch_due_lsp_sync(editor, context.lsp_manager, Instant::now());
    dispatch_due_lsp_completion(editor, context.lsp_manager);
    dispatch_due_lsp_signature_help(editor, context.lsp_manager);

    // The loop always reacts to pending signals before waiting on the next key.
    loop {
        // Honor termination before any redraw so the shell regains a restored
        // terminal instead of one more TUI frame.
        if signals.take_termination_signal().is_some() {
            break;
        }

        // Refresh terminal dimensions only when SIGWINCH arrives.
        if signals.take_resize_pending() {
            let current_size = TerminalSize::from_termion(termion::terminal_size()?);
            if current_size != terminal_size {
                terminal_size = current_size;
                resize_editor(editor, terminal_size);
                needs_render = true;
            }
        }

        if needs_render {
            // Full redraws also reset the smaller targeted redraw flags.
            render_editor(term, editor, terminal_size, &mut cursor_hidden_by_overlay)?;
            editor.finish_full_render();
            needs_render = false;
            needs_message_render = false;
            needs_cursor_render = false;
            needs_vertical_cursor_render = None;
        } else if let Some(previous_cursor_line) = needs_vertical_cursor_render.take() {
            render_vertical_cursor_motion(
                term,
                editor,
                terminal_size,
                previous_cursor_line,
                &mut cursor_hidden_by_overlay,
            )?;
            needs_cursor_render = false;
        } else if needs_cursor_render {
            render_status_cursor(term, editor, terminal_size, &mut cursor_hidden_by_overlay)?;
            needs_cursor_render = false;
        } else if needs_message_render {
            render_message_line(term, editor, terminal_size, &mut cursor_hidden_by_overlay)?;
            editor.finish_message_render();
            needs_message_render = false;
        }

        // Poll while asynchronous picker work or a live language-server session
        // may still update visible UI state without user input.
        let next_input =
            if editor.needs_background_poll() || context.lsp_manager.should_background_poll() {
                tui::Terminal::read_input_event_timeout(BACKGROUND_POLL_INTERVAL)
            } else {
                tui::Terminal::read_input_event().map(Some)
            };
        match next_input {
            Ok(Some(input)) => {
                let mode_before = editor.mode_name();
                // Compare visible state before and after the key to pick the smallest redraw.
                let before = RenderSnapshot::capture(editor);
                match input {
                    tui::InputEvent::Key(key) => {
                        editor.handle_key(key);
                        log_key_event(context.key_log, key, mode_before, editor);
                    }
                    tui::InputEvent::Paste(text) => editor.handle_paste(&text),
                }
                handle_editor_request(
                    editor,
                    context.lsp_manager,
                    context.clipboard,
                    context.config_path,
                    context.loaded_session_name,
                );
                refresh_lsp_completion_trigger(editor, context.lsp_manager);
                refresh_lsp_signature_help_trigger(editor, context.lsp_manager);
                dispatch_due_lsp_sync(editor, context.lsp_manager, Instant::now());
                dispatch_due_lsp_completion(editor, context.lsp_manager);
                dispatch_due_lsp_signature_help(editor, context.lsp_manager);
                context.lsp_manager.poll(editor);
                if editor.should_quit() {
                    let quit_finalization =
                        finalize_pending_quit(editor, context.loaded_session_name);
                    if quit_finalization.should_exit {
                        shutdown_warning = quit_finalization.shutdown_warning;
                        break;
                    }
                }
                let after = RenderSnapshot::capture(editor);
                apply_render_decision(
                    RenderSnapshot::decide(&before, &after),
                    before.cursor_line(),
                    &mut needs_render,
                    &mut needs_message_render,
                    &mut needs_cursor_render,
                    &mut needs_vertical_cursor_render,
                );
            }
            Ok(None) => {
                let before = RenderSnapshot::capture(editor);
                handle_editor_request(
                    editor,
                    context.lsp_manager,
                    context.clipboard,
                    context.config_path,
                    context.loaded_session_name,
                );
                // A timeout can fire before the worker sends a new batch or after
                // the picker has already been closed, so skip redraw work unless
                // polling actually changed visible state.
                refresh_lsp_completion_trigger(editor, context.lsp_manager);
                refresh_lsp_signature_help_trigger(editor, context.lsp_manager);
                dispatch_due_lsp_sync(editor, context.lsp_manager, Instant::now());
                dispatch_due_lsp_completion(editor, context.lsp_manager);
                dispatch_due_lsp_signature_help(editor, context.lsp_manager);
                editor.poll_background_tasks();
                context.lsp_manager.poll(editor);
                // Deferred write completions can request quit from the timeout
                // path, so this branch must honor quit state the same way the
                // key-input branch already does.
                if editor.should_quit() {
                    let quit_finalization =
                        finalize_pending_quit(editor, context.loaded_session_name);
                    if quit_finalization.should_exit {
                        shutdown_warning = quit_finalization.shutdown_warning;
                        break;
                    }
                }
                let after = RenderSnapshot::capture(editor);
                apply_render_decision(
                    RenderSnapshot::decide(&before, &after),
                    before.cursor_line(),
                    &mut needs_render,
                    &mut needs_message_render,
                    &mut needs_cursor_render,
                    &mut needs_vertical_cursor_render,
                );
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                // Signals interrupt the blocking read; the next loop iteration
                // decides whether that means resize handling or termination.
            }
            Err(error) => return Err(error),
        }
    }

    Ok(EventLoopOutcome {
        exit_code: editor.quit_exit_code(),
        shutdown_warning,
    })
}

/// Apply one render decision to the queued redraw flags for the next loop turn.
fn apply_render_decision(
    decision: RenderDecision,
    previous_cursor_line: usize,
    needs_render: &mut bool,
    needs_message_render: &mut bool,
    needs_cursor_render: &mut bool,
    needs_vertical_cursor_render: &mut Option<usize>,
) {
    match decision {
        RenderDecision::Full => {
            *needs_render = true;
            *needs_message_render = false;
            *needs_cursor_render = false;
            *needs_vertical_cursor_render = None;
        }
        RenderDecision::VerticalCursor => {
            if !*needs_render {
                *needs_vertical_cursor_render = Some(previous_cursor_line);
                *needs_message_render = false;
                *needs_cursor_render = false;
            }
        }
        RenderDecision::CursorOnly => {
            if !*needs_render && needs_vertical_cursor_render.is_none() {
                *needs_cursor_render = true;
            }
        }
        RenderDecision::MessageOnly => {
            if !*needs_render && needs_vertical_cursor_render.is_none() && !*needs_cursor_render {
                *needs_message_render = true;
            }
        }
        RenderDecision::None => {}
    }
}

/// Dispatch one due proactive LSP sync without blocking the editor event loop.
fn dispatch_due_lsp_sync(editor: &mut EditorState, lsp_manager: &mut LspManager, now: Instant) {
    let Some(snapshot) = editor.take_due_document_sync_snapshot(now) else {
        return;
    };
    // Proactive sync intentionally stays best-effort so missing language-server
    // tooling does not interrupt ordinary editing before the user asks for `gd`.
    lsp_manager.request_document_sync(snapshot);
}

/// Dispatch one due automatic LSP completion lookup without blocking typing.
fn dispatch_due_lsp_completion(editor: &mut EditorState, lsp_manager: &mut LspManager) {
    let Some(snapshot) = editor.take_due_completion_request_snapshot() else {
        return;
    };
    lsp_manager.request_completion(snapshot);
}

/// Dispatch one due automatic LSP signature-help lookup without blocking typing.
fn dispatch_due_lsp_signature_help(editor: &mut EditorState, lsp_manager: &mut LspManager) {
    let Some(snapshot) = editor.take_due_signature_help_request_snapshot() else {
        return;
    };
    lsp_manager.request_signature_help(snapshot);
}

/// Promote or queue one trigger-character completion using the server's advertised policy.
fn refresh_lsp_completion_trigger(editor: &mut EditorState, lsp_manager: &mut LspManager) {
    let max_trigger_chars = editor
        .pending_lsp_trigger_file_path()
        .or_else(|| editor.lsp_trigger_candidate_file_path())
        .map(|file_path| lsp_manager.max_completion_trigger_chars(&file_path))
        .unwrap_or(0);
    if max_trigger_chars == 0 {
        return;
    }
    if let Some((file_path, recent_text)) = editor.pending_lsp_trigger_context(max_trigger_chars) {
        // Trigger metadata is cached after session initialize, so promotion only
        // consults the already-known provider state instead of re-querying the server.
        if let Some(trigger_text) =
            lsp_manager.matching_completion_trigger(&file_path, &recent_text)
        {
            editor.set_pending_lsp_trigger_text(&trigger_text);
            editor.promote_pending_lsp_completion();
        }
        return;
    }
    let Some((file_path, recent_text)) = editor.lsp_trigger_candidate_context(max_trigger_chars)
    else {
        return;
    };
    let Some(trigger_text) = lsp_manager.matching_completion_trigger(&file_path, &recent_text)
    else {
        return;
    };
    editor.queue_lsp_trigger_completion(&trigger_text);
    editor.promote_pending_lsp_completion();
}

/// Promote or queue one trigger-character signature-help lookup using the advertised policy.
fn refresh_lsp_signature_help_trigger(editor: &mut EditorState, lsp_manager: &mut LspManager) {
    let max_trigger_chars = editor
        .pending_signature_help_trigger_file_path()
        .or_else(|| editor.signature_help_trigger_candidate_file_path())
        .map(|file_path| lsp_manager.max_signature_help_trigger_chars(&file_path))
        .unwrap_or(0);
    if max_trigger_chars == 0 {
        return;
    }
    if let Some((file_path, recent_text)) =
        editor.pending_signature_help_trigger_context(max_trigger_chars)
    {
        if let Some(trigger_text) =
            lsp_manager.matching_signature_help_trigger(&file_path, &recent_text)
        {
            editor.set_pending_signature_help_trigger_text(&trigger_text);
            editor.promote_pending_signature_help();
        }
        return;
    }
    let Some((file_path, recent_text)) =
        editor.signature_help_trigger_candidate_context(max_trigger_chars)
    else {
        return;
    };
    let Some(trigger_text) = lsp_manager.matching_signature_help_trigger(&file_path, &recent_text)
    else {
        return;
    };
    editor.queue_lsp_trigger_signature_help(&trigger_text);
    editor.promote_pending_signature_help();
}

/// Run deferred editor requests that need process-level state from the app layer.
///
/// The editor parses commands while handling keys, but it deliberately does not
/// own CLI arguments or perform config-file or buffer-write I/O directly.
/// `pending_request` bridges that boundary: `EditorState` records the next
/// process-level action, and the application loop executes it once control
/// returns to the layer that owns the active config path and filesystem access.
fn handle_editor_request(
    editor: &mut EditorState,
    lsp_manager: &mut LspManager,
    clipboard: &mut ClipboardState,
    config_path: Option<&str>,
    loaded_session_name: &mut Option<String>,
) {
    let Some(request) = editor.take_pending_request() else {
        return;
    };

    // Execute at most one request per loop turn so chained follow-up work yields
    // back to the renderer and input polling before the next request runs.
    match request {
        EditorRequest::ReloadConfig => reload_editor_config(editor, config_path),
        EditorRequest::WriteBuffer(write) => execute_deferred_write(editor, lsp_manager, write),
        EditorRequest::WriteClipboard(write) => {
            execute_deferred_clipboard_write(editor, clipboard, &write)
        }
        EditorRequest::PasteClipboard(paste) => {
            execute_deferred_clipboard_paste(editor, clipboard, &paste)
        }
        EditorRequest::SaveSession(name) => {
            execute_deferred_session_save(editor, &name, loaded_session_name)
        }
        EditorRequest::OpenSession(name) => {
            execute_deferred_session_open(editor, &name, loaded_session_name)
        }
        EditorRequest::DeleteSession(name) => {
            execute_deferred_session_delete(editor, &name, loaded_session_name)
        }
        EditorRequest::LspNavigation(kind) => {
            if let Some(snapshot) = editor.navigation_request_snapshot() {
                match kind {
                    crate::lsp::NavigationKind::Definition => {
                        lsp_manager.request_definition(snapshot)
                    }
                    crate::lsp::NavigationKind::References => {
                        lsp_manager.request_references(snapshot)
                    }
                }
            }
        }
        EditorRequest::LspHover => {
            if let Some(snapshot) = editor.hover_request_snapshot() {
                lsp_manager.request_hover(snapshot);
            }
        }
        EditorRequest::LspRename(new_name) => {
            if let Some(snapshot) = editor.rename_request_snapshot(&new_name) {
                lsp_manager.request_rename(snapshot);
            }
        }
        EditorRequest::LspCodeAction => {
            if let Some(snapshot) = editor.code_action_request_snapshot() {
                lsp_manager.request_code_actions(snapshot);
            }
        }
    }
}

/// Finalize one pending quit request and report whether shutdown may proceed.
fn finalize_pending_quit(
    editor: &mut EditorState,
    loaded_session_name: &Option<String>,
) -> QuitFinalization {
    finalize_pending_quit_from_autosave_result(
        editor,
        autosave_loaded_session_on_quit(editor, loaded_session_name.as_deref()),
    )
}

/// Finalize one pending quit request against either the default or one explicit sessions directory.
#[cfg(test)]
fn finalize_pending_quit_in_directory(
    editor: &mut EditorState,
    loaded_session_name: &Option<String>,
    working_directory: PathBuf,
    sessions_dir: Option<&Path>,
) -> QuitFinalization {
    let autosave_result = match loaded_session_name.as_deref() {
        Some(name) => autosave_loaded_session_with_resolved_working_directory(
            editor,
            name,
            working_directory,
            sessions_dir,
        ),
        None => Ok(QuitAutosaveOutcome::NoSession),
    };
    finalize_pending_quit_from_autosave_result(editor, autosave_result)
}

/// Convert one quit-autosave result into a final quit decision.
fn finalize_pending_quit_from_autosave_result(
    editor: &mut EditorState,
    autosave_result: io::Result<QuitAutosaveOutcome>,
) -> QuitFinalization {
    let mut shutdown_warning = None;
    match autosave_result {
        Ok(QuitAutosaveOutcome::NoSession | QuitAutosaveOutcome::Saved) => {}
        Ok(QuitAutosaveOutcome::SkippedMissingWorkingDirectory { session_name }) => {
            shutdown_warning = Some(quit_autosave_skipped_warning(&session_name));
        }
        Err(error) => {
            editor.cancel_quit();
            editor.show_status_message(error.to_string());
            return QuitFinalization::stay_in_editor();
        }
    }
    editor.cleanup_all_swap_files();
    QuitFinalization::exit(shutdown_warning)
}

/// Reload configuration from the active config path and apply it immediately.
fn reload_editor_config(editor: &mut EditorState, config_path: Option<&str>) {
    let Some(config_path) = config_path else {
        editor.show_status_message("No config file to reload");
        return;
    };

    let outcome = config::load_config(Path::new(config_path));
    editor.replace_config(&outcome.settings);
    editor.show_status_message(reload_status_message(&outcome));
}

/// Execute one deferred clipboard write request against the active session backend.
pub(crate) fn execute_deferred_clipboard_write(
    editor: &mut EditorState,
    clipboard: &mut ClipboardState,
    request: &ClipboardWriteRequest,
) {
    match clipboard.write(request) {
        Ok(()) => {}
        Err(error) => editor.show_status_message(error.to_string()),
    }
}

/// Execute one deferred clipboard paste request against the active session backend.
pub(crate) fn execute_deferred_clipboard_paste(
    editor: &mut EditorState,
    clipboard: &mut ClipboardState,
    request: &ClipboardPasteRequest,
) {
    match clipboard.read(request.register) {
        Ok(payload) => editor.apply_clipboard_paste(payload, request.position, request.count),
        Err(error) => editor.show_status_message(error.to_string()),
    }
}

/// Execute one deferred buffer-write request against the filesystem.
pub(crate) fn execute_deferred_write(
    editor: &mut EditorState,
    lsp_manager: &mut LspManager,
    write: DeferredWrite,
) {
    let save_snapshot = editor.document_save_snapshot(&write.path, write.update_file_path);
    match write_buffer_atomically(editor, &write.path) {
        Ok(()) => {
            // Notify the language server only after the filesystem write
            // succeeded so `didSave` always reflects on-disk reality.
            if let Some(snapshot) = save_snapshot {
                lsp_manager.request_document_save(snapshot);
            }
            editor.complete_deferred_write(write.clone());
            if let Some(warning) = editor.finalize_swap_after_successful_write(&write) {
                editor.show_status_message(warning);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            editor.flush_pending_swap_refresh();
            editor.report_file_create_error(error);
        }
        Err(error) => {
            editor.flush_pending_swap_refresh();
            editor.report_file_write_error(error);
        }
    }
}

/// Write the active buffer through a temp file, `sync_all`, and atomic rename.
///
/// The temp file is created beside the final target so the rename stays on the
/// same filesystem. That keeps the on-disk file either fully old or fully new,
/// which prevents interrupted writes from leaving a truncated document behind.
fn write_buffer_atomically(editor: &EditorState, target_path: &Path) -> io::Result<()> {
    let temp_path = temp_write_path(target_path)?;
    let write_result = (|| {
        // `create_new(true)` refuses to reuse any pre-existing sibling path, so a
        // stale temp name from another process cannot be truncated and mistaken
        // for the fresh write that this save operation is about to produce.
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        // Stream the in-memory buffer into the sibling temp file first. The final
        // destination is only touched by the last rename, so readers never see a
        // partially-written target file if the process exits mid-write.
        editor.write_buffer_to(&mut file)?;
        // `sync_all` forces both file data and metadata out before the rename, so
        // the durable-save path does not report success for bytes still sitting
        // only in the kernel page cache.
        file.sync_all()?;
        // The rename is the visibility switch: after it succeeds, the target path
        // refers to the fully-written temp file in one atomic directory update.
        fs::rename(&temp_path, target_path)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

/// Build one temp save path beside the final target file.
fn temp_write_path(target_path: &Path) -> io::Result<PathBuf> {
    temp_paths::unique_sibling_temp_path(target_path, "ordex")
}

/// Save the current editor state as one named project session.
fn execute_deferred_session_save(
    editor: &mut EditorState,
    name: &str,
    loaded_session_name: &mut Option<String>,
) {
    match save_current_project_session_and_track(editor, name, loaded_session_name) {
        Ok(_) => {
            editor.show_status_message(format!("Session \"{name}\" saved"));
        }
        Err(error) => {
            editor.show_status_message(session_save_error_message(name, &error));
        }
    }
}

/// Load one named project session and restore it into the running editor.
fn execute_deferred_session_open(
    editor: &mut EditorState,
    name: &str,
    loaded_session_name: &mut Option<String>,
) {
    let previous_directory = env::current_dir().ok();
    let outcome = match session::load_project_session(name) {
        Ok(outcome) => outcome,
        Err(error) => {
            editor.show_status_message(format!("Error opening session \"{name}\": {error}"));
            return;
        }
    };

    if let Err(error) = env::set_current_dir(&outcome.session.working_directory) {
        editor.show_status_message(format!(
            "Error opening session \"{name}\": failed to restore working directory ({error})"
        ));
        return;
    }

    // Restore the editor only after the process working directory is in place so
    // relative session paths reopen against the saved project root.
    if let Err(error) = editor.restore_project_session(&outcome.session) {
        if let Some(previous_directory) = previous_directory {
            let _ = env::set_current_dir(previous_directory);
        }
        editor.show_status_message(format!("Error opening session \"{name}\": {error}"));
        return;
    }

    *loaded_session_name = Some(name.to_string());
    editor.show_status_message(session::load_status_message(name, outcome.warnings.len()));
}

/// Delete one named project session from disk.
fn execute_deferred_session_delete(
    editor: &mut EditorState,
    name: &str,
    loaded_session_name: &mut Option<String>,
) {
    match session::delete_project_session(name) {
        Ok(()) => {
            if loaded_session_name.as_deref() == Some(name) {
                *loaded_session_name = None;
            }
            editor.show_status_message(format!("Session \"{name}\" deleted"));
        }
        Err(error) => {
            editor.show_status_message(format!("Error deleting session \"{name}\": {error}"));
        }
    }
}

/// Save one named project session using the process working directory.
fn save_current_project_session(editor: &EditorState, name: &str) -> io::Result<PathBuf> {
    let working_directory = env::current_dir()
        .map_err(|error| io::Error::other(format!("failed to read working directory: {error}")))?;
    save_project_session_in_directory(editor, name, working_directory, None)
}

/// Save one named project session and mark it as the active autosave target.
fn save_current_project_session_and_track(
    editor: &EditorState,
    name: &str,
    loaded_session_name: &mut Option<String>,
) -> io::Result<PathBuf> {
    let path = save_current_project_session(editor, name)?;
    *loaded_session_name = Some(name.to_string());
    Ok(path)
}

/// Save one named project session into either the default or one explicit directory and track it.
#[cfg(test)]
fn save_project_session_in_directory_and_track(
    editor: &EditorState,
    name: &str,
    loaded_session_name: &mut Option<String>,
    working_directory: PathBuf,
    sessions_dir: Option<&Path>,
) -> io::Result<PathBuf> {
    let path = save_project_session_in_directory(editor, name, working_directory, sessions_dir)?;
    *loaded_session_name = Some(name.to_string());
    Ok(path)
}

/// Save one named project session to either the default or one explicit sessions directory.
fn save_project_session_in_directory(
    editor: &EditorState,
    name: &str,
    working_directory: PathBuf,
    sessions_dir: Option<&Path>,
) -> io::Result<PathBuf> {
    // Tests can inject a temp sessions directory here without mutating process
    // environment variables, while the runtime path still uses the default store.
    let session = editor.build_project_session(working_directory);
    match sessions_dir {
        Some(dir) => session::save_project_session_in_dir(name, &session, dir),
        None => session::save_project_session(name, &session),
    }
}

/// Persist the currently loaded session name during quit, if one is active.
fn autosave_loaded_session_on_quit(
    editor: &EditorState,
    loaded_session_name: Option<&str>,
) -> io::Result<QuitAutosaveOutcome> {
    let Some(name) = loaded_session_name else {
        return Ok(QuitAutosaveOutcome::NoSession);
    };
    let working_directory = match env::current_dir() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(QuitAutosaveOutcome::SkippedMissingWorkingDirectory {
                session_name: name.to_string(),
            });
        }
        Err(error) => {
            return Err(io::Error::other(format!(
                "failed to read working directory: {error}"
            )));
        }
    };
    autosave_loaded_session_with_resolved_working_directory(editor, name, working_directory, None)
}

/// Save one loaded session on quit, or skip with warning when working directory is missing.
fn autosave_loaded_session_with_resolved_working_directory(
    editor: &EditorState,
    name: &str,
    working_directory: PathBuf,
    sessions_dir: Option<&Path>,
) -> io::Result<QuitAutosaveOutcome> {
    // Some platforms can still resolve a cwd string after directory deletion.
    // Verify the resolved path before using it for autosave.
    match fs::metadata(&working_directory) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Ok(QuitAutosaveOutcome::SkippedMissingWorkingDirectory {
                    session_name: name.to_string(),
                });
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(QuitAutosaveOutcome::SkippedMissingWorkingDirectory {
                session_name: name.to_string(),
            });
        }
        Err(error) => {
            return Err(io::Error::other(format!(
                "failed to verify working directory: {error}"
            )));
        }
    }
    autosave_loaded_session_on_quit_in_directory(
        editor,
        Some(name),
        working_directory,
        sessions_dir,
    )?;
    Ok(QuitAutosaveOutcome::Saved)
}

/// Persist the currently loaded session name into either the default or one explicit directory.
fn autosave_loaded_session_on_quit_in_directory(
    editor: &EditorState,
    loaded_session_name: Option<&str>,
    working_directory: PathBuf,
    sessions_dir: Option<&Path>,
) -> io::Result<()> {
    let Some(name) = loaded_session_name else {
        return Ok(());
    };

    // Quit-time autosave reuses the same serialization path as `:save-session`
    // so session persistence stays consistent whether it is manual or automatic.
    save_project_session_in_directory(editor, name, working_directory, sessions_dir)
        .map(|_| ())
        .map_err(|error| io::Error::other(session_save_error_message(name, &error)))
}

/// Return the user-facing error message for one failed session save.
fn session_save_error_message(name: &str, error: &io::Error) -> String {
    format!("Error saving session \"{name}\": {error}")
}

/// Return one shutdown warning when quit-time session autosave is skipped.
fn quit_autosave_skipped_warning(session_name: &str) -> String {
    format!(
        "Warning: skipped autosaving session \"{session_name}\" on quit because the working directory no longer exists"
    )
}

/// Parse supported CLI flags and positional arguments.
fn parse_cli_args(args: &[String]) -> io::Result<CliArgs> {
    let mut parsed = CliArgs::default();
    let mut idx = 0;

    while idx < args.len() {
        let current = &args[idx];
        if current == "--config" {
            // `--config` consumes the next token as its file path value.
            let Some(next) = args.get(idx + 1) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Missing value for --config",
                ));
            };
            parsed.config_path = Some(next.clone());
            idx += 2;
            continue;
        }

        if let Some(value) = current.strip_prefix("--config=") {
            if value.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Missing value for --config",
                ));
            }
            parsed.config_path = Some(value.to_string());
            idx += 1;
            continue;
        }

        // Bare arguments are startup file paths in the order they were provided.
        parsed.file_paths.push(current.clone());
        idx += 1;
    }

    if parsed.config_path.is_none() && !env_flag_enabled("ORDEX_DISABLE_DEFAULT_CONFIG") {
        parsed.config_path =
            find_default_config_path().map(|path| path.to_string_lossy().into_owned());
    }

    Ok(parsed)
}

/// Resolve the default XDG config path and return it only when the file exists.
fn find_default_config_path() -> Option<PathBuf> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let candidate = resolve_default_config_path(xdg_config_home.as_deref(), home.as_deref())?;
    candidate.is_file().then_some(candidate)
}

/// Let users read startup warnings before entering the TUI screen.
fn wait_for_warning_ack() -> io::Result<()> {
    eprint!("Configuration warnings found. Press Enter to continue...");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

/// Return whether startup warning prompts should pause for user acknowledgement.
///
/// Returns `true` when startup should wait for Enter after printing warnings,
/// and `false` when the warning pause has been disabled by environment.
fn should_pause_for_warnings() -> bool {
    !env_flag_enabled("ORDEX_NO_WARNING_PAUSE")
}

/// Parse a boolean-like environment flag.
///
/// Returns `true` for enabled values such as `1`, `true`, `yes`, or `on`, and
/// `false` when the variable is unset or carries any other value.
fn env_flag_enabled(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| {
        let normalized = value.to_string_lossy().trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
}

/// Print a human-readable startup summary for config loading.
fn emit_config_summary(outcome: &config::ConfigLoadOutcome) {
    let report = &outcome.report;
    let startup = if report.startup_allowed {
        "startup continues"
    } else {
        "startup blocked"
    };

    // Count-based output keeps the banner brief even when many keys were skipped.
    eprintln!(
        "Configuration loaded: {}.\n  Applied sections: {}\n  Skipped sections: {}\n  Defaults used: {}\n  Unknown settings ignored: {}\n  Warnings: {}",
        startup,
        report.applied_sections.len(),
        report.skipped_sections.len(),
        report.defaulted_keys.len(),
        report.ignored_unknown_keys.len(),
        report.warnings.len()
    );
}

/// Return whether config startup should print a summary banner.
///
/// Returns `true` when the load outcome has warnings, skipped/defaulted values,
/// ignored settings, or a startup-blocking error, and `false` for a clean load.
fn should_emit_config_summary(outcome: &config::ConfigLoadOutcome) -> bool {
    let report = &outcome.report;
    !report.warnings.is_empty()
        || !report.skipped_sections.is_empty()
        || !report.defaulted_keys.is_empty()
        || !report.ignored_unknown_keys.is_empty()
        || !report.startup_allowed
}

/// Summarize runtime reload results in one TUI-safe status line.
fn reload_status_message(outcome: &config::ConfigLoadOutcome) -> String {
    match outcome.report.warnings.len() {
        0 => "Config reloaded".to_string(),
        1 => "Config reloaded with 1 warning".to_string(),
        count => format!("Config reloaded with {count} warnings"),
    }
}

/// Build the default config path from environment-derived directories.
fn resolve_default_config_path(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let base = if let Some(xdg) = xdg_config_home {
        xdg.to_path_buf()
    } else {
        home?.join(".config")
    };
    Some(base.join("ordex").join("config.cfg"))
}

/// Initialize optional key logging from `ORDEX_KEY_LOG`.
fn init_key_log() -> io::Result<Option<File>> {
    match env::var("ORDEX_KEY_LOG") {
        // Empty values disable logging even when the variable exists in the environment.
        Ok(path) if !path.trim().is_empty() => OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(Some)
            .map_err(|error| {
                io::Error::other(format!("failed to open ORDEX_KEY_LOG file: {error}"))
            }),
        _ => Ok(None),
    }
}

/// Detect terminal color capability from standard environment hints.
fn detect_color_capability() -> themes::ColorCapability {
    if env_flag_enabled("ORDEX_TRUECOLOR") {
        return themes::ColorCapability::TrueColor;
    }
    themes::detect_color_capability(
        env::var("COLORTERM").ok().as_deref(),
        env::var("TERM").ok().as_deref(),
    )
}

/// Append one key event to the debug key log when `ORDEX_KEY_LOG` is enabled.
fn log_key_event(log: &mut Option<File>, key: Key, mode_before: &str, editor: &EditorState) {
    if let Some(log_file) = log.as_mut() {
        // Logging is best-effort so input handling never fails because debugging is enabled.
        let _ = writeln!(
            log_file,
            "key={:?} mode_before={} mode_after={} cursor={}:{}",
            key,
            mode_before,
            editor.mode_name(),
            editor.cursor_line() + 1,
            editor.cursor_column() + 1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Prefer `XDG_CONFIG_HOME` over `HOME` when both are available.
    #[test]
    fn resolve_default_config_path_prefers_xdg_home() {
        let path = resolve_default_config_path(
            Some(Path::new("/tmp/custom-xdg")),
            Some(Path::new("/home/alice")),
        );
        assert_eq!(
            path,
            Some(PathBuf::from("/tmp/custom-xdg/ordex/config.cfg"))
        );
    }

    /// Fall back to `$HOME/.config` when `XDG_CONFIG_HOME` is unset.
    #[test]
    fn resolve_default_config_path_falls_back_to_home() {
        let path = resolve_default_config_path(None, Some(Path::new("/home/alice")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/alice/.config/ordex/config.cfg"))
        );
    }

    /// Return no path when neither config base directory is available.
    #[test]
    fn resolve_default_config_path_requires_base_directory() {
        assert_eq!(resolve_default_config_path(None, None), None);
    }

    /// Preserve every positional file argument so startup can open multiple buffers.
    #[test]
    fn parse_cli_args_collects_multiple_file_paths() {
        let args = vec![
            "--config".to_string(),
            "config.cfg".to_string(),
            "one.txt".to_string(),
            "two.txt".to_string(),
        ];

        let parsed = parse_cli_args(&args).expect("parse cli args");

        assert_eq!(parsed.config_path.as_deref(), Some("config.cfg"));
        assert_eq!(parsed.file_paths, vec!["one.txt", "two.txt"]);
    }

    /// Autosave should rewrite the loaded session name back to disk during quit.
    #[test]
    fn autosave_loaded_session_on_quit_persists_current_workspace() {
        let session_root =
            std::env::temp_dir().join(format!("ordex_app_session_autosave_{}", std::process::id()));
        let sessions_dir = session_root.join("sessions");
        let _ = fs::remove_dir_all(&session_root);
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let mut editor = EditorState::new(24);
        editor.set_startup_path("src/main.rs");

        save_project_session_in_directory(
            &editor,
            "loaded",
            PathBuf::from("/tmp/project"),
            Some(&sessions_dir),
        )
        .expect("seed session");

        editor
            .open_startup_buffer("src/lib.rs")
            .expect("open extra buffer");

        autosave_loaded_session_on_quit_in_directory(
            &editor,
            Some("loaded"),
            PathBuf::from("/tmp/project"),
            Some(&sessions_dir),
        )
        .expect("autosave session");

        let outcome =
            session::load_project_session_from_dir("loaded", &sessions_dir).expect("load session");
        assert_eq!(outcome.session.buffers.len(), 2);
        assert_eq!(
            outcome.session.working_directory,
            PathBuf::from("/tmp/project")
        );

        let _ = fs::remove_dir_all(&session_root);
    }

    /// Manual session saves should become the autosave target for the next quit.
    #[test]
    fn manual_session_save_becomes_quit_autosave_target() {
        let session_root = std::env::temp_dir().join(format!(
            "ordex_app_session_manual_save_{}",
            std::process::id()
        ));
        let sessions_dir = session_root.join("sessions");
        let project_dir = session_root.join("project");
        let _ = fs::remove_dir_all(&session_root);
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::create_dir_all(&project_dir).expect("create project dir");

        let mut editor = EditorState::new(24);
        editor.set_startup_path("src/main.rs");
        let mut loaded_session_name = None;

        save_project_session_in_directory_and_track(
            &editor,
            "manual",
            &mut loaded_session_name,
            project_dir.clone(),
            Some(&sessions_dir),
        )
        .expect("seed manual session");

        editor
            .open_startup_buffer("src/lib.rs")
            .expect("open extra buffer");

        let quit_finalization = finalize_pending_quit_in_directory(
            &mut editor,
            &loaded_session_name,
            project_dir,
            Some(&sessions_dir),
        );

        assert!(quit_finalization.should_exit);
        assert_eq!(quit_finalization.shutdown_warning, None);
        let outcome =
            session::load_project_session_from_dir("manual", &sessions_dir).expect("load session");
        assert_eq!(outcome.session.buffers.len(), 2);

        let _ = fs::remove_dir_all(&session_root);
    }

    /// Quit autosave should leave existing session files unchanged when no session is active.
    #[test]
    fn autosave_loaded_session_on_quit_leaves_existing_session_unchanged_when_inactive() {
        let session_root =
            std::env::temp_dir().join(format!("ordex_app_session_skip_{}", std::process::id()));
        let sessions_dir = session_root.join("sessions");
        let _ = fs::remove_dir_all(&session_root);
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let seed = session::ProjectSession {
            working_directory: PathBuf::from("/tmp/original"),
            active_buffer: 0,
            buffers: vec![session::SessionBuffer {
                path: PathBuf::from("before.rs"),
                cursor: crate::cursor::Cursor::new(1, 0),
            }],
        };
        session::save_project_session_in_dir("loaded", &seed, &sessions_dir)
            .expect("seed session file");

        let mut editor = EditorState::new(24);
        editor.set_startup_path("after.rs");

        autosave_loaded_session_on_quit_in_directory(
            &editor,
            None,
            PathBuf::from("/tmp/project"),
            Some(&sessions_dir),
        )
        .expect("skip autosave");
        let outcome =
            session::load_project_session_from_dir("loaded", &sessions_dir).expect("load session");
        assert_eq!(outcome.session, seed);

        let _ = fs::remove_dir_all(&session_root);
    }

    /// Finalizing quit should abort exit when autosave cannot write the loaded session.
    #[test]
    fn finalize_pending_quit_aborts_when_autosave_fails() {
        let session_root = std::env::temp_dir().join(format!(
            "ordex_app_session_quit_abort_{}",
            std::process::id()
        ));
        let blocking_path = session_root.join("not_a_directory");
        let project_dir = session_root.join("project");
        let _ = fs::remove_dir_all(&session_root);
        fs::create_dir_all(&session_root).expect("create temp root");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(&blocking_path, "blocker").expect("create blocking file");

        let mut editor = EditorState::new(24);
        editor.set_startup_path("src/main.rs");
        editor.set_mode(crate::mode::Mode::command_with_text("q!"));
        editor.handle_key(Key::Char('\n'));

        let quit_finalization = finalize_pending_quit_in_directory(
            &mut editor,
            &Some("loaded".to_string()),
            project_dir,
            Some(&blocking_path),
        );

        assert!(!quit_finalization.should_exit);
        assert_eq!(quit_finalization.shutdown_warning, None);
        assert!(!editor.should_quit());
        assert!(
            editor
                .status_message()
                .is_some_and(|message| message.starts_with("Error saving session \"loaded\":"))
        );

        let _ = fs::remove_dir_all(&session_root);
    }

    /// Finalizing quit should continue when autosave is skipped for a missing working directory.
    #[test]
    fn finalize_pending_quit_allows_missing_working_directory_autosave_skip() {
        let mut editor = EditorState::new(24);
        editor.set_mode(crate::mode::Mode::command_with_text("q!"));
        editor.handle_key(Key::Char('\n'));

        let quit_finalization = finalize_pending_quit_from_autosave_result(
            &mut editor,
            Ok(QuitAutosaveOutcome::SkippedMissingWorkingDirectory {
                session_name: "loaded".to_string(),
            }),
        );

        assert!(quit_finalization.should_exit);
        assert_eq!(
            quit_finalization.shutdown_warning,
            Some(quit_autosave_skipped_warning("loaded"))
        );
        assert!(editor.should_quit());
    }

    /// Finalizing quit should warn when injected working directory path no longer exists.
    #[test]
    fn finalize_pending_quit_warns_when_injected_working_directory_path_is_missing() {
        let mut editor = EditorState::new(24);
        editor.set_mode(crate::mode::Mode::command_with_text("q!"));
        editor.handle_key(Key::Char('\n'));

        let quit_finalization = finalize_pending_quit_in_directory(
            &mut editor,
            &Some("loaded".to_string()),
            PathBuf::from("/tmp/ordex_missing_working_directory_for_quit_test"),
            None,
        );

        assert!(quit_finalization.should_exit);
        assert_eq!(
            quit_finalization.shutdown_warning,
            Some(quit_autosave_skipped_warning("loaded"))
        );
    }
}
