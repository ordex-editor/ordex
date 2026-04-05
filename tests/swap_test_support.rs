use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// Return the default swap directory used by the ordex binary in integration tests.
pub fn default_swap_dir() -> PathBuf {
    if let Some(xdg_cache_home) = env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(xdg_cache_home).join("ordex").join("swap");
    }
    let home = env::var_os("HOME").expect("HOME must be set for swap integration tests");
    PathBuf::from(home)
        .join(".cache")
        .join("ordex")
        .join("swap")
}

/// Return the swap path corresponding to one source file path.
pub fn swap_path_for_path(path: &Path) -> PathBuf {
    default_swap_dir().join(format!("{}.swp", encode_path(path)))
}

/// Remove one swap file before or after a test run.
pub fn cleanup_swap_for_path(path: &Path) {
    let _ = fs::remove_file(swap_path_for_path(path));
}

/// Wait for one swap file to exist.
pub fn wait_for_swap_file(path: &Path) {
    let swap_path = swap_path_for_path(path);
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
