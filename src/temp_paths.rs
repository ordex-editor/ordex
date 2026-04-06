//! Unique sibling temp-path helpers for atomic file replacement.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
const UNIQUE_PATH_ATTEMPTS: usize = 16;

/// Build one unique sibling temp path next to `target_path`.
pub(crate) fn unique_sibling_temp_path(target_path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let file_name = target_path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target path is missing a file name",
        )
    })?;
    for _ in 0..UNIQUE_PATH_ATTEMPTS {
        let mut temp_name = OsString::from(file_name);
        temp_name.push(format!(
            ".{suffix}.{}.{}.tmp",
            std::process::id(),
            unique_temp_nonce()?
        ));
        let candidate = target_path.with_file_name(temp_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique temp path beside the target file",
    ))
}

/// Return one monotonic nonce for temp-file names.
fn unique_temp_nonce() -> io::Result<u64> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos() as u64;
    Ok(timestamp ^ TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed))
}
