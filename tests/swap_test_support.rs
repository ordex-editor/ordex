use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use test_utils::PtySession;

/// Return the swap directory used by one PTY-backed ordex session.
pub fn swap_dir(cache_root: &Path) -> PathBuf {
    cache_root.join("ordex").join("swap")
}

/// Return the swap path corresponding to one source file path.
pub fn compute_swap_path(cache_root: &Path, path: &Path) -> PathBuf {
    swap_dir(cache_root).join(format!("{}.swp", encode_path(path)))
}

/// Wait for one swap file to exist, draining the PTY master so ordex can write renders.
///
/// Draining prevents the kernel PTY pipe from filling up and blocking ordex's
/// writes to stdout when the test stops reading between `wait_until` calls.
#[track_caller]
pub fn wait_for_swap_file(session: &mut PtySession, path: &Path) {
    let swap_path = compute_swap_path(session.cache_root(), path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let _ = session.read_available();
        if swap_path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("swap file did not appear at {}", swap_path.display());
}

/// Wait for the swap file body to match, draining the PTY master between polls.
///
/// Draining prevents the kernel PTY pipe from filling up and blocking ordex's
/// writes to stdout when the test stops reading between `wait_until` calls.
#[track_caller]
pub fn wait_for_swap_body(session: &mut PtySession, path: &Path, expected_body: &str) {
    let swap_path = compute_swap_path(session.cache_root(), path);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let _ = session.read_available();
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
