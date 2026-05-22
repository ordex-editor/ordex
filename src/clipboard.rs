//! System clipboard integration for X11 and Wayland sessions.

use crate::editor_state::PastePosition;
use std::fmt;
use std::io;
use std::io::Write;
use std::process::{Command, Stdio};

/// Distinguish the Vim-style system clipboard registers supported by Ordex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ClipboardRegister {
    Clipboard,
    Primary,
}

impl ClipboardRegister {
    /// Return the Vim register character for this clipboard target.
    pub(crate) fn key_char(self) -> char {
        match self {
            Self::Clipboard => '+',
            Self::Primary => '*',
        }
    }

    /// Return the X11 selection name for this clipboard target.
    fn x11_selection_name(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Primary => "primary",
        }
    }
}

/// Preserve the editor-side paste shape associated with clipboard text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardPayloadKind {
    Character,
    Line,
    Block,
}

impl ClipboardPayloadKind {
    /// Infer one best-effort payload kind from externally supplied clipboard text.
    pub(crate) fn infer(text: &str) -> Self {
        if text.ends_with('\n') {
            return Self::Line;
        }
        Self::Character
    }
}

/// Clipboard text plus the editor-side paste semantics associated with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardPayload {
    pub(crate) text: String,
    pub(crate) kind: ClipboardPayloadKind,
}

/// One deferred clipboard write request emitted by `EditorState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardWriteRequest {
    pub(crate) register: ClipboardRegister,
    pub(crate) payload: ClipboardPayload,
}

/// One deferred clipboard read request emitted by `EditorState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardPasteRequest {
    pub(crate) register: ClipboardRegister,
    pub(crate) position: PastePosition,
    pub(crate) count: usize,
}

/// Runtime clipboard state shared by the outer app loop.
#[derive(Debug)]
pub(crate) struct ClipboardState {
    backend: Option<ClipboardBackend>,
    clipboard_cache: Option<ClipboardPayload>,
    primary_cache: Option<ClipboardPayload>,
}

impl ClipboardState {
    /// Detect the active clipboard backend once for the current Ordex session.
    pub(crate) fn new() -> Self {
        Self {
            backend: ClipboardBackend::detect().ok(),
            clipboard_cache: None,
            primary_cache: None,
        }
    }

    /// Write one payload into the selected system clipboard register.
    pub(crate) fn write(&mut self, request: &ClipboardWriteRequest) -> Result<(), ClipboardError> {
        let backend = self.backend.ok_or(ClipboardError::MissingSession)?;
        backend.write_text(request.register, &request.payload.text)?;
        *self.cache_slot_mut(request.register) = Some(request.payload.clone());
        Ok(())
    }

    /// Read one payload from the selected system clipboard register.
    pub(crate) fn read(
        &mut self,
        register: ClipboardRegister,
    ) -> Result<ClipboardPayload, ClipboardError> {
        let backend = self.backend.ok_or(ClipboardError::MissingSession)?;
        let text = backend.read_text(register)?;

        // Preserve linewise or blockwise semantics when Ordex itself wrote the
        // clipboard and the external text still matches that cached payload.
        if let Some(cached) = self.cache_slot(register).cloned()
            && cached.text == text
        {
            return Ok(cached);
        }

        Ok(ClipboardPayload {
            kind: ClipboardPayloadKind::infer(&text),
            text,
        })
    }

    /// Return the cached payload slot for `register`, if any.
    fn cache_slot(&self, register: ClipboardRegister) -> Option<&ClipboardPayload> {
        match register {
            ClipboardRegister::Clipboard => self.clipboard_cache.as_ref(),
            ClipboardRegister::Primary => self.primary_cache.as_ref(),
        }
    }

    /// Return the mutable cached payload slot for `register`.
    fn cache_slot_mut(&mut self, register: ClipboardRegister) -> &mut Option<ClipboardPayload> {
        match register {
            ClipboardRegister::Clipboard => &mut self.clipboard_cache,
            ClipboardRegister::Primary => &mut self.primary_cache,
        }
    }
}

/// Errors surfaced while talking to external clipboard tools.
#[derive(Debug)]
pub(crate) enum ClipboardError {
    MissingSession,
    MissingTool {
        tool: &'static str,
        backend: &'static str,
    },
    UnsupportedPrimarySelection,
    LaunchFailed {
        tool: &'static str,
        error: io::Error,
    },
    Io {
        tool: &'static str,
        error: io::Error,
    },
    CommandFailed {
        tool: &'static str,
        details: String,
    },
}

impl fmt::Display for ClipboardError {
    /// Render this clipboard failure as one user-facing status message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSession => {
                write!(
                    f,
                    "Clipboard unavailable: no X11 or Wayland session detected"
                )
            }
            Self::MissingTool { tool, backend } => {
                write!(f, "Clipboard unavailable: missing {tool} for {}", backend)
            }
            Self::UnsupportedPrimarySelection => write!(
                f,
                "Clipboard unavailable: Wayland primary selection for \"* is unsupported"
            ),
            Self::LaunchFailed { tool, error } => {
                write!(f, "Clipboard command {tool} failed to start: {error}")
            }
            Self::Io { tool, error } => {
                write!(f, "Clipboard command {tool} failed: {error}")
            }
            Self::CommandFailed { tool, details } => {
                write!(f, "Clipboard command {tool} failed: {details}")
            }
        }
    }
}

/// Distinguish the external clipboard command family selected for this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardBackend {
    Wayland,
    X11,
}

