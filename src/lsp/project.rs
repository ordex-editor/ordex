//! Rust workspace discovery helpers for LSP session reuse.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Supported project-root marker kinds for Rust workspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProjectKind {
    CargoWorkspace,
    RustProjectJson,
}

/// Canonical Rust project context used to key rust-analyzer sessions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProjectWorkspace {
    pub(crate) root_path: PathBuf,
    pub(crate) kind: ProjectKind,
    pub(crate) manifest_path: PathBuf,
}

/// Failure returned when a file cannot be mapped to a supported Rust workspace.
#[derive(Debug)]
pub(crate) enum WorkspaceError {
    UnsupportedFileType(PathBuf),
    UnsupportedProject(PathBuf),
    CurrentDirectory(io::Error),
    Canonicalize {
        path: PathBuf,
        error: io::Error,
    },
    CargoMetadata {
        manifest_path: PathBuf,
        error: String,
    },
}

impl fmt::Display for WorkspaceError {
    /// Format a user-facing explanation for one workspace discovery failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFileType(path) => {
                write!(
                    f,
                    "\"{}\" is not a supported Rust source file",
                    path.display()
                )
            }
            Self::UnsupportedProject(path) => write!(
                f,
                "\"{}\" is not inside a supported Cargo workspace or rust-project.json root",
                path.display()
            ),
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

/// Resolve one Rust file path into its canonical reusable project workspace.
pub(crate) fn detect_workspace_for_file(path: &Path) -> Result<ProjectWorkspace, WorkspaceError> {
    if path.extension().and_then(|value| value.to_str()) != Some("rs") {
        return Err(WorkspaceError::UnsupportedFileType(path.to_path_buf()));
    }
    let canonical_path = canonicalize_path(path)?;
    let start_dir = canonical_path
        .parent()
        .ok_or_else(|| WorkspaceError::UnsupportedProject(canonical_path.clone()))?;
    detect_workspace_from_dir(start_dir).ok_or(WorkspaceError::UnsupportedProject(canonical_path))
}

/// Canonicalize one path, resolving relative inputs against the current directory.
fn canonicalize_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(WorkspaceError::CurrentDirectory)?
            .join(path)
    };
    absolute
        .canonicalize()
        .map_err(|error| WorkspaceError::Canonicalize {
            path: absolute,
            error,
        })
}

/// Walk upward from one directory until a supported project root is found.
fn detect_workspace_from_dir(start_dir: &Path) -> Option<ProjectWorkspace> {
    for ancestor in start_dir.ancestors() {
        let rust_project = ancestor.join("rust-project.json");
        if rust_project.is_file() {
            let root_path = ancestor.canonicalize().ok()?;
            let manifest_path = rust_project.canonicalize().unwrap_or(rust_project);
            return Some(ProjectWorkspace {
                root_path,
                kind: ProjectKind::RustProjectJson,
                manifest_path,
            });
        }

        let cargo_toml = ancestor.join("Cargo.toml");
        if cargo_toml.is_file() {
            return resolve_cargo_workspace(&cargo_toml).ok();
        }
    }
    None
}

/// Resolve the actual Cargo workspace root for one manifest path.
fn resolve_cargo_workspace(manifest_path: &Path) -> Result<ProjectWorkspace, WorkspaceError> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .output()
        .map_err(|error| WorkspaceError::CargoMetadata {
            manifest_path: manifest_path.to_path_buf(),
            error: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(WorkspaceError::CargoMetadata {
            manifest_path: manifest_path.to_path_buf(),
            error: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let parsed = json::parse(&String::from_utf8_lossy(&output.stdout)).map_err(|error| {
        WorkspaceError::CargoMetadata {
            manifest_path: manifest_path.to_path_buf(),
            error: error.to_string(),
        }
    })?;
    let workspace_root =
        parsed["workspace_root"]
            .as_str()
            .ok_or_else(|| WorkspaceError::CargoMetadata {
                manifest_path: manifest_path.to_path_buf(),
                error: "missing workspace_root in cargo metadata output".to_string(),
            })?;
    let root_path = PathBuf::from(workspace_root)
        .canonicalize()
        .map_err(|error| WorkspaceError::CargoMetadata {
            manifest_path: manifest_path.to_path_buf(),
            error: error.to_string(),
        })?;
    Ok(ProjectWorkspace {
        root_path,
        kind: ProjectKind::CargoWorkspace,
        manifest_path: manifest_path
            .canonicalize()
            .unwrap_or_else(|_| manifest_path.to_path_buf()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempTree;

    /// Write one Cargo workspace tree used by the workspace-detection tests.
    fn write_workspace(tree: &TempTree) {
        tree.write_file(
            "Cargo.toml",
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo manifest");
        tree.write_file("src/main.rs", "fn main() {}\n")
            .expect("write Rust source");
    }

    #[test]
    fn test_detect_workspace_for_cargo_project() {
        let tree = TempTree::new().expect("temp tree");
        write_workspace(&tree);

        let workspace =
            detect_workspace_for_file(&tree.path().join("src/main.rs")).expect("workspace");

        assert_eq!(workspace.kind, ProjectKind::CargoWorkspace);
        assert_eq!(
            workspace.root_path,
            tree.path().canonicalize().expect("canonical root")
        );
    }

    #[test]
    fn test_detect_workspace_for_rust_project_json() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("rust-project.json", "{}\n")
            .expect("write rust-project.json");
        tree.write_file("nested/src/lib.rs", "pub fn answer() -> i32 { 42 }\n")
            .expect("write Rust source");

        let workspace =
            detect_workspace_for_file(&tree.path().join("nested/src/lib.rs")).expect("workspace");

        assert_eq!(workspace.kind, ProjectKind::RustProjectJson);
        assert_eq!(
            workspace.root_path,
            tree.path().canonicalize().expect("canonical root")
        );
    }

    #[test]
    fn test_detect_workspace_rejects_non_rust_file() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/main.txt", "text\n")
            .expect("write text file");

        let error =
            detect_workspace_for_file(&tree.path().join("src/main.txt")).expect_err("error");

        assert!(matches!(error, WorkspaceError::UnsupportedFileType(_)));
    }

    #[test]
    fn test_detect_workspace_rejects_file_without_workspace() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file("src/lib.rs", "pub fn lonely() {}\n")
            .expect("write Rust source");

        let error = detect_workspace_for_file(&tree.path().join("src/lib.rs")).expect_err("error");

        assert!(matches!(error, WorkspaceError::UnsupportedProject(_)));
    }
}
