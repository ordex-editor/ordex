use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// Return the swap directory used by one PTY-backed ordex session.
pub fn swap_dir(cache_root: &Path) -> PathBuf {
    cache_root.join("ordex").join("swap")
}

/// Return the swap path corresponding to one source file path.
pub fn compute_swap_path(cache_root: &Path, path: &Path) -> PathBuf {
    swap_dir(cache_root).join(format!("{}.swp", encode_path(path)))
}

/// Wait for one swap file to exist under `cache_root`.
pub fn wait_for_swap_file(cache_root: &Path, path: &Path) {
    let swap_path = compute_swap_path(cache_root, path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if swap_path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("swap file did not appear at {}", swap_path.display());
}

/// Wait for the swap file body to contain `expected_body`.
pub fn wait_for_swap_body(cache_root: &Path, path: &Path, expected_body: &str) {
    let swap_path = compute_swap_path(cache_root, path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(contents) = std::fs::read_to_string(&swap_path)
            && contents
                .split_once("\n\n")
                .is_some_and(|(_, body)| body == expected_body)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "swap body did not match {:?} at {}",
        expected_body,
        swap_path.display()
    );
}

/// Encode one absolute path using ordex's swap filename scheme.
fn encode_path(path: &Path) -> String {
    path.to_str()
        .expect("swap test paths must be UTF-8")
        .replace('%', "%%")
        .replace('/', "%2F")
}
