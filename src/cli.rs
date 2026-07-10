//! Command-line parsing for Ordex startup.

use std::env;
use std::io;
use std::path::{Path, PathBuf};

/// Parsed command-line action requested before the interactive runtime starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliCommand {
    Launch(CliArgs),
    PrintHelp,
    PrintVersion,
}

/// Startup arguments consumed by the application runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CliArgs {
    pub(crate) file_paths: Vec<String>,
    pub(crate) config_path: Option<String>,
    pub(crate) lsp_config_path: Option<String>,
}

/// Parse process command-line arguments after the binary name.
pub(crate) fn parse_env_args() -> io::Result<CliCommand> {
    let args: Vec<String> = env::args().skip(1).collect();
    parse_args(&args)
}

/// Build the user-facing version string.
pub(crate) fn version_string() -> String {
    format!("ordex v{}", env!("CARGO_PKG_VERSION"))
}

/// Build the user-facing help text for startup flags and arguments.
pub(crate) fn help_string() -> String {
    [
        "ordex - A terminal text editor",
        "",
        "Usage: ordex [OPTIONS] [FILE...]",
        "",
        "Options:",
        "  -h, --help            Print this help message and exit",
        "  -V, --version         Print version information and exit",
        "      --config PATH     Path to editor configuration file",
        "      --lsp-config PATH Path to LSP configuration file",
        "",
        "Arguments:",
        "  FILE                  One or more files to open at startup",
        "",
        "Notes:",
        "  - Multiple files open in separate buffers; the first file is active.",
        "  - Use `--` before a dash-prefixed path so it is treated as a filename.",
        "",
        "Documentation: https://ordex-editor.github.io/ordex/",
    ]
    .join("\n")
}

/// Parse supported CLI flags and positional arguments.
fn parse_args(args: &[String]) -> io::Result<CliCommand> {
    parse_args_with_default_config(args, true)
}

/// Parse CLI arguments with optional default-config discovery.
fn parse_args_with_default_config(
    args: &[String],
    include_default_config: bool,
) -> io::Result<CliCommand> {
    let mut parsed = CliArgs::default();
    let mut idx = 0;
    let mut positional_only = false;

    while idx < args.len() {
        let current = &args[idx];
        if positional_only {
            parsed.file_paths.push(current.clone());
            idx += 1;
            continue;
        }

        if current == "--" {
            positional_only = true;
            idx += 1;
            continue;
        }

        if current == "--help" || current == "-h" {
            return Ok(CliCommand::PrintHelp);
        }

        if current == "--version" || current == "-V" {
            return Ok(CliCommand::PrintVersion);
        }

        if current == "--config" {
            // `--config` consumes the next token as its file path value.
            let Some(next) = args.get(idx + 1) else {
                return Err(invalid_input("Missing value for --config"));
            };
            parsed.config_path = Some(next.clone());
            idx += 2;
            continue;
        }

        if current == "--lsp-config" {
            // `--lsp-config` consumes the next token as its file path value.
            let Some(next) = args.get(idx + 1) else {
                return Err(invalid_input("Missing value for --lsp-config"));
            };
            parsed.lsp_config_path = Some(next.clone());
            idx += 2;
            continue;
        }

        if let Some(value) = current.strip_prefix("--config=") {
            if value.is_empty() {
                return Err(invalid_input("Missing value for --config"));
            }
            parsed.config_path = Some(value.to_string());
            idx += 1;
            continue;
        }

        if let Some(value) = current.strip_prefix("--lsp-config=") {
            if value.is_empty() {
                return Err(invalid_input("Missing value for --lsp-config"));
            }
            parsed.lsp_config_path = Some(value.to_string());
            idx += 1;
            continue;
        }

        if current.starts_with('-') && current != "-" {
            return Err(invalid_input(format!("Unknown flag: {current}")));
        }

        // Bare arguments are startup file paths in the order they were provided.
        parsed.file_paths.push(current.clone());
        idx += 1;
    }

    if include_default_config
        && parsed.config_path.is_none()
        && !env_flag_enabled("ORDEX_DISABLE_DEFAULT_CONFIG")
    {
        parsed.config_path =
            find_default_config_path().map(|path| path.to_string_lossy().into_owned());
    }
    if include_default_config
        && parsed.lsp_config_path.is_none()
        && !env_flag_enabled("ORDEX_DISABLE_DEFAULT_CONFIG")
    {
        parsed.lsp_config_path =
            find_default_lsp_config_path().map(|path| path.to_string_lossy().into_owned());
    }

    Ok(CliCommand::Launch(parsed))
}

/// Return an invalid-input error with one user-facing message.
fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
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

/// Resolve the default XDG LSP config path and return it only when the file exists.
fn find_default_lsp_config_path() -> Option<PathBuf> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let candidate = resolve_default_lsp_config_path(xdg_config_home.as_deref(), home.as_deref())?;
    candidate.is_file().then_some(candidate)
}

