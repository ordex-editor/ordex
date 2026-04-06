//! Swap-directory resolution and swap filename encoding.

use crate::cache_dirs;
use std::io;
use std::path::{Path, PathBuf};

/// Resolve the default swap directory from XDG cache state or `HOME`.
pub(crate) fn default_swap_dir() -> io::Result<PathBuf> {
    cache_dirs::default_ordex_cache_subdir("swap")
}

/// Return the swap path for `source_path` inside `swap_dir`.
pub(crate) fn swap_path_for(source_path: &Path, swap_dir: &Path) -> io::Result<PathBuf> {
    Ok(swap_dir.join(format!("{}.swp", encode_path(source_path)?)))
}

/// Encode one absolute path into a flat swap filename component.
pub(crate) fn encode_path(path: &Path) -> io::Result<String> {
    let path = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "swap paths must be valid UTF-8",
        )
    })?;
    Ok(path.replace('%', "%%").replace('/', "%2F"))
}

/// Resolve the swap directory from optional XDG and HOME base paths.
#[cfg(test)]
fn resolve_swap_dir(xdg_cache_home: Option<&Path>, home: Option<&Path>) -> io::Result<PathBuf> {
    cache_dirs::resolve_ordex_cache_subdir("swap", xdg_cache_home, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_percent_before_slashes() {
        assert_eq!(
            encode_path(Path::new("/tmp/100%/demo.txt")).expect("encode path"),
            "%2Ftmp%2F100%%%2Fdemo.txt"
        );
    }

    #[test]
    fn builds_swap_path_with_expected_suffix() {
        let swap_path = swap_path_for(Path::new("/tmp/demo.txt"), Path::new("/cache/ordex/swap"))
            .expect("swap path");
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
