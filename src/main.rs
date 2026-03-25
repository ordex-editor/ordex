#![allow(clippy::question_mark)]

//! Ordex - A TUI text editor
//!
//! This is the main entry point for the ordex text editor.
//! It handles CLI argument parsing, file loading, terminal initialization,
//! and the main event loop.

// TODO: Write the asciidoctor doc for ordex (possibly using Hugo if asciidoctor alone is not
// enough).

mod config;
mod cursor;
mod editor_state;
mod keybindings;
mod mode;
mod navigation;
mod render;
mod signal;
mod soft_wrap;
mod syntax;
mod text_buffer;
mod themes;
mod tui;
mod viewport;

use editor_state::{EditorRequest, EditorState};
use render::{
    RenderDecision, RenderSnapshot, TerminalSize, render_editor, render_message_line,
    render_status_cursor, render_vertical_cursor_motion, resize_editor,
};
use signal::SignalGuard;
use std::env;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use termion::event::Key;

/// Entry point for the application
///
/// Delegates to run() and handles errors by printing to stderr
fn main() {
    match run() {
        Ok(0) => {}
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

/// Main application logic
///
/// Loads the file, initializes the terminal, and runs the event loop
fn run() -> io::Result<i32> {
    let args: Vec<String> = env::args().collect();
    let cli_args = parse_cli_args(&args[1..])?;
    let config_path = cli_args.config_path.clone();
    let config_outcome = cli_args
        .config_path
        .as_deref()
        .map(|config_path| config::load_config(Path::new(config_path)));

    if let Some(outcome) = &config_outcome {
        config::emit_startup_warnings(&outcome.report.warnings);
        if should_emit_config_summary(outcome) {
            emit_config_summary(outcome);
        }
        if !outcome.report.warnings.is_empty() && should_pause_for_warnings() {
            wait_for_warning_ack()?;
        }
    }

    // Initialize terminal
    let mut term = tui::Terminal::new()?;
    term.clear_screen()?;

    let mut terminal_size = TerminalSize::from_termion(termion::terminal_size()?);
    let signals = SignalGuard::install()?;

    // Initialize editor state with terminal height
    let mut editor = EditorState::new(terminal_size.height as usize);
    editor.set_color_capability(detect_color_capability());

    if let Some(outcome) = &config_outcome {
        editor.replace_config(&outcome.settings);
    }

    if let Some(path) = &cli_args.file_path {
        if std::path::Path::new(path).exists() {
            editor.load_file(path)?;
        } else {
            // New file with specified name
            editor.file_path = std::path::PathBuf::from(path);
            editor.refresh_syntax();
        }
    }

    let mut key_log = init_key_log()?;

    let mut needs_render = true;
    let mut needs_message_render = false;
    let mut needs_cursor_render = false;
    let mut needs_vertical_cursor_render = None;
    // The discovery popup can temporarily hide the terminal cursor when it lands
    // on top of the logical cursor cell. Track that across redraws so we only
    // emit `Show`/`Hide` when the visibility state actually changes.
    let mut cursor_hidden_by_overlay = false;
    signals.mark_resize_pending();

    // Main event loop
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
                resize_editor(&mut editor, terminal_size);
                needs_render = true;
            }
        }

        if needs_render {
            // Render current view
            render_editor(
                &mut term,
                &mut editor,
                terminal_size,
                &mut cursor_hidden_by_overlay,
            )?;

            // Clear status message after displaying
            editor.status_message = None;
            needs_render = false;
            needs_message_render = false;
            needs_cursor_render = false;
            needs_vertical_cursor_render = None;
        } else if let Some(previous_cursor_line) = needs_vertical_cursor_render.take() {
            render_vertical_cursor_motion(
                &mut term,
                &mut editor,
                terminal_size,
                previous_cursor_line,
                &mut cursor_hidden_by_overlay,
            )?;
            needs_cursor_render = false;
        } else if needs_cursor_render {
            render_status_cursor(
                &mut term,
                &editor,
                terminal_size,
                &mut cursor_hidden_by_overlay,
            )?;
            needs_cursor_render = false;
        } else if needs_message_render {
            render_message_line(
                &mut term,
                &editor,
                terminal_size,
                &mut cursor_hidden_by_overlay,
            )?;
            editor.status_message = None;
            needs_message_render = false;
        }

        // Block for input; SIGWINCH interrupts this read to trigger a resize redraw.
        match tui::Terminal::read_key() {
            Ok(key) => {
                let before_mode = editor.mode.mode_label();
                // Capture state before handling input so we can decide the minimal
                // redraw needed after applying the key.
                let before = RenderSnapshot::capture(&editor);
                editor.handle_key(key);
                handle_editor_request(&mut editor, config_path.as_deref());
                log_key_event(&mut key_log, key, before_mode, &editor);
                if editor.should_quit {
                    break;
                }
                let after = RenderSnapshot::capture(&editor);
                match RenderSnapshot::decide(&before, &after) {
                    RenderDecision::Full => {
                        needs_render = true;
                        needs_message_render = false;
                        needs_cursor_render = false;
                        needs_vertical_cursor_render = None;
                    }
                    RenderDecision::VerticalCursor => {
                        if !needs_render {
                            needs_vertical_cursor_render = Some(before.cursor_line());
                            needs_message_render = false;
                            needs_cursor_render = false;
                        }
                    }
                    RenderDecision::CursorOnly => {
                        if !needs_render && needs_vertical_cursor_render.is_none() {
                            needs_cursor_render = true;
                        }
                    }
                    RenderDecision::MessageOnly => {
                        if !needs_render
                            && needs_vertical_cursor_render.is_none()
                            && !needs_cursor_render
                        {
                            needs_message_render = true;
                        }
                    }
                    RenderDecision::None => {}
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                // Signals interrupt the blocking read; the next loop iteration
                // decides whether that means resize handling or termination.
            }
            Err(e) => return Err(e),
        }
    }

    Ok(editor.quit_exit_code())
}

