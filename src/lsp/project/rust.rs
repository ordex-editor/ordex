//! Rust-specific workspace detection helpers.

use super::{ProjectRootKind, ProjectWorkspace, WorkspaceError};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Walk upward from one directory until a supported Rust project root is found.
pub(super) fn detect_workspace_from_dir(
    start_dir: &Path,
) -> Result<Option<ProjectWorkspace>, WorkspaceError> {
    for ancestor in start_dir.ancestors() {
        let rust_project = ancestor.join("rust-project.json");
        if rust_project.is_file() {
            let root_path =
                ancestor
                    .canonicalize()
                    .map_err(|error| WorkspaceError::Canonicalize {
                        path: ancestor.to_path_buf(),
                        error: error.to_string(),
                    })?;
            let marker_path = rust_project.canonicalize().unwrap_or(rust_project);
            return Ok(Some(ProjectWorkspace {
                root_path,
                kind: ProjectRootKind::RustProjectJson,
                marker_path,
            }));
        }

        let cargo_toml = ancestor.join("Cargo.toml");
        if cargo_toml.is_file() {
            return resolve_cargo_workspace(&cargo_toml).map(Some);
        }
    }
    Ok(None)
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
        kind: ProjectRootKind::CargoWorkspace,
        marker_path: manifest_path
            .canonicalize()
            .unwrap_or_else(|_| manifest_path.to_path_buf()),
    })
}
