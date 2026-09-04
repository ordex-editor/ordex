//! Rust-specific workspace detection helpers.

use super::{ProjectRootKind, ProjectWorkspace, WorkspaceError};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// One remembered Cargo resolution together with the manifest state behind it.
struct RememberedWorkspace {
    /// Manifest modification time observed when this entry was resolved.
    manifest_modified: Option<SystemTime>,
    /// Workspace Cargo reported for that manifest.
    workspace: ProjectWorkspace,
}

/// Return the store of Cargo resolutions already paid for in this process.
///
/// Every navigation, completion, and document sync resolves the edited file's
/// project again, so spawning Cargo each time would cost far more than the
/// request it precedes.
fn remembered_workspaces() -> &'static Mutex<HashMap<PathBuf, RememberedWorkspace>> {
    static WORKSPACES: OnceLock<Mutex<HashMap<PathBuf, RememberedWorkspace>>> = OnceLock::new();
    WORKSPACES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Return the modification time `manifest_path` reports, when it has one.
fn manifest_modified(manifest_path: &Path) -> Option<SystemTime> {
    fs::metadata(manifest_path).ok()?.modified().ok()
}

/// Walk upward from one directory until a supported Rust project root is found.
pub(super) fn detect_workspace_from_dir(
    start_dir: &Path,
) -> Result<Option<ProjectWorkspace>, WorkspaceError> {
    let mut package_manifest = None;
    for ancestor in start_dir.ancestors() {
        // `rust-project.json` is an explicit rust-analyzer project description,
        // so it owns the workspace immediately without any Cargo probing.
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

        // Keep the nearest Cargo manifest as a fallback root, then let Cargo
        // metadata refine it to the real workspace root when Cargo is available.
        let cargo_toml = ancestor.join("Cargo.toml");
        if package_manifest.is_none() && cargo_toml.is_file() {
            package_manifest = Some((ancestor.to_path_buf(), cargo_toml));
        }
    }
    if let Some((root_dir, manifest_path)) = package_manifest {
        return resolve_cargo_workspace(&root_dir, &manifest_path).map(Some);
    }
    Ok(None)
}

/// Return whether one command name resolves from the current process `PATH`.
///
/// Returns `true` when `program` resolves to an executable path, and `false`
/// when the current process environment does not expose that command.
fn command_available(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Resolve one Cargo-backed root, reusing the previous answer for one manifest.
///
/// A manifest keeps the same workspace until it is edited, so the resolution is
/// remembered and only recomputed once its modification time moves. Failures
/// stay unremembered so a transient Cargo error does not stick for the session.
fn resolve_cargo_workspace(
    root_dir: &Path,
    manifest_path: &Path,
) -> Result<ProjectWorkspace, WorkspaceError> {
    let modified = manifest_modified(manifest_path);
    {
        let remembered = remembered_workspaces()
            .lock()
            .expect("lock remembered workspaces");
        if let Some(entry) = remembered.get(manifest_path)
            && entry.manifest_modified == modified
        {
            return Ok(entry.workspace.clone());
        }
    }
    let workspace = query_cargo_workspace(root_dir, manifest_path)?;
    remembered_workspaces()
        .lock()
        .expect("lock remembered workspaces")
        .insert(
            manifest_path.to_path_buf(),
            RememberedWorkspace {
                manifest_modified: modified,
                workspace: workspace.clone(),
            },
        );
    Ok(workspace)
}

/// Ask Cargo which workspace owns `manifest_path`, falling back when it cannot.
///
/// Callers reach this from the editor's own thread, so a Cargo invocation that
/// does not return promptly stops the editor until it does.
#[allow(
    clippy::disallowed_methods,
    reason = "known unbounded wait on the editor thread, tracked as its own change"
)]
fn query_cargo_workspace(
    root_dir: &Path,
    manifest_path: &Path,
) -> Result<ProjectWorkspace, WorkspaceError> {
    if !command_available("cargo") {
        return build_cargo_workspace(root_dir, manifest_path);
    }
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
    // Cargo can resolve the true workspace root for member crates, but keep the
    // manifest directory as a production-style fallback if metadata is incomplete.
    let Some(workspace_root) = parsed["workspace_root"].as_str() else {
        return build_cargo_workspace(root_dir, manifest_path);
    };
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

/// Build one Cargo-backed project root directly from the discovered manifest path.
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