/// Run deferred editor requests that need process-level state from `run()`.
///
/// The editor parses commands while handling keys, but it deliberately does not
/// own CLI arguments or perform config-file I/O directly. `pending_request`
/// bridges that boundary: `EditorState` records "what should happen next", and
/// the main loop executes it once it has returned to the layer that owns the
/// active config path and other application-wide resources.
fn handle_editor_request(editor: &mut EditorState, config_path: Option<&str>) {
    match editor.take_pending_request() {
        Some(EditorRequest::ReloadConfig) => reload_editor_config(editor, config_path),
        None => {}
    }
}

/// Reload configuration from the active config path and apply it immediately.
fn reload_editor_config(editor: &mut EditorState, config_path: Option<&str>) {
    let Some(config_path) = config_path else {
        editor.status_message = Some("No config file to reload".to_string());
        return;
    };

    let outcome = config::load_config(Path::new(config_path));
    editor.replace_config(&outcome.settings);
    editor.status_message = Some(reload_status_message(&outcome));
}

#[derive(Debug, Default)]
struct CliArgs {
    file_path: Option<String>,
    config_path: Option<String>,
}

/// Parse supported CLI flags and positional arguments.
fn parse_cli_args(args: &[String]) -> io::Result<CliArgs> {
    let mut parsed = CliArgs::default();
    let mut idx = 0;
    while idx < args.len() {
        let current = &args[idx];
        if current == "--config" {
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

        if parsed.file_path.is_none() {
            parsed.file_path = Some(current.clone());
        }
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
///
/// When set to a non-empty path, events are appended to that file.
fn init_key_log() -> io::Result<Option<File>> {
    match env::var("ORDEX_KEY_LOG") {
        Ok(path) if !path.trim().is_empty() => OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(Some)
            .map_err(|e| io::Error::other(format!("failed to open ORDEX_KEY_LOG file: {e}"))),
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

/// Append one key event to the debug key log (when enabled).
fn log_key_event(log: &mut Option<File>, key: Key, mode_before: &str, editor: &EditorState) {
    if let Some(log_file) = log.as_mut() {
        let _ = writeln!(
            log_file,
            "key={:?} mode_before={} mode_after={} cursor={}:{}",
            key,
            mode_before,
            editor.mode_name(),
            editor.cursor.line() + 1,
            editor.cursor.column() + 1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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

    #[test]
    fn resolve_default_config_path_falls_back_to_home() {
        let path = resolve_default_config_path(None, Some(Path::new("/home/alice")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/alice/.config/ordex/config.cfg"))
        );
    }

    #[test]
    fn resolve_default_config_path_requires_base_directory() {
        assert_eq!(resolve_default_config_path(None, None), None);
    }
}
