//! Swap-file lifecycle management.

pub(crate) mod format;
pub(crate) mod glob;
pub(crate) mod location;
mod platform;

use crate::swap::format::SwapMeta;
use crate::temp_paths;
use crate::text_buffer::TextBuffer;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const UNNAMED_BUFFER_MARKER: &str = "__ordex_unnamed_buffer__";

/// One on-disk swap file attached to one editor buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwapHandle {
    pub(crate) swap_path: PathBuf,
    meta: SwapMeta,
}

/// Recovery payload loaded from an existing swap file.
#[derive(Debug)]
pub(crate) struct SwapRecovery {
    pub(crate) handle: SwapHandle,
    pub(crate) buffer: TextBuffer,
}

impl SwapHandle {
    /// Create a fresh swap file for `source_path` from the current buffer text.
    pub(crate) fn create_from_buffer(source_path: &Path, buffer: &TextBuffer) -> io::Result<Self> {
        let swap_path = location::swap_path_for(source_path, &location::default_swap_dir()?)?;
        let meta = build_swap_meta(source_path)?;
        write_swap_from_buffer(&swap_path, &meta, buffer)?;
        Ok(Self { swap_path, meta })
    }

    /// Create a fresh swap file for one unnamed buffer from the current buffer text.
    pub(crate) fn create_for_unnamed_buffer(buffer: &TextBuffer) -> io::Result<Self> {
        let swap_dir = unnamed_swap_dir()?;
        let identity = unnamed_buffer_identity()?;
        let swap_path = unnamed_swap_path_in_dir(&swap_dir)?;
        let meta = build_swap_meta(&identity)?;
        write_swap_from_buffer(&swap_path, &meta, buffer)?;
        Ok(Self { swap_path, meta })
    }

    /// Rewrite the swap file so it contains the current in-memory buffer.
    pub(crate) fn refresh(&mut self, buffer: &TextBuffer) -> io::Result<()> {
        self.meta.last_refreshed_at = unix_timestamp()?;
        write_swap_from_buffer(&self.swap_path, &self.meta, buffer)
    }

    /// Delete the swap file, tolerating an already-missing path.
    pub(crate) fn delete(self) -> io::Result<()> {
        delete_swap_path(&self.swap_path)
    }

    /// Return the filesystem path of the underlying swap file.
    pub(crate) fn swap_path(&self) -> &Path {
        &self.swap_path
    }
}

/// Open one existing swap file for recovery, if it exists.
pub(crate) fn load_recovery(source_path: &Path) -> io::Result<Option<SwapRecovery>> {
    let swap_path = location::swap_path_for(source_path, &location::default_swap_dir()?)?;
    if !swap_path.exists() {
        return Ok(None);
    }

    let file = File::open(&swap_path)?;
    let mut reader = BufReader::new(file);
    let meta = format::SwapMeta::read_header(&mut reader)?;
    if meta.original_path != source_path {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "swap original path does not match the requested file",
        ));
    }
    let buffer = TextBuffer::from_reader(reader)?;
    Ok(Some(SwapRecovery {
        handle: SwapHandle { swap_path, meta },
        buffer,
    }))
}

/// Open the newest unnamed-buffer swap file for recovery, if one exists.
pub(crate) fn load_unnamed_recovery() -> io::Result<Option<SwapRecovery>> {
    let swap_dir = unnamed_swap_dir()?;
    let identity = unnamed_buffer_identity()?;
    let Some(swap_path) = newest_unnamed_swap_path_in_dir(&swap_dir)? else {
        return Ok(None);
    };
    let file = File::open(&swap_path)?;
    let mut reader = BufReader::new(file);
    let meta = format::SwapMeta::read_header(&mut reader)?;
    if meta.original_path != identity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unnamed swap marker does not match the expected recovery path",
        ));
    }
    let buffer = TextBuffer::from_reader(reader)?;
    Ok(Some(SwapRecovery {
        handle: SwapHandle { swap_path, meta },
        buffer,
    }))
}

/// Delete one swap file path, treating `NotFound` as success.
pub(crate) fn delete_swap_path(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Return the synthetic absolute path stored in unnamed-buffer swap headers.
fn unnamed_buffer_identity() -> io::Result<PathBuf> {
    Ok(location::default_swap_dir()?.join(UNNAMED_BUFFER_MARKER))
}

/// Return the directory that stores unnamed-buffer swap files.
fn unnamed_swap_dir() -> io::Result<PathBuf> {
    Ok(location::default_swap_dir()?.join("unnamed"))
}

/// Return one unique unnamed-buffer swap path inside `swap_dir`.
fn unnamed_swap_path_in_dir(swap_dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(swap_dir)?;
    temp_paths::unique_path_in_directory(swap_dir, "unnamed", "swp")
}

/// Return the newest unnamed-buffer swap path in `swap_dir`, if any candidates exist.
fn newest_unnamed_swap_path_in_dir(swap_dir: &Path) -> io::Result<Option<PathBuf>> {
    let mut newest = None::<(SystemTime, PathBuf)>;
    let entries = match fs::read_dir(swap_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    // Recovery only needs one unnamed buffer at startup, so pick the newest swap
    // file first and leave older unnamed swaps untouched until the user inspects them.
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("swp") {
            continue;
        }
        let modified_at = entry.metadata()?.modified()?;
        let replace_newest = newest
            .as_ref()
            .is_none_or(|(current_time, _)| modified_at > *current_time);
        if replace_newest {
            newest = Some((modified_at, path));
        }
    }

    Ok(newest.map(|(_, path)| path))
}

/// Write one swap file atomically from the current in-memory buffer contents.
fn write_swap_from_buffer(
    swap_path: &Path,
    meta: &SwapMeta,
    buffer: &TextBuffer,
) -> io::Result<()> {
    atomic_write_file(swap_path, "tmp", |file| {
        meta.write_header(file)?;
        buffer.write_to(file)
    })
}

/// Atomically replace `target_path` by writing through one temp file first.
fn atomic_write_file<F>(target_path: &Path, temp_suffix: &str, write_body: F) -> io::Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let Some(parent) = target_path.parent() else {
        // `Path::parent()` can be absent for bare relative names such as `foo`,
        // which are invalid here because swap paths always resolve under a cache
        // directory and therefore must already have a parent component.
        return Err(io::Error::other(
            "swap path is missing its parent directory",
        ));
    };
    fs::create_dir_all(parent)?;
    let temp_path = temp_path_for(target_path, temp_suffix)?;

    // The temp file lives beside the final target so the rename stays atomic on
    // the same filesystem and never exposes a partially-written swap file.
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        write_body(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp_path, target_path)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

/// Build one sibling temp path next to `target_path`.
fn temp_path_for(target_path: &Path, suffix: &str) -> io::Result<PathBuf> {
    temp_paths::unique_sibling_temp_path(target_path, suffix)
}

/// Return the current process identifier.
fn current_pid() -> u32 {
    std::process::id()
}

/// Return the current hostname through the platform helper.
fn current_hostname() -> io::Result<String> {
    platform::current_hostname()
}

/// Return the current Unix timestamp in seconds.
fn unix_timestamp() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}

