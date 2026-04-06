//! Shared Ordex cache-directory resolution.

use std::env;
use std::io;
use std::path::{Path, PathBuf};

/// Resolve one Ordex cache subdirectory from XDG cache state or `HOME`.
#[cfg(test)]
pub(crate) fn default_ordex_cache_subdir(name: &str) -> io::Result<PathBuf> {
    Ok(test_cache_root()?.join(name))
}

/// Resolve one Ordex cache subdirectory from XDG cache state or `HOME`.
#[cfg(not(test))]
pub(crate) fn default_ordex_cache_subdir(name: &str) -> io::Result<PathBuf> {
    resolve_ordex_cache_subdir(
        name,
        env::var_os("XDG_CACHE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .as_deref(),
        env::home_dir().as_deref(),
    )
}

/// Resolve one Ordex cache subdirectory from optional XDG and HOME base paths.
pub(crate) fn resolve_ordex_cache_subdir(
    name: &str,
    xdg_cache_home: Option<&Path>,
    home: Option<&Path>,
) -> io::Result<PathBuf> {
    if let Some(base) = xdg_cache_home {
        return Ok(base.join("ordex").join(name));
    }
    let Some(home) = home else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("HOME is not set; cannot resolve the {name} directory"),
        ));
    };
    Ok(home.join(".cache").join("ordex").join(name))
}

/// Return the per-test cache root used by unit tests.
#[cfg(test)]
fn test_cache_root() -> io::Result<PathBuf> {
    // Unit tests run in parallel inside one process, so the thread id keeps each
    // test on its own cache root and avoids cross-test swap/session collisions.
    let thread_id = format!("{:?}", std::thread::current().id());
    Ok(env::temp_dir().join(format!(
        "ordex_test_cache_{}_{}",
        std::process::id(),
        thread_id
    )))
}
