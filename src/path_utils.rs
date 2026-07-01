//! Filesystem path helpers shared across editor and LSP surfaces.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Return `path` relative to the current directory when possible.
pub(crate) fn current_dir_relative_path(path: &Path) -> Cow<'_, Path> {
    if path.is_absolute()
        && let Ok(current_directory) = std::env::current_dir()
        && let Ok(relative) = path.strip_prefix(&current_directory)
    {
        return Cow::Owned(PathBuf::from(relative));
    }
    Cow::Borrowed(path)
}

/// Return one user-facing path label that prefers compact in-editor display.
pub(crate) fn display_path_for_ui(path: &Path) -> String {
    let relative_to_cwd = current_dir_relative_path(path);
    // Keep relative paths when possible because they are usually the shortest
    // and most useful labels in in-editor UI surfaces.
    if !relative_to_cwd.as_ref().is_absolute() {
        if relative_to_cwd.as_ref().as_os_str().is_empty() {
            return ".".to_string();
        }
        return relative_to_cwd.display().to_string();
    }
    let Some(home) = std::env::home_dir() else {
        return relative_to_cwd.display().to_string();
    };
    // Compact only the current user's home prefix while leaving other absolute
    // paths unchanged so non-home filesystem roots stay explicit.
    if relative_to_cwd.as_ref() == home {
        return "~".to_string();
    }
    if let Ok(relative_to_home) = relative_to_cwd.as_ref().strip_prefix(&home) {
        if relative_to_home.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!(
            "~{}{}",
            std::path::MAIN_SEPARATOR,
            relative_to_home.display()
        );
    }
    relative_to_cwd.display().to_string()
}

/// Expand a leading `~` or `~/` in `path` to the user's home directory.
///
/// Returns the expanded path when the input starts with `~` and the home
/// directory can be determined, or the original path unchanged otherwise.
pub(crate) fn expand_tilde(path: &str) -> Cow<'_, Path> {
    if !path.starts_with('~') {
        return Cow::Borrowed(Path::new(path));
    }
    let Some(home) = std::env::home_dir() else {
        return Cow::Borrowed(Path::new(path));
    };
    if path == "~" {
        return Cow::Owned(home);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Cow::Owned(home.join(rest));
    }
    Cow::Borrowed(Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::{CurrentDirectoryGuard, EnvVarGuard, TempTree, lock_process_environment};

    #[test]
    /// Verify display labels prefer paths relative to the current directory.
    fn test_display_path_for_ui_prefers_current_directory_relative_path() {
        let tree = TempTree::new().expect("create temp tree");
        let project = tree.path().join("project");
        std::fs::create_dir_all(project.join("src")).expect("create project tree");
        let lock = lock_process_environment();
        let _home_guard =
            EnvVarGuard::set(&lock, "HOME", tree.path().to_path_buf().into_os_string());
        let _cwd_guard = CurrentDirectoryGuard::change_to(&project);

        assert_eq!(
            display_path_for_ui(&project.join("src/main.rs")),
            "src/main.rs"
        );
    }

    #[test]
    /// Verify the current directory itself renders as a visible label.
    fn test_display_path_for_ui_formats_current_directory_as_dot() {
        let tree = TempTree::new().expect("create temp tree");
        let project = tree.path().join("project");
        std::fs::create_dir_all(&project).expect("create project tree");
        let lock = lock_process_environment();
        let _home_guard =
            EnvVarGuard::set(&lock, "HOME", tree.path().to_path_buf().into_os_string());
        let _cwd_guard = CurrentDirectoryGuard::change_to(&project);

        assert_eq!(display_path_for_ui(&project), ".");
    }

    #[test]
    /// Verify home-directory descendants compact to one `~/...` display label.
    fn test_display_path_for_ui_compacts_home_descendant_path() {
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home-user");
        std::fs::create_dir_all(home.join("workspace")).expect("create home tree");
        let lock = lock_process_environment();
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());

        assert_eq!(
            display_path_for_ui(&home.join("workspace/main.rs")),
            "~/workspace/main.rs"
        );
    }

    #[test]
    /// Verify the home directory itself collapses to one bare `~` label.
    fn test_display_path_for_ui_compacts_home_root_to_tilde() {
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home-user");
        std::fs::create_dir_all(&home).expect("create home directory");
        let lock = lock_process_environment();
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());

        assert_eq!(display_path_for_ui(&home), "~");
    }

    #[test]
    /// Verify non-home absolute paths stay absolute when no relative form exists.
    fn test_display_path_for_ui_keeps_non_home_absolute_path() {
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home-user");
        let outside = tree.path().join("outside/main.rs");
        std::fs::create_dir_all(home.join("workspace")).expect("create home workspace");
        std::fs::create_dir_all(outside.parent().expect("parent directory"))
            .expect("create outside directory");
        let lock = lock_process_environment();
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.into_os_string());

        assert_eq!(display_path_for_ui(&outside), outside.display().to_string());
    }

    #[test]
    /// Verify `~/rest` expands to the home directory joined with the remainder.
    fn test_expand_tilde_resolves_home_slash_prefix() {
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home-user");
        std::fs::create_dir_all(&home).expect("create home directory");
        let lock = lock_process_environment();
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());

        assert_eq!(
            expand_tilde("~/notes.txt"),
            Cow::<Path>::Owned(home.join("notes.txt"))
        );
    }

    #[test]
    /// Verify a bare `~` expands to the home directory itself.
    fn test_expand_tilde_resolves_bare_tilde_to_home() {
        let tree = TempTree::new().expect("create temp tree");
        let home = tree.path().join("home-user");
        std::fs::create_dir_all(&home).expect("create home directory");
        let lock = lock_process_environment();
        let _home_guard = EnvVarGuard::set(&lock, "HOME", home.clone().into_os_string());

        assert_eq!(expand_tilde("~"), Cow::<Path>::Owned(home.clone()));
    }

    #[test]
    /// Verify absolute paths pass through without allocation.
    fn test_expand_tilde_leaves_absolute_path_unchanged() {
        assert_eq!(
            expand_tilde("/etc/hosts"),
            Cow::Borrowed(Path::new("/etc/hosts"))
        );
    }

    #[test]
    /// Verify relative paths pass through without allocation.
    fn test_expand_tilde_leaves_relative_path_unchanged() {
        assert_eq!(
            expand_tilde("src/main.rs"),
            Cow::Borrowed(Path::new("src/main.rs"))
        );
    }

    #[test]
    /// Verify an empty string passes through without allocation.
    fn test_expand_tilde_leaves_empty_string_unchanged() {
        assert_eq!(expand_tilde(""), Cow::Borrowed(Path::new("")));
    }
}
