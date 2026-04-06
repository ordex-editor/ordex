//! Named project-session persistence.

use crate::cache_dirs;
use crate::cursor::Cursor;
use crate::toml_like_parser::{ParsedDocument, ParsedSection, ParsedValue, parse_reader};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Component, Path, PathBuf};

const SESSION_SECTION: &str = "session";
const BUFFER_SECTION_PREFIX: &str = "buffer.";
const SESSION_FILE_SUFFIX: &str = ".toml";

/// One buffer entry stored inside a project session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionBuffer {
    pub(crate) path: PathBuf,
    pub(crate) cursor: Cursor,
}

/// Full persisted state for one named project session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectSession {
    pub(crate) working_directory: PathBuf,
    pub(crate) active_buffer: usize,
    pub(crate) buffers: Vec<SessionBuffer>,
}

/// Result of loading one session file with recoverable warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionLoadOutcome {
    pub(crate) session: ProjectSession,
    pub(crate) warnings: Vec<String>,
}

/// Resolve the default session-storage directory from XDG cache locations or `HOME`.
pub(crate) fn default_sessions_dir() -> io::Result<PathBuf> {
    cache_dirs::default_ordex_cache_subdir("sessions")
}

/// Save one project session under its validated user-visible name.
pub(crate) fn save_project_session(name: &str, session: &ProjectSession) -> io::Result<PathBuf> {
    let sessions_dir = default_sessions_dir()?;
    save_project_session_in_dir(name, session, &sessions_dir)
}

/// Save one project session into one caller-provided sessions directory.
pub(crate) fn save_project_session_in_dir(
    name: &str,
    session: &ProjectSession,
    sessions_dir: &Path,
) -> io::Result<PathBuf> {
    let path = session_file_path_in_dir(name, sessions_dir)?;
    let Some(parent) = path.parent() else {
        return Err(io::Error::other(
            "session path is missing its parent directory",
        ));
    };
    fs::create_dir_all(parent)?;
    let mut file = File::create(&path)?;
    file.write_all(format_session_document(session).as_bytes())?;
    Ok(path)
}

/// Load one named project session and collect recoverable warnings.
pub(crate) fn load_project_session(name: &str) -> io::Result<SessionLoadOutcome> {
    let sessions_dir = default_sessions_dir()?;
    load_project_session_from_dir(name, &sessions_dir)
}

/// Load one named project session from one caller-provided sessions directory.
pub(crate) fn load_project_session_from_dir(
    name: &str,
    sessions_dir: &Path,
) -> io::Result<SessionLoadOutcome> {
    let path = session_file_path_in_dir(name, sessions_dir)?;
    let file = File::open(&path)?;
    let reader = BufReader::new(file);
    let document = parse_reader(&path, reader)?;
    validate_session_document(&document)
}

/// Delete one named project session from the default sessions directory.
pub(crate) fn delete_project_session(name: &str) -> io::Result<()> {
    let sessions_dir = default_sessions_dir()?;
    delete_project_session_in_dir(name, &sessions_dir)
}

/// Delete one named project session from one caller-provided sessions directory.
pub(crate) fn delete_project_session_in_dir(name: &str, sessions_dir: &Path) -> io::Result<()> {
    let path = session_file_path_in_dir(name, sessions_dir)?;
    fs::remove_file(path)
}

/// Convert one absolute or relative buffer path into a session-stable representation.
pub(crate) fn normalize_session_buffer_path(path: &Path, working_directory: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    if path.is_absolute()
        && let Ok(relative) = path.strip_prefix(working_directory)
    {
        return relative.to_path_buf();
    }
    path.to_path_buf()
}

/// Return a user-facing summary message for one session load result.
pub(crate) fn load_status_message(name: &str, warning_count: usize) -> String {
    match warning_count {
        0 => format!("Session \"{name}\" opened"),
        1 => format!("Session \"{name}\" opened with 1 warning"),
        count => format!("Session \"{name}\" opened with {count} warnings"),
    }
}

