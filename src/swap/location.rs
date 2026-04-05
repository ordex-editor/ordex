//! Swap-directory resolution and swap filename encoding.

use std::env;
use std::io;
use std::path::{Path, PathBuf};

/// Resolve the default swap directory from XDG cache state or `HOME`.
pub(crate) fn default_swap_dir() -> io::Result<PathBuf> {
    let xdg_cache_home = env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    resolve_swap_dir(xdg_cache_home.as_deref(), env::home_dir().as_deref())
}

/// Return the swap path for `source_path` inside `swap_dir`.
pub(crate) fn swap_path_for(source_path: &Path, swap_dir: &Path) -> PathBuf {
    swap_dir.join(format!("{}.swp", encode_path(source_path)))
}

/// Encode one absolute path into a flat swap filename component.
pub(crate) fn encode_path(path: &Path) -> String {
    path.to_str()
        .expect("swap paths must be valid UTF-8")
        .replace('%', "%%")
        .replace('/', "%2F")
}

/// Resolve the swap directory from optional XDG and HOME base paths.
fn resolve_swap_dir(xdg_cache_home: Option<&Path>, home: Option<&Path>) -> io::Result<PathBuf> {
    if let Some(base) = xdg_cache_home {
        return Ok(base.join("ordex").join("swap"));
    }
    let Some(home) = home else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "HOME is not set; cannot resolve the swap directory",
        ));
    };
    Ok(home.join(".cache").join("ordex").join("swap"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_percent_before_slashes() {
        assert_eq!(
            encode_path(Path::new("/tmp/100%/demo.txt")),
            "%2Ftmp%2F100%%%2Fdemo.txt"
        );
    }

    #[test]
    fn builds_swap_path_with_expected_suffix() {
        let swap_path = swap_path_for(Path::new("/tmp/demo.txt"), Path::new("/cache/ordex/swap"));
        assert_eq!(
            swap_path,
            PathBuf::from("/cache/ordex/swap/%2Ftmp%2Fdemo.txt.swp")
        );
    }

    #[test]
    fn resolves_xdg_cache_swap_directory() {
        let dir = resolve_swap_dir(
            Some(Path::new("/tmp/cache")),
            Some(Path::new("/home/alice")),
        )
        .expect("resolve swap dir");
        assert_eq!(dir, PathBuf::from("/tmp/cache/ordex/swap"));
    }

    #[test]
    fn falls_back_to_home_cache_swap_directory() {
        let dir = resolve_swap_dir(None, Some(Path::new("/home/alice"))).expect("resolve swap dir");
        assert_eq!(dir, PathBuf::from("/home/alice/.cache/ordex/swap"));
    }

    #[test]
    fn requires_some_cache_base_directory() {
        let error = resolve_swap_dir(None, None).expect_err("reject missing cache roots");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
