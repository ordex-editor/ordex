//! Ex-command parsing helpers for `EditorState`.

use super::{OverwriteBehavior, PostSaveAction};
use crate::substitute::{SubstituteCommand, parse_substitute_command};

/// Parsed command-mode input that is ready for execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Command {
    GotoLine(usize),
    Edit(String),
    New,
    BufferNext,
    BufferPrev,
    Buffers,
    BufferDelete,
    Quit {
        force: bool,
        exit_code: i32,
    },
    Update {
        post_save_action: PostSaveAction,
    },
    Undo,
    Redo,
    SaveSession(String),
    OpenSession(String),
    DeleteSession(String),
    Write {
        overwrite_behavior: OverwriteBehavior,
        target: WriteTarget,
        post_save_action: PostSaveAction,
    },
    WriteAll,
    ReloadConfig,
    Diagnostics,
    NextDiagnostic,
    PrevDiagnostic,
    RenameSymbol(String),
    Substitute(SubstituteCommand),
}

/// Target location for a parsed write command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WriteTarget {
    CurrentFile,
    Path(String),
}

/// Error returned when command-mode input does not match a supported command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CommandParseError {
    Unknown(String),
    MissingArgument(&'static str),
    InvalidSubstitute(String),
}

impl CommandParseError {
    /// Convert a parse error into the status message shown to the user.
    pub(super) fn into_status_message(self) -> String {
        match self {
            Self::Unknown(command) => format!("Unknown command: {}", command),
            Self::MissingArgument(command) => format!("{command} requires an argument"),
            Self::InvalidSubstitute(error) => error,
        }
    }
}