/// Build one storage path inside one caller-provided sessions directory.
fn session_file_path_in_dir(name: &str, sessions_dir: &Path) -> io::Result<PathBuf> {
    Ok(sessions_dir.join(session_file_name(name)?))
}

/// Resolve the session directory from XDG cache locations or one home directory.
#[cfg(test)]
fn resolve_sessions_dir(xdg_cache_home: Option<&Path>, home: Option<&Path>) -> io::Result<PathBuf> {
    cache_dirs::resolve_ordex_cache_subdir("sessions", xdg_cache_home, home)
}

/// Return the validated single-path-component session name.
fn validated_session_name(name: &str) -> io::Result<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session name cannot be empty",
        ));
    }
    if !matches!(
        Path::new(trimmed).components().next(),
        Some(Component::Normal(_))
    ) || Path::new(trimmed).components().count() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session name must not contain path separators",
        ));
    }
    Ok(trimmed)
}

/// Return the on-disk file name used for one session command argument.
fn session_file_name(name: &str) -> io::Result<String> {
    let name = validated_session_name(name)?;
    if name.ends_with(SESSION_FILE_SUFFIX) {
        return Ok(name.to_string());
    }
    Ok(format!("{name}{SESSION_FILE_SUFFIX}"))
}

/// Serialize one session into the TOML-like on-disk representation.
fn format_session_document(session: &ProjectSession) -> String {
    let mut document = String::new();
    document.push_str("[session]\n");
    document.push_str(&format!(
        "working_directory = \"{}\"\n",
        escape_string(&session.working_directory.display().to_string())
    ));

    for (index, buffer) in session.buffers.iter().enumerate() {
        document.push('\n');
        document.push_str(&format!("[buffer.{index}]\n"));
        document.push_str(&format!(
            "path = \"{}\"\n",
            escape_string(&buffer.path.display().to_string())
        ));
        if index == session.active_buffer {
            document.push_str("active = true\n");
        }
        document.push_str(&format!("line = {}\n", buffer.cursor.line()));
        document.push_str(&format!("column = {}\n", buffer.cursor.column()));
    }

    document
}

/// Escape one string value for the TOML-like quoted-string format.
fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Validate one parsed session document and keep recoverable warnings.
fn validate_session_document(document: &ParsedDocument) -> io::Result<SessionLoadOutcome> {
    let mut warnings = collect_parser_warnings(document);
    let mut working_directory = None;
    let mut buffers = BTreeMap::new();

    // Session validation keeps section-specific logic local so malformed buffer
    // entries can be skipped without dropping the entire document.
    for section in &document.sections {
        if section.name == "root" {
            collect_root_warnings(section, &mut warnings);
        } else if section.name == SESSION_SECTION {
            apply_session_section(section, &mut working_directory, &mut warnings);
        } else if let Some(index) = parse_buffer_section_index(&section.name) {
            buffers.insert(index, parse_buffer_section(section, &mut warnings));
        } else {
            warnings.push(format!("Unknown section `{}` ignored", section.name));
        }
    }

    let working_directory = match working_directory {
        Some(path) if !path.as_os_str().is_empty() => path,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Session is missing `session.working_directory`",
            ));
        }
    };
    let finalized = finalize_buffers(buffers, &mut warnings);

    Ok(SessionLoadOutcome {
        session: ProjectSession {
            working_directory,
            active_buffer: finalized.active_buffer,
            buffers: finalized.buffers,
        },
        warnings,
    })
}

/// Turn parser diagnostics into recoverable session-load warnings.
fn collect_parser_warnings(document: &ParsedDocument) -> Vec<String> {
    document
        .diagnostics
        .iter()
        .map(|diagnostic| format!("Line {}: {}", diagnostic.line, diagnostic.message))
        .collect()
}

/// Collect warnings for unknown top-level assignments.
fn collect_root_warnings(section: &ParsedSection, warnings: &mut Vec<String>) {
    for item in &section.items {
        warnings.push(format!("Unknown top-level key `{}` ignored", item.key));
    }
}

