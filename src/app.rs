//! Application startup and runtime orchestration.

use crate::config;
use crate::editor_state::{DeferredWrite, EditorRequest, EditorState};
use crate::render::{
    RenderDecision, RenderSnapshot, TerminalSize, render_editor, render_message_line,
    render_status_cursor, render_vertical_cursor_motion, resize_editor,
};
use crate::signal::SignalGuard;
use crate::themes;
use crate::tui;
use std::env;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::time::Duration;
use termion::event::Key;

#[derive(Debug, Default)]
struct CliArgs {
    file_paths: Vec<String>,
    config_path: Option<String>,
}

/// Launch the application and translate runtime results into process exit behavior.
pub(crate) fn launch() {
    match run() {
        Ok(0) => {}
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("Error: {error}");
            process::exit(1);
        }
    }
}

/// Execute startup, terminal setup, and the interactive editor runtime.
fn run() -> io::Result<i32> {
    let args: Vec<String> = env::args().collect();
    let cli_args = parse_cli_args(&args[1..])?;
    let config_outcome = load_startup_config(cli_args.config_path.as_deref())?;

    // Startup warnings must stay on the shell screen before raw mode takes over.
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    let terminal_size = TerminalSize::from_termion(termion::terminal_size()?);
    let signals = SignalGuard::install()?;
    let mut editor = initialize_editor(&cli_args, config_outcome.as_ref(), terminal_size.height)?;
    let mut key_log = init_key_log()?;

    run_event_loop(
        &mut term,
        &signals,
        &mut editor,
        cli_args.config_path.as_deref(),
        terminal_size,
        &mut key_log,
    )
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
    config_path: Option<&str>,
    mut terminal_size: TerminalSize,
    key_log: &mut Option<File>,
) -> io::Result<i32> {
    const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_millis(50);
    let mut needs_render = true;
    let mut needs_message_render = false;
    let mut needs_cursor_render = false;
    let mut needs_vertical_cursor_render = None;
    // The discovery popup can temporarily hide the terminal cursor when it lands
    // on top of the logical cursor cell. Track that across redraws so we only
    // emit `Show`/`Hide` when the visibility state actually changes.
    let mut cursor_hidden_by_overlay = false;
    signals.mark_resize_pending();

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
            editor.clear_status_message();
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
            editor.clear_status_message();
            needs_message_render = false;
        }

        // Poll only while asynchronous picker work is active so other modes keep
        // the original blocking input behavior and its existing escape timing.
        let next_key = if editor.needs_background_poll() {
            tui::Terminal::read_key_timeout(BACKGROUND_POLL_INTERVAL)
        } else {
            tui::Terminal::read_key().map(Some)
        };
        match next_key {
            Ok(Some(key)) => {
                let mode_before = editor.mode_name();
                // Compare visible state before and after the key to pick the smallest redraw.
                let before = RenderSnapshot::capture(editor);
                editor.handle_key(key);
                handle_editor_request(editor, config_path);
                log_key_event(key_log, key, mode_before, editor);
                if editor.should_quit() {
                    break;
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
                if !editor.poll_background_tasks() {
                    continue;
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

    Ok(editor.quit_exit_code())
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

/// Run deferred editor requests that need process-level state from the app layer.
///
/// The editor parses commands while handling keys, but it deliberately does not
/// own CLI arguments or perform config-file or buffer-write I/O directly.
/// `pending_request` bridges that boundary: `EditorState` records the next
/// process-level action, and the application loop executes it once control
/// returns to the layer that owns the active config path and filesystem access.
fn handle_editor_request(editor: &mut EditorState, config_path: Option<&str>) {
    match editor.take_pending_request() {
        Some(EditorRequest::ReloadConfig) => reload_editor_config(editor, config_path),
        Some(EditorRequest::WriteBuffer(write)) => execute_deferred_write(editor, write),
        None => {}
    }
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

/// Execute one deferred buffer-write request against the filesystem.
pub(crate) fn execute_deferred_write(editor: &mut EditorState, write: DeferredWrite) {
    match File::create(&write.path) {
        Ok(mut file) => match editor.write_buffer_to(&mut file) {
            Ok(()) => editor.complete_deferred_write(write),
            Err(error) => editor.report_file_write_error(error),
        },
        Err(error) => editor.report_file_create_error(error),
    }
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
fn should_pause_for_warnings() -> bool {
    !env_flag_enabled("ORDEX_NO_WARNING_PAUSE")
}

/// Parse a boolean-like environment flag.
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
}
