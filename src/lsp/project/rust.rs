//! Rust-specific workspace detection helpers.

use super::{ProjectRootKind, ProjectWorkspace, WorkspaceError};
use std::fs;
use std::path::Path;

/// Walk upward from one directory until a supported Rust project root is found.
pub(super) fn detect_workspace_from_dir(
    start_dir: &Path,
) -> Result<Option<ProjectWorkspace>, WorkspaceError> {
    let mut package_manifest = None;
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
            let manifest = read_cargo_manifest(&cargo_toml)?;
            if manifest.has_workspace_table {
                return build_cargo_workspace(ancestor, &cargo_toml).map(Some);
            }
            if package_manifest.is_none() {
                package_manifest = Some((ancestor.to_path_buf(), cargo_toml));
            }
        }
    }
    if let Some((root_dir, manifest_path)) = package_manifest {
        return build_cargo_workspace(&root_dir, &manifest_path).map(Some);
    }
    Ok(None)
}

/// Parsed Cargo-manifest facts used by Rust workspace detection.
struct CargoManifestInfo {
    has_workspace_table: bool,
}

/// Read one Cargo manifest and extract the workspace facts Ordex needs.
fn read_cargo_manifest(manifest_path: &Path) -> Result<CargoManifestInfo, WorkspaceError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|error| WorkspaceError::CargoMetadata {
            manifest_path: manifest_path.to_path_buf(),
            error: error.to_string(),
        })?;
    Ok(CargoManifestInfo {
        has_workspace_table: manifest_has_workspace_table(&contents),
    })
}

/// Return whether one Cargo manifest declares a top-level `[workspace]` table.
fn manifest_has_workspace_table(contents: &str) -> bool {
    contents.lines().any(|line| {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        trimmed == "[workspace]"
    })
}

/// Build one Cargo-backed project root from one discovered manifest directory.
fn build_cargo_workspace(
    root_dir: &Path,
    manifest_path: &Path,
) -> Result<ProjectWorkspace, WorkspaceError> {
    let root_path = root_dir
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