/// Apply the `[session]` section onto the accumulating session state.
fn apply_session_section(
    section: &ParsedSection,
    working_directory: &mut Option<PathBuf>,
    warnings: &mut Vec<String>,
) {
    for item in &section.items {
        match item.key.as_str() {
            "working_directory" => match &item.value {
                ParsedValue::String(value) => *working_directory = Some(PathBuf::from(value)),
                _ => warnings.push("`session.working_directory` must be a string".to_string()),
            },
            _ => warnings.push(format!("Unknown key `session.{}` ignored", item.key)),
        }
    }
}

/// Return the numeric index for one `[buffer.N]` section name.
fn parse_buffer_section_index(name: &str) -> Option<usize> {
    name.strip_prefix(BUFFER_SECTION_PREFIX)?.parse().ok()
}

/// Parse one `[buffer.N]` section into a draft buffer snapshot.
fn parse_buffer_section(section: &ParsedSection, warnings: &mut Vec<String>) -> SessionBufferDraft {
    let mut draft = SessionBufferDraft::default();

    for item in &section.items {
        match item.key.as_str() {
            "path" => match &item.value {
                ParsedValue::String(value) => draft.path = Some(PathBuf::from(value)),
                _ => warnings.push(format!("`{}.{}` must be a string", section.name, item.key)),
            },
            "line" => match parse_non_negative_integer(
                item.value.clone(),
                &format!("{}.line", section.name),
            ) {
                Ok(line) => draft.line = Some(line),
                Err(message) => warnings.push(message),
            },
            "column" => {
                match parse_non_negative_integer(
                    item.value.clone(),
                    &format!("{}.column", section.name),
                ) {
                    Ok(column) => draft.column = Some(column),
                    Err(message) => warnings.push(message),
                }
            }
            "active" => match item.value.clone() {
                ParsedValue::Boolean(active) => draft.active = Some(active),
                _ => warnings.push(format!("`{}.{}` must be a boolean", section.name, item.key)),
            },
            _ => warnings.push(format!(
                "Unknown key `{}.{}` ignored",
                section.name, item.key
            )),
        }
    }

    draft
}

/// Parse one non-negative integer value for session validation.
fn parse_non_negative_integer(value: ParsedValue, key: &str) -> Result<usize, String> {
    match value {
        ParsedValue::Integer(number) if number >= 0 => Ok(number as usize),
        ParsedValue::Integer(_) => Err(format!("`{key}` must be non-negative")),
        _ => Err(format!("`{key}` must be an integer")),
    }
}

/// Finalize parsed buffer drafts into ordered session buffers.
fn finalize_buffers(
    drafts: BTreeMap<usize, SessionBufferDraft>,
    warnings: &mut Vec<String>,
) -> FinalizedBuffers {
    let mut buffers = Vec::new();
    let mut active_buffer = None;

    // Finalization runs after every section has been parsed so we can skip only
    // the malformed buffer entries and still keep the rest of the session.
    for (index, draft) in drafts {
        let Some(path) = draft.path else {
            warnings.push(format!("buffer.{index} is missing `path` and was skipped"));
            continue;
        };
        let line = draft.line.unwrap_or_else(|| {
            warnings.push(format!("buffer.{index}.line missing; using 0"));
            0
        });
        let column = draft.column.unwrap_or_else(|| {
            warnings.push(format!("buffer.{index}.column missing; using 0"));
            0
        });
        if draft.active == Some(true) && active_buffer.replace(buffers.len()).is_some() {
            warnings.push(format!(
                "Multiple buffers are marked active; using buffer.{index}"
            ));
        }
        buffers.push(SessionBuffer {
            path,
            cursor: Cursor::new(line, column),
        });
    }

    if buffers.is_empty() {
        warnings.push("Session contains no valid buffers; opening an empty buffer".to_string());
        return FinalizedBuffers {
            buffers,
            active_buffer: 0,
        };
    }

    let active_buffer = active_buffer.unwrap_or_else(|| {
        warnings.push("No buffer is marked active; using the first buffer".to_string());
        0
    });
    FinalizedBuffers {
        buffers,
        active_buffer,
    }
}

