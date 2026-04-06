use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// Return the swap directory used by one PTY-backed ordex session.
pub fn swap_dir(cache_root: &Path) -> PathBuf {
    cache_root.join("ordex").join("swap")
}

/// Return the swap path corresponding to one source file path.
pub fn swap_path_for_path(cache_root: &Path, path: &Path) -> PathBuf {
    swap_dir(cache_root).join(format!("{}.swp", encode_path(path)))
}

/// Wait for one swap file to exist under `cache_root`.
pub fn wait_for_swap_file(cache_root: &Path, path: &Path) {
    let swap_path = swap_path_for_path(cache_root, path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if swap_path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("swap file did not appear at {}", swap_path.display());
}

/// Encode one absolute path using ordex's swap filename scheme.
fn encode_path(path: &Path) -> String {
    path.to_str()
        .expect("swap test paths must be UTF-8")
        .replace('%', "%%")
        .replace('/', "%2F")
}