impl ClipboardBackend {
    /// Detect the active clipboard backend from the current process environment.
    fn detect() -> Result<Self, ClipboardError> {
        if Self::session_type_is("wayland") {
            return Ok(Self::Wayland);
        }
        if Self::session_type_is("x11") {
            return Ok(Self::X11);
        }
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Ok(Self::Wayland);
        }
        if std::env::var_os("DISPLAY").is_some() {
            return Ok(Self::X11);
        }
        Err(ClipboardError::MissingSession)
    }

    /// Return the user-facing backend name used in error messages.
    fn display_name(self) -> &'static str {
        match self {
            Self::Wayland => "Wayland",
            Self::X11 => "X11",
        }
    }

    /// Return whether the current session type exactly matches `expected`.
    fn session_type_is(expected: &str) -> bool {
        std::env::var_os("XDG_SESSION_TYPE").is_some_and(|value| value == expected)
    }

    /// Write one UTF-8 payload into the selected system clipboard register.
    fn write_text(self, register: ClipboardRegister, text: &str) -> Result<(), ClipboardError> {
        match self {
            Self::Wayland => {
                let args = match register {
                    ClipboardRegister::Clipboard => Vec::new(),
                    ClipboardRegister::Primary => vec!["--primary"],
                };
                self.run_write_command("wl-copy", &args, text, register)
            }
            Self::X11 => self.run_write_command(
                "xclip",
                &["-selection", register.x11_selection_name()],
                text,
                register,
            ),
        }
    }

    /// Read one UTF-8 payload from the selected system clipboard register.
    fn read_text(self, register: ClipboardRegister) -> Result<String, ClipboardError> {
        match self {
            Self::Wayland => {
                let args = match register {
                    ClipboardRegister::Clipboard => vec!["--no-newline"],
                    ClipboardRegister::Primary => vec!["--no-newline", "--primary"],
                };
                self.run_read_command("wl-paste", &args, register)
            }
            Self::X11 => self.run_read_command(
                "xclip",
                &["-o", "-selection", register.x11_selection_name()],
                register,
            ),
        }
    }

    /// Spawn one clipboard write command and feed `text` through stdin.
    fn run_write_command(
        self,
        tool: &'static str,
        args: &[&str],
        text: &str,
        register: ClipboardRegister,
    ) -> Result<(), ClipboardError> {
        let mut child = self.spawn_command(tool, args, true)?;

        // Write the payload before waiting so the clipboard helper can start
        // owning the selection using the full text stream.
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|error| ClipboardError::Io { tool, error })?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| ClipboardError::Io { tool, error })?;
        self.ensure_success(tool, output, register).map(|_| ())
    }

    /// Spawn one clipboard read command and collect stdout as UTF-8 text.
    fn run_read_command(
        self,
        tool: &'static str,
        args: &[&str],
        register: ClipboardRegister,
    ) -> Result<String, ClipboardError> {
        let output = self.run_command(tool, args)?;
        self.ensure_success(tool, output, register)
            .and_then(|stdout| {
                String::from_utf8(stdout).map_err(|error| ClipboardError::CommandFailed {
                    tool,
                    details: error.to_string(),
                })
            })
    }

    /// Spawn one clipboard command and wait for its collected output.
    fn run_command(
        self,
        tool: &'static str,
        args: &[&str],
    ) -> Result<std::process::Output, ClipboardError> {
        let child = self.spawn_command(tool, args, false)?;
        child
            .wait_with_output()
            .map_err(|error| ClipboardError::Io { tool, error })
    }

    /// Spawn one clipboard command with the requested stdin behavior.
    fn spawn_command(
        self,
        tool: &'static str,
        args: &[&str],
        with_stdin: bool,
    ) -> Result<std::process::Child, ClipboardError> {
        let mut command = Command::new(tool);
        command.args(args).stderr(Stdio::piped());
        if with_stdin {
            command.stdin(Stdio::piped()).stdout(Stdio::null());
        } else {
            command.stdout(Stdio::piped()).stdin(Stdio::null());
        }
        command.spawn().map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ClipboardError::MissingTool {
                    tool,
                    backend: self.display_name(),
                }
            } else {
                ClipboardError::LaunchFailed { tool, error }
            }
        })
    }

    /// Convert one process result into stdout bytes or one clipboard-specific error.
    fn ensure_success(
        self,
        tool: &'static str,
        output: std::process::Output,
        register: ClipboardRegister,
    ) -> Result<Vec<u8>, ClipboardError> {
        if output.status.success() {
            return Ok(output.stdout);
        }

        // Wayland primary-selection failures need one stable, explicit error so
        // `\"*` stays distinct instead of silently degrading to `\"+`.
        if self == Self::Wayland && register == ClipboardRegister::Primary {
            return Err(ClipboardError::UnsupportedPrimarySelection);
        }

        let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let details = if details.is_empty() {
            format!("exit status {}", output.status)
        } else {
            details
        };
        Err(ClipboardError::CommandFailed { tool, details })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Infer linewise pastes from clipboard text that ends with a newline.
    #[test]
    fn infer_payload_kind_treats_trailing_newline_as_linewise() {
        assert_eq!(
            ClipboardPayloadKind::infer("alpha\n"),
            ClipboardPayloadKind::Line
        );
    }

    /// Infer ordinary characterwise pastes when clipboard text lacks a trailing newline.
    #[test]
    fn infer_payload_kind_treats_plain_text_as_characterwise() {
        assert_eq!(
            ClipboardPayloadKind::infer("alpha\nbeta"),
            ClipboardPayloadKind::Character
        );
    }
}