/// Build the swap metadata used by both source-backed and buffer-backed writers.
fn build_swap_meta(source_path: &Path) -> io::Result<SwapMeta> {
    let now = unix_timestamp()?;
    Ok(SwapMeta {
        pid: current_pid(),
        hostname: current_hostname()?,
        original_path: source_path.to_path_buf(),
        opened_at: now,
        last_refreshed_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use test_utils::{TempFile, TempTree};

    /// Build one absolute swap target under a temp directory.
    fn temp_swap_path(tree: &TempTree) -> PathBuf {
        tree.path().join("file.swp")
    }

    #[test]
    fn deletes_missing_swap_as_success() {
        let tree = TempTree::with_prefix("ordex_swap_missing").expect("temp tree");
        delete_swap_path(&tree.path().join("missing.swp")).expect("delete missing swap");
    }

    #[test]
    fn refresh_rewrites_swap_body_from_buffer() {
        let swap_root = TempTree::with_prefix("ordex_swap_refresh_root").expect("temp tree");
        let source_file = TempFile::with_suffix("_refresh_source.txt").expect("temp file");
        source_file.write_all(b"disk").expect("seed source");
        let source_path = source_file.path().to_path_buf();
        let swap_path = location::swap_path_for(&source_path, swap_root.path()).expect("swap path");

        let now = unix_timestamp().expect("timestamp");
        let mut handle = SwapHandle {
            swap_path: swap_path.clone(),
            meta: SwapMeta {
                pid: 1,
                hostname: "host".to_string(),
                original_path: source_path.clone(),
                opened_at: now,
                last_refreshed_at: now,
            },
        };

        handle
            .refresh(
                &TextBuffer::from_reader(std::io::Cursor::new(b"edited".to_vec())).expect("buffer"),
            )
            .expect("refresh swap");

        let mut reader = BufReader::new(File::open(&swap_path).expect("open swap"));
        let meta = SwapMeta::read_header(&mut reader).expect("read header");
        let mut body = String::new();
        reader.read_to_string(&mut body).expect("read body");
        assert_eq!(meta.original_path, source_path);
        assert_eq!(body, "edited");
    }

    #[test]
    fn atomic_write_replaces_target_without_leaving_temp_file() {
        let tree = TempTree::with_prefix("ordex_swap_atomic").expect("temp tree");
        let swap_path = temp_swap_path(&tree);

        atomic_write_file(&swap_path, "tmp", |file| file.write_all(b"first")).expect("write first");
        atomic_write_file(&swap_path, "tmp", |file| file.write_all(b"second"))
            .expect("write second");

        assert_eq!(fs::read_to_string(&swap_path).expect("read swap"), "second");
        assert!(!swap_path.with_file_name("file.swp.tmp").exists());
        assert_eq!(
            fs::read_dir(tree.path())
                .expect("read temp dir")
                .filter_map(Result::ok)
                .count(),
            1
        );
    }

    #[test]
    fn loads_newest_unnamed_recovery() {
        let cache_root = TempTree::with_prefix("ordex_swap_unnamed_cache").expect("temp tree");
        let unnamed_dir = cache_root.path().join("unnamed");
        let identity = cache_root.path().join(UNNAMED_BUFFER_MARKER);

        let older_path = unnamed_swap_path_in_dir(&unnamed_dir).expect("older unnamed path");
        let older_meta = build_swap_meta(&identity).expect("older meta");
        write_swap_from_buffer(&older_path, &older_meta, &TextBuffer::from_str("older"))
            .expect("older unnamed swap");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let newer_path = unnamed_swap_path_in_dir(&unnamed_dir).expect("newer unnamed path");
        let newer_meta = build_swap_meta(&identity).expect("newer meta");
        write_swap_from_buffer(&newer_path, &newer_meta, &TextBuffer::from_str("newer"))
            .expect("newer unnamed swap");

        let recovery_path = newest_unnamed_swap_path_in_dir(&unnamed_dir)
            .expect("load newest unnamed path")
            .expect("unnamed recovery path");
        assert_eq!(recovery_path, newer_path);

        delete_swap_path(&older_path).expect("delete older unnamed swap");
        delete_swap_path(&newer_path).expect("delete newest unnamed swap");
    }
}