/// Parse one command-mode input string into a structured command.
pub(super) fn parse_command(input: &str) -> Result<Command, CommandParseError> {
    let trimmed = input.trim();

    // Numeric input maps directly to the command-mode line jump.
    if let Ok(line_num) = trimmed.parse::<usize>() {
        return Ok(Command::GotoLine(line_num));
    }
    if let Some(result) = parse_substitute_command(trimmed) {
        return result
            .map(Command::Substitute)
            .map_err(CommandParseError::InvalidSubstitute);
    }

    // Split once so `:w path with spaces` preserves the full target path.
    let (name, arg) = match trimmed.split_once(' ') {
        Some((name, arg)) => (name, Some(arg.trim())),
        None => (trimmed, None),
    };

    match (name, arg) {
        ("q", None) => Ok(Command::Quit {
            force: false,
            exit_code: 0,
        }),
        ("q!", None) => Ok(Command::Quit {
            force: true,
            exit_code: 0,
        }),
        ("cq" | "cquit", None) => Ok(Command::Quit {
            force: true,
            exit_code: 1,
        }),
        ("up" | "update", None) => Ok(Command::Update {
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("x", None) => Ok(Command::Update {
            post_save_action: PostSaveAction::QuitOnSuccess,
        }),
        ("u" | "undo", None) => Ok(Command::Undo),
        ("red" | "redo", None) => Ok(Command::Redo),
        ("ss" | "save-session", Some(name)) => Ok(Command::SaveSession(name.to_string())),
        ("os" | "open-session", Some(name)) => Ok(Command::OpenSession(name.to_string())),
        ("ds" | "delete-session", Some(name)) => Ok(Command::DeleteSession(name.to_string())),
        ("e" | "edit", Some(path)) => Ok(Command::Edit(path.to_string())),
        ("new", None) => Ok(Command::New),
        ("bn" | "buffer-next", None) => Ok(Command::BufferNext),
        ("bp" | "buffer-prev", None) => Ok(Command::BufferPrev),
        ("ls" | "buffers", None) => Ok(Command::Buffers),
        ("bd" | "buffer-delete", None) => Ok(Command::BufferDelete),
        ("w", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("w!", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::Force,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("w", Some(filename)) | ("write", Some(filename)) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
            target: WriteTarget::Path(filename.to_string()),
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("w!", Some(filename)) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::Force,
            target: WriteTarget::Path(filename.to_string()),
            post_save_action: PostSaveAction::StayOpen,
        }),
        ("wq", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::QuitOnSuccess,
        }),
        ("wq!", None) => Ok(Command::Write {
            overwrite_behavior: OverwriteBehavior::Force,
            target: WriteTarget::CurrentFile,
            post_save_action: PostSaveAction::QuitOnSuccess,
        }),
        ("wall" | "wa", None) => Ok(Command::WriteAll),
        ("rc" | "reload-config", None) => Ok(Command::ReloadConfig),
        ("dia" | "diagnostics", None) => Ok(Command::Diagnostics),
        ("dn" | "next-diagnostic", None) => Ok(Command::NextDiagnostic),
        ("dp" | "prev-diagnostic", None) => Ok(Command::PrevDiagnostic),
        ("ren" | "rename", Some(new_name)) if !new_name.is_empty() => {
            Ok(Command::RenameSymbol(new_name.to_string()))
        }
        ("ren" | "rename", _) => Err(CommandParseError::MissingArgument("rename")),
        _ => Err(CommandParseError::Unknown(trimmed.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse numeric command input as command-mode go-to-line shorthand.
    #[test]
    fn test_parse_command_parses_line_numbers() {
        assert_eq!(parse_command(" 42 "), Ok(Command::GotoLine(42)));
    }

    /// Parse `:w` paths without splitting away spaces inside the filename.
    #[test]
    fn test_parse_command_preserves_write_target_spacing() {
        assert_eq!(
            parse_command("w  notes and drafts.txt"),
            Ok(Command::Write {
                overwrite_behavior: OverwriteBehavior::ConfirmIfDifferentPath,
                target: WriteTarget::Path("notes and drafts.txt".to_string()),
                post_save_action: PostSaveAction::StayOpen,
            })
        );
    }

    /// Parse force-write-and-quit commands into one structured write request.
    #[test]
    fn test_parse_command_parses_force_write_quit() {
        assert_eq!(
            parse_command("wq!"),
            Ok(Command::Write {
                overwrite_behavior: OverwriteBehavior::Force,
                target: WriteTarget::CurrentFile,
                post_save_action: PostSaveAction::QuitOnSuccess,
            })
        );
    }

    /// Parse new, write-all, and conditional-quit aliases into structured commands.
    #[test]
    fn test_parse_command_parses_new_write_all_and_x() {
        assert_eq!(parse_command("new"), Ok(Command::New));
        assert_eq!(parse_command("wall"), Ok(Command::WriteAll));
        assert_eq!(parse_command("wa"), Ok(Command::WriteAll));
        assert_eq!(
            parse_command("x"),
            Ok(Command::Update {
                post_save_action: PostSaveAction::QuitOnSuccess,
            })
        );
    }

    /// Parse substitute commands into a structured command variant.
    #[test]
    fn test_parse_command_parses_substitute_commands() {
        assert_eq!(
            parse_command("s/foo/bar/"),
            Ok(Command::Substitute(SubstituteCommand {
                scope: crate::substitute::SubstituteScope::CurrentLine,
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
            }))
        );
        assert_eq!(
            parse_command(r"%s#([a-z]+)-(\d+)#$2:$1#"),
            Ok(Command::Substitute(SubstituteCommand {
                scope: crate::substitute::SubstituteScope::WholeFile,
                pattern: r"([a-z]+)-(\d+)".to_string(),
                replacement: "$2:$1".to_string(),
            }))
        );
        assert_eq!(
            parse_command("s/foo/bar"),
            Ok(Command::Substitute(SubstituteCommand {
                scope: crate::substitute::SubstituteScope::CurrentLine,
                pattern: "foo".to_string(),
                replacement: "bar".to_string(),
            }))
        );
    }

    /// Parse both long and short aliases for buffer commands.
    #[test]
    fn test_parse_command_parses_buffer_aliases() {
        assert_eq!(parse_command("bn"), Ok(Command::BufferNext));
        assert_eq!(parse_command("buffer-prev"), Ok(Command::BufferPrev));
        assert_eq!(parse_command("ls"), Ok(Command::Buffers));
        assert_eq!(parse_command("buffer-delete"), Ok(Command::BufferDelete));
        assert_eq!(
            parse_command("save-session project-one"),
            Ok(Command::SaveSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("open-session project-one"),
            Ok(Command::OpenSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("delete-session project-one"),
            Ok(Command::DeleteSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("e notes.txt"),
            Ok(Command::Edit("notes.txt".to_string()))
        );
    }

    /// Parse short aliases for command-mode commands that otherwise need long names.
    #[test]
    fn test_parse_command_parses_short_aliases() {
        assert_eq!(
            parse_command("cq"),
            Ok(Command::Quit {
                force: true,
                exit_code: 1,
            })
        );
        assert_eq!(
            parse_command("up"),
            Ok(Command::Update {
                post_save_action: PostSaveAction::StayOpen,
            })
        );
        assert_eq!(parse_command("u"), Ok(Command::Undo));
        assert_eq!(parse_command("red"), Ok(Command::Redo));
        assert_eq!(parse_command("dia"), Ok(Command::Diagnostics));
        assert_eq!(parse_command("dn"), Ok(Command::NextDiagnostic));
        assert_eq!(parse_command("dp"), Ok(Command::PrevDiagnostic));
        assert_eq!(parse_command("rc"), Ok(Command::ReloadConfig));
        assert_eq!(
            parse_command("ren helper_total"),
            Ok(Command::RenameSymbol("helper_total".to_string()))
        );
        assert_eq!(
            parse_command("ss project-one"),
            Ok(Command::SaveSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("os project-one"),
            Ok(Command::OpenSession("project-one".to_string()))
        );
        assert_eq!(
            parse_command("ds project-one"),
            Ok(Command::DeleteSession("project-one".to_string()))
        );
    }
}
