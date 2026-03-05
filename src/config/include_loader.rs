//! Include-file path resolution and file loading helpers.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Read a configuration file as UTF-8 text.
pub(crate) fn read_config_file(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Resolve an include path relative to the main config file location.
pub(crate) fn resolve_include_path(base_path: &Path, include_path: &str) -> PathBuf {
    let include = PathBuf::from(include_path);
    if include.is_absolute() {
        return include;
    }
    base_path
        .parent()
        .map(|parent| parent.join(&include))
        .unwrap_or(include)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_path_from_base_parent() {
        let base = Path::new("/tmp/a/main.cfg");
        assert_eq!(
            resolve_include_path(base, "extra.cfg"),
            PathBuf::from("/tmp/a/extra.cfg")
        );
    }
}