/// Parse a boolean-like environment flag.
///
/// Returns `true` for enabled values such as `1`, `true`, `yes`, or `on`, and
/// `false` when the variable is unset or carries any other value.
pub(crate) fn env_flag_enabled(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| {
        let normalized = value.to_string_lossy().trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
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

/// Build the default LSP config path from environment-derived directories.
fn resolve_default_lsp_config_path(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let base = if let Some(xdg) = xdg_config_home {
        xdg.to_path_buf()
    } else {
        home?.join(".config")
    };
    Some(base.join("ordex").join("lsp.cfg"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_args_collects_multiple_file_paths() {
        let args = vec![
            "--config".to_string(),
            "config.cfg".to_string(),
            "one.txt".to_string(),
            "two.txt".to_string(),
        ];

        let parsed = parse_args_with_default_config(&args, false).expect("parse cli args");

        assert_eq!(
            parsed,
            CliCommand::Launch(CliArgs {
                config_path: Some("config.cfg".to_string()),
                lsp_config_path: None,
                file_paths: vec!["one.txt".to_string(), "two.txt".to_string()],
            })
        );
    }

    /// Report `--version` as a non-interactive startup action.
    #[test]
    fn parse_args_recognizes_version_flag() {
        let args = vec!["--version".to_string()];

        let parsed = parse_args_with_default_config(&args, false).expect("parse version flag");

        assert_eq!(parsed, CliCommand::PrintVersion);
    }

    /// Report `--help` as a non-interactive startup action.
    #[test]
    fn parse_args_recognizes_help_flag() {
        let args = vec!["--help".to_string()];

        let parsed = parse_args_with_default_config(&args, false).expect("parse help flag");

        assert_eq!(parsed, CliCommand::PrintHelp);
    }

    /// Report `-h` as a non-interactive startup action.
    #[test]
    fn parse_args_recognizes_short_help_flag() {
        let args = vec!["-h".to_string()];

        let parsed = parse_args_with_default_config(&args, false).expect("parse short help flag");

        assert_eq!(parsed, CliCommand::PrintHelp);
    }

    /// Stop parsing once `--help` is seen so later flags are ignored.
    #[test]
    fn parse_args_help_exits_before_later_flags() {
        let args = vec![
            "--help".to_string(),
            "--config".to_string(),
            "x.cfg".to_string(),
        ];

        let parsed = parse_args_with_default_config(&args, false).expect("parse help flag");

        assert_eq!(parsed, CliCommand::PrintHelp);
    }

    /// Treat `--help` as a filename after the end-of-options marker.
    #[test]
    fn parse_args_allows_help_token_after_marker() {
        let args = vec!["--".to_string(), "--help".to_string()];

        let parsed = parse_args_with_default_config(&args, false).expect("parse cli args");

        assert_eq!(
            parsed,
            CliCommand::Launch(CliArgs {
                config_path: None,
                lsp_config_path: None,
                file_paths: vec!["--help".to_string()],
            })
        );
    }

    /// Include usage, options, and documentation link in help output.
    #[test]
    fn help_string_lists_supported_startup_options() {
        let help = help_string();

        assert!(help.contains("Usage: ordex [OPTIONS] [FILE...]"));
        assert!(help.contains("--help"));
        assert!(help.contains("--version"));
        assert!(help.contains("--config PATH"));
        assert!(help.contains("--lsp-config PATH"));
        assert!(help.contains("https://ordex-editor.github.io/ordex/"));
    }

    /// Reject unknown long flags before startup can treat them as filenames.
    #[test]
    fn parse_args_rejects_unknown_long_flags() {
        let args = vec!["--bogus".to_string()];

        let error =
            parse_args_with_default_config(&args, false).expect_err("unknown flag should fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "Unknown flag: --bogus");
    }

    /// Reject unknown short flags before startup can treat them as filenames.
    #[test]
    fn parse_args_rejects_unknown_short_flags() {
        let args = vec!["-z".to_string()];

        let error =
            parse_args_with_default_config(&args, false).expect_err("unknown flag should fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "Unknown flag: -z");
    }

    /// Preserve dash-prefixed filenames after the end-of-options marker.
    #[test]
    fn parse_args_allows_dash_prefixed_paths_after_marker() {
        let args = vec!["--".to_string(), "--notes".to_string()];

        let parsed = parse_args_with_default_config(&args, false).expect("parse cli args");

        assert_eq!(
            parsed,
            CliCommand::Launch(CliArgs {
                config_path: None,
                lsp_config_path: None,
                file_paths: vec!["--notes".to_string()],
            })
        );
    }

    /// Resolve the default LSP config path from `XDG_CONFIG_HOME`.
    #[test]
    fn resolve_default_lsp_config_path_prefers_xdg_home() {
        let path = resolve_default_lsp_config_path(
            Some(Path::new("/tmp/custom-xdg")),
            Some(Path::new("/home/alice")),
        );
        assert_eq!(path, Some(PathBuf::from("/tmp/custom-xdg/ordex/lsp.cfg")));
    }

    /// Fall back to `$HOME/.config` when the XDG base directory is missing.
    #[test]
    fn resolve_default_lsp_config_path_falls_back_to_home() {
        let path = resolve_default_lsp_config_path(None, Some(Path::new("/home/alice")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/alice/.config/ordex/lsp.cfg"))
        );
    }

    /// Parse `--lsp-config` and keep it alongside positional file paths.
    #[test]
    fn parse_args_collects_lsp_config_flag() {
        let args = vec![
            "--lsp-config".to_string(),
            "lsp.cfg".to_string(),
            "main.rs".to_string(),
        ];

        let parsed = parse_args_with_default_config(&args, false).expect("parse cli args");

        assert_eq!(
            parsed,
            CliCommand::Launch(CliArgs {
                config_path: None,
                lsp_config_path: Some("lsp.cfg".to_string()),
                file_paths: vec!["main.rs".to_string()],
            })
        );
    }
}