/// One partially parsed buffer section before validation defaults are applied.
#[derive(Debug, Default)]
struct SessionBufferDraft {
    path: Option<PathBuf>,
    line: Option<usize>,
    column: Option<usize>,
    active: Option<bool>,
}

/// Ordered validated buffers plus the active-buffer index chosen during validation.
struct FinalizedBuffers {
    buffers: Vec<SessionBuffer>,
    active_buffer: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toml_like_parser::parse_str;

    /// Build one temp session directory rooted under the process temp dir.
    fn temp_home(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ordex_session_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn resolves_default_session_directory_from_home() {
        let dir = resolve_sessions_dir(None, Some(Path::new("/home/alice")))
            .expect("resolve session dir");
        assert_eq!(dir, PathBuf::from("/home/alice/.cache/ordex/sessions"));
    }

    #[test]
    fn resolves_default_session_directory_from_xdg_cache_home() {
        let dir = resolve_sessions_dir(
            Some(Path::new("/tmp/cache-home")),
            Some(Path::new("/home/alice")),
        )
        .expect("resolve session dir");
        assert_eq!(dir, PathBuf::from("/tmp/cache-home/ordex/sessions"));
    }

    #[test]
    fn rejects_session_names_with_path_separators() {
        let error = validated_session_name("nested/name").expect_err("reject nested name");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn normalizes_paths_relative_to_working_directory() {
        let working_directory = Path::new("/tmp/project");
        let path = Path::new("/tmp/project/src/main.rs");
        assert_eq!(
            normalize_session_buffer_path(path, working_directory),
            PathBuf::from("src/main.rs")
        );
    }

    #[test]
    fn formats_and_validates_session_document() {
        let session = ProjectSession {
            working_directory: PathBuf::from("/tmp/project"),
            active_buffer: 1,
            buffers: vec![
                SessionBuffer {
                    path: PathBuf::from("src/main.rs"),
                    cursor: Cursor::new(4, 2),
                },
                SessionBuffer {
                    path: PathBuf::new(),
                    cursor: Cursor::new(0, 0),
                },
            ],
        };
        let document = parse_str(Path::new("session"), &format_session_document(&session));
        let outcome = validate_session_document(&document).expect("validate session document");
        assert_eq!(outcome.session, session);
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn keeps_recoverable_warnings_while_loading() {
        let document = parse_str(
            Path::new("session"),
            r#"
[session]
working_directory = "/tmp/project"
extra = true

[buffer.0]
path = "src/main.rs"
active = true

[buffer.nope]
path = "ignored.txt"
"#,
        );
        let outcome = validate_session_document(&document).expect("validate session document");
        assert_eq!(outcome.session.buffers.len(), 1);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|warning| warning.contains("Unknown key `session.extra`"))
        );
    }

    #[test]
    fn saves_session_file_under_cache_directory() {
        let home = temp_home("save");
        let _ = fs::remove_dir_all(&home);
        let sessions_dir = resolve_sessions_dir(None, Some(&home)).expect("session dir");
        fs::create_dir_all(&sessions_dir).expect("create session dir");
        let result = save_project_session_in_dir(
            "demo",
            &ProjectSession {
                working_directory: PathBuf::from("/tmp/project"),
                active_buffer: 0,
                buffers: vec![SessionBuffer {
                    path: PathBuf::from("src/main.rs"),
                    cursor: Cursor::new(1, 2),
                }],
            },
            &sessions_dir,
        )
        .expect("save session");

        assert_eq!(result, sessions_dir.join("demo.toml"));
        let _ = fs::remove_dir_all(temp_home("save"));
    }

    #[test]
    fn deletes_session_file_from_directory() {
        let root = temp_home("delete");
        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create session dir");
        fs::write(sessions_dir.join("demo.toml"), "").expect("write session file");

        delete_project_session_in_dir("demo", &sessions_dir).expect("delete session");

        assert!(!sessions_dir.join("demo.toml").exists());
        let _ = fs::remove_dir_all(root);
    }
}
