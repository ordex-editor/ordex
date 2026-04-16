//! Project-root detection shared by built-in language servers.

pub(crate) mod rust;

use super::server::{
    LspServerDescriptor, ProjectDetection, language_for_path, supported_project_description,
};
use std::fmt;
use std::path::{Path, PathBuf};

/// Marker kind used to describe how one reusable LSP project root was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProjectRootKind {
    CargoWorkspace,
    RustProjectJson,
    MarkerFile(&'static str),
    FileDirectory,
}

/// Canonical project context used to key one reusable language-server session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProjectWorkspace {
    pub(crate) root_path: PathBuf,
    pub(crate) kind: ProjectRootKind,
    pub(crate) marker_path: PathBuf,
}

/// Failure returned when a file cannot be mapped to one reusable LSP project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceError {
    UnsupportedFileType(PathBuf),
    UnsupportedProject {
        path: PathBuf,
        required_root_description: String,
    },
    CurrentDirectory(String),
    Canonicalize {
        path: PathBuf,
        error: String,
    },
    CargoMetadata {
        manifest_path: PathBuf,
        error: String,
    },
}

impl WorkspaceError {
    /// Build one unsupported-project failure with a caller-supplied explanation.
    pub(crate) fn unsupported_project(
        path: PathBuf,
        required_root_description: impl Into<String>,
    ) -> Self {
        Self::UnsupportedProject {
            path,
            required_root_description: required_root_description.into(),
        }
    }
}

impl fmt::Display for WorkspaceError {
    /// Format a user-facing explanation for one project-detection failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFileType(path) => {
                write!(
                    f,
                    "\"{}\" is not a supported file for built-in LSP",
                    path.display()
                )
            }
            Self::UnsupportedProject {
                path,
                required_root_description,
            } => {
                write!(
                    f,
                    "\"{}\" is not inside {required_root_description}",
                    path.display()
                )
            }
            Self::CurrentDirectory(error) => {
                write!(f, "failed to read the current directory: {error}")
            }
            Self::Canonicalize { path, error } => {
                write!(f, "failed to resolve \"{}\": {error}", path.display())
            }
            Self::CargoMetadata {
                manifest_path,
                error,
            } => write!(
                f,
                "failed to inspect Cargo workspace for \"{}\": {}",
                manifest_path.display(),
                error
            ),
        }
    }
}

impl std::error::Error for WorkspaceError {}

/// Resolve one file path into its canonical reusable project workspace for `server`.
pub(crate) fn detect_workspace_for_server(
    path: &Path,
    server: &LspServerDescriptor,
) -> Result<ProjectWorkspace, WorkspaceError> {
    let language = language_for_path(path)
        .ok_or_else(|| WorkspaceError::UnsupportedFileType(path.to_path_buf()))?;
    if !server.supports_language(language) {
        return Err(WorkspaceError::UnsupportedFileType(path.to_path_buf()));
    }
    let canonical_path = canonicalize_path(path)?;
    let start_dir = canonical_path.parent().ok_or_else(|| {
        WorkspaceError::unsupported_project(
            canonical_path.clone(),
            supported_project_description(language),
        )
    })?;

    // Root detection is server-specific so Rust can keep Cargo semantics while
    // Python and C-family servers follow their own marker or fallback rules.
    let workspace = match server.project_detection() {
        ProjectDetection::RustWorkspace => rust::detect_workspace_from_dir(start_dir)?,
        ProjectDetection::MarkerBased {
            markers,
            fallback_to_file_directory,
        } => detect_marker_workspace(start_dir, markers, fallback_to_file_directory),
    };
    workspace.ok_or_else(|| {
        WorkspaceError::unsupported_project(canonical_path, supported_project_description(language))
    })
}

/// Canonicalize one path, resolving relative inputs against the current directory.
fn canonicalize_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| WorkspaceError::CurrentDirectory(error.to_string()))?
            .join(path)
    };
    absolute
        .canonicalize()
        .map_err(|error| WorkspaceError::Canonicalize {
            path: absolute,
            error: error.to_string(),
        })
}

