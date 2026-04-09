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