/// Walk upward until one marker-based project root is found.
fn detect_marker_workspace(
    start_dir: &Path,
    markers: &'static [&'static str],
    fallback_to_file_directory: bool,
) -> Option<ProjectWorkspace> {
    // Marker-based servers follow the editor conventions we surveyed: walk up the
    // ancestor chain and stop at the first root marker that should own the file.
    for ancestor in start_dir.ancestors() {
        for marker in markers {
            let marker_path = ancestor.join(marker);
            if marker_path.is_file() || marker_path.is_dir() {
                return Some(ProjectWorkspace {
                    root_path: ancestor.canonicalize().ok()?,
                    kind: ProjectRootKind::MarkerFile(marker),
                    marker_path: marker_path.canonicalize().unwrap_or(marker_path),
                });
            }
        }
    }
    if fallback_to_file_directory {
        let root_path = start_dir.canonicalize().ok()?;
        return Some(ProjectWorkspace {
            root_path: root_path.clone(),
            kind: ProjectRootKind::FileDirectory,
            marker_path: root_path,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::server::{CLANGD, PYLSP, RUFF, TY};
    use crate::syntax::profile::LanguageId;
    use test_utils::TempTree;

    /// Write one file into a temporary tree and return its path.
    fn write_source(tree: &TempTree, relative: &str) -> PathBuf {
        tree.write_file(relative, "pass\n").expect("write source");
        tree.path().join(relative)
    }

    /// Return one stable fixture language for marker-based test trees.
    fn expected_language(path: &Path) -> crate::syntax::profile::LanguageId {
        language_for_path(path).expect("test path should map to one built-in language")
    }

    /// Verify Rust project detection resolves Cargo workspaces.
    #[test]
    fn test_detect_workspace_for_rust_cargo_project() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "Cargo.toml",
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo manifest");
        let path = write_source(&tree, "src/main.rs");

        let workspace = detect_workspace_for_server(&path, &crate::lsp::server::RUST_ANALYZER)
            .expect("workspace");

        assert_eq!(workspace.kind, ProjectRootKind::CargoWorkspace);
    }

    /// Verify Python marker-based servers detect their configured roots.
    #[test]
    fn test_detect_workspace_for_python_marker_servers() {
        let cases = [(&TY, "ty.toml"), (&RUFF, "ruff.toml"), (&PYLSP, "Pipfile")];
        for (server, marker) in cases {
            let tree = TempTree::new().expect("temp tree");
            tree.write_file(marker, "root\n").expect("write marker");
            let path = write_source(&tree, "pkg/main.py");

            let workspace = detect_workspace_for_server(&path, server).expect("workspace");

            assert_eq!(expected_language(&path), LanguageId::Python);
            assert_eq!(
                workspace.root_path,
                tree.path().canonicalize().expect("root")
            );
        }
    }

    /// Verify clangd uses marker-based root detection for C-family files.
    #[test]
    fn test_detect_workspace_for_clangd_marker_project() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("compile_commands.json", "[]\n")
            .expect("write compile_commands");
        let path = write_source(&tree, "src/main.cpp");

        let workspace = detect_workspace_for_server(&path, &CLANGD).expect("workspace");

        assert_eq!(
            workspace.kind,
            ProjectRootKind::MarkerFile("compile_commands.json")
        );
    }

    /// Verify clangd falls back to the opened file directory when no marker exists.
    #[test]
    fn test_detect_workspace_for_clangd_without_markers() {
        let tree = TempTree::new().expect("temp tree");
        let path = write_source(&tree, "src/main.cpp");

        let workspace = detect_workspace_for_server(&path, &CLANGD).expect("workspace");

        assert_eq!(workspace.kind, ProjectRootKind::FileDirectory);
        assert_eq!(
            workspace.root_path,
            tree.path()
                .join("src")
                .canonicalize()
                .expect("source directory")
        );
    }

    /// Verify unsupported LSP file types fail before project-root detection starts.
    #[test]
    fn test_detect_workspace_rejects_unsupported_file_type() {
        let tree = TempTree::new().expect("temp tree");
        let path = write_source(&tree, "notes.txt");

        let error = detect_workspace_for_server(&path, &TY).expect_err("unsupported file");

        assert!(matches!(error, WorkspaceError::UnsupportedFileType(_)));
    }

    /// Verify standalone Python files fall back to their containing directory.
    #[test]
    fn test_detect_workspace_for_python_without_markers() {
        let tree = TempTree::new().expect("temp tree");
        let path = write_source(&tree, "pkg/main.py");

        let workspace = detect_workspace_for_server(&path, &TY).expect("workspace");

        assert_eq!(workspace.kind, ProjectRootKind::FileDirectory);
        assert_eq!(
            workspace.root_path,
            tree.path()
                .join("pkg")
                .canonicalize()
                .expect("source directory")
        );
    }
}
