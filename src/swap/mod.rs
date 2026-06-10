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
    pub(crate) swap_path: PathBuf,
    pub(crate) buffer: TextBuffer,
}

/// One swap file that appears to belong to another Ordex instance.
#[derive(Debug)]
pub(crate) struct SwapConflict {
    pub(crate) swap_path: PathBuf,
    pub(crate) meta: SwapMeta,
    pub(crate) buffer: TextBuffer,
    pub(crate) state: SwapConflictState,
}

/// Classification of one existing swap file discovered while opening a buffer.
#[derive(Debug)]
pub(crate) enum ExistingSwap {
    Recoverable(SwapRecovery),
    Conflicting(SwapConflict),
}

/// Why Ordex treats an existing swap file as belonging to another instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwapConflictState {
    RunningLocally,
    OtherHost,
    UnknownLocalStatus,
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
        let identity = unnamed_buffer_identity()?;
        Self::create_from_buffer(&identity, buffer)
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

/// Inspect one existing swap file for `source_path`, if it exists.
pub(crate) fn inspect_existing_swap(source_path: &Path) -> io::Result<Option<ExistingSwap>> {
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
    if meta.pid == current_pid()
        && current_hostname().ok().as_deref() == Some(meta.hostname.as_str())
    {
        return Ok(None);
    }
    let buffer = TextBuffer::from_reader(reader)?;
    Ok(Some(classify_existing_swap(swap_path, meta, buffer)))
}

/// Inspect one unnamed-buffer swap file, if it exists.
///
/// Scans the default swap directory for orphaned unnamed-buffer swap files whose
/// recorded working directory matches the current process. Swap files belonging to
/// other still-running ordex instances are silently skipped because each process
/// owns its own unique unnamed-buffer identity.
pub(crate) fn inspect_unnamed_swap() -> io::Result<Option<ExistingSwap>> {
    let swap_dir = location::default_swap_dir()?;
    let cwd = std::env::current_dir()?;
    let prefix = unnamed_buffer_prefix();
    scan_unnamed_swap_candidates(&swap_dir, &cwd, &prefix)
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
///
/// Each process embeds its PID in the filename so that concurrent ordex instances
/// in the same working directory produce distinct swap paths and do not interfere.
fn unnamed_buffer_identity() -> io::Result<PathBuf> {
    Ok(std::env::current_dir()?.join(format!("{UNNAMED_BUFFER_MARKER}.{}", current_pid())))
}

/// Return the shared filename prefix that identifies unnamed-buffer swap files.
fn unnamed_buffer_prefix() -> PathBuf {
    PathBuf::from(UNNAMED_BUFFER_MARKER)
}

/// Scan `swap_dir` for orphaned unnamed-buffer swap files and return the best
/// candidate for recovery.
///
/// Entries whose recorded parent directory differ from `cwd` are ignored so that
/// instances started in different working directories never cross-match.
///
/// Returns the most recently refreshed unnamed-buffer swap whose originating
/// process is no longer running. Live-process swaps are silently skipped because
/// each instance owns its own unique unnamed-buffer identity.
///
/// Returns `None` when no candidate exists, and `Ok(Some(...))` when the best
/// recoverable swap is returned or when a same-host conflict cannot be resolved.
fn scan_unnamed_swap_candidates(
    swap_dir: &Path,
    cwd: &Path,
    prefix: &Path,
) -> io::Result<Option<ExistingSwap>> {
    let entries = match fs::read_dir(swap_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let mut best_recovery: Option<(u64, SwapRecovery)> = None;
    let mut first_conflict: Option<SwapConflict> = None;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Some(candidate) = parse_unnamed_candidate(&entry_path, cwd, prefix)? else {
            continue;
        };

        match candidate {
            ExistingSwap::Recoverable(recovery) => {
                let refreshed = scan_last_refreshed_for(&entry_path).unwrap_or(0);
                let is_better = best_recovery
                    .as_ref()
                    .is_none_or(|(best_time, _)| refreshed > *best_time);
                if is_better {
                    best_recovery = Some((refreshed, recovery));
                }
            }
            ExistingSwap::Conflicting(conflict) => {
                if first_conflict.is_none() {
                    first_conflict = Some(conflict);
                }
            }
        }
    }

    if let Some((_, recovery)) = best_recovery {
        return Ok(Some(ExistingSwap::Recoverable(recovery)));
    }
    Ok(first_conflict.map(ExistingSwap::Conflicting))
}

/// Parse one swap file and return its classification when it is an unnamed-buffer
/// candidate under `cwd`.
///
/// Returns `None` for files that are not `.swp`, whose header cannot be parsed,
/// whose `original_path` does not record an unnamed-buffer identity under `cwd`,
/// or when the originating process is still running.
fn parse_unnamed_candidate(
    path: &Path,
    cwd: &Path,
    prefix: &Path,
) -> io::Result<Option<ExistingSwap>> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("swp") {
        return Ok(None);
    }

    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut reader = BufReader::new(file);
    let meta = match format::SwapMeta::read_header(&mut reader) {
        Ok(meta) => meta,
        Err(_) => return Ok(None),
    };

    if !original_path_is_unnamed(&meta.original_path, cwd, prefix) {
        return Ok(None);
    }

    if meta.pid == current_pid() {
        return Ok(None);
    }

    let current_hostname = match current_hostname() {
        Ok(hostname) => hostname,
        Err(_) => return Ok(None),
    };

    if meta.hostname == current_hostname {
        match platform::process_is_running(meta.pid) {
            Ok(true) => return Ok(None),
            Ok(false) => {}
            Err(_) => return Ok(None),
        }
    }

    let buffer = TextBuffer::from_reader(reader)?;
    Ok(Some(classify_existing_swap(
        path.to_path_buf(),
        meta,
        buffer,
    )))
}

/// Return whether `original_path` records an unnamed-buffer identity rooted at `cwd`.
///
/// The match succeeds only when the filename begins with the marker followed by a `.` and the per-PID suffix.
fn original_path_is_unnamed(original_path: &Path, cwd: &Path, prefix: &Path) -> bool {
    let Some(recorded_parent) = original_path.parent() else {
        return false;
    };
    if recorded_parent != cwd {
        return false;
    }

    let Some(recorded_name) = original_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let prefix_name = prefix
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    recorded_name.starts_with(&format!("{prefix_name}."))
}

/// Return the `last_refreshed_at` timestamp from a swap file without reading its body.
fn scan_last_refreshed_for(path: &Path) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let meta = format::SwapMeta::read_header(&mut reader).ok()?;
    Some(meta.last_refreshed_at)
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

/// Classify one parsed swap file as either recoverable or owned by another instance.
fn classify_existing_swap(swap_path: PathBuf, meta: SwapMeta, buffer: TextBuffer) -> ExistingSwap {
    match classify_swap_conflict_state(&meta) {
        None => ExistingSwap::Recoverable(SwapRecovery { swap_path, buffer }),
        Some(state) => ExistingSwap::Conflicting(SwapConflict {
            swap_path,
            meta,
            buffer,
            state,
        }),
    }
}

/// Return the conflict state for `meta`, or `None` when the swap is stale.
fn classify_swap_conflict_state(meta: &SwapMeta) -> Option<SwapConflictState> {
    let current_hostname = match current_hostname() {
        Ok(hostname) => hostname,
        Err(_) => return Some(SwapConflictState::UnknownLocalStatus),
    };
    if meta.hostname != current_hostname {
        return Some(SwapConflictState::OtherHost);
    }
    if meta.pid == current_pid() {
        return None;
    }

    match platform::process_is_running(meta.pid) {
        Ok(true) => Some(SwapConflictState::RunningLocally),
        Ok(false) => None,
        Err(_) => Some(SwapConflictState::UnknownLocalStatus),
    }
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

    /// Recovery is allowed when the swap pid matches the current process.
    #[test]
    fn classifies_current_process_swap_as_recoverable() {
        let meta = SwapMeta {
            pid: current_pid(),
            hostname: current_hostname().expect("hostname"),
            original_path: PathBuf::from("/tmp/current.txt"),
            opened_at: 1,
            last_refreshed_at: 1,
        };

        assert_eq!(classify_swap_conflict_state(&meta), None);
    }

    /// Recovery is allowed once the originating process is no longer running.
    #[test]
    fn classifies_missing_process_swap_as_recoverable() {
        let meta = SwapMeta {
            pid: u32::MAX,
            hostname: current_hostname().expect("hostname"),
            original_path: PathBuf::from("/tmp/stale.txt"),
            opened_at: 1,
            last_refreshed_at: 1,
        };

        assert_eq!(classify_swap_conflict_state(&meta), None);
    }

    /// Swaps from a different host remain conservative conflict warnings.
    #[test]
    fn classifies_other_host_swap_as_conflict() {
        let meta = SwapMeta {
            pid: 1,
            hostname: "other-host".to_string(),
            original_path: PathBuf::from("/tmp/shared.txt"),
            opened_at: 1,
            last_refreshed_at: 1,
        };

        assert_eq!(
            classify_swap_conflict_state(&meta),
            Some(SwapConflictState::OtherHost)
        );
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

    /// Write one swap file directly into `swap_dir` with the given identity and timestamps.
    fn write_unnamed_swap_file(
        swap_dir: &Path,
        original_path: &Path,
        pid: u32,
        hostname: &str,
        opened_at: u64,
        last_refreshed_at: u64,
        body: &str,
    ) -> PathBuf {
        let swap_path = location::swap_path_for(original_path, swap_dir).expect("swap path");
        let meta = SwapMeta {
            pid,
            hostname: hostname.to_string(),
            original_path: original_path.to_path_buf(),
            opened_at,
            last_refreshed_at,
        };
        let buffer = TextBuffer::from_reader(std::io::Cursor::new(body.as_bytes().to_vec()))
            .expect("buffer");
        fs::create_dir_all(swap_dir).expect("create swap dir");
        write_swap_from_buffer(&swap_path, &meta, &buffer).expect("write swap");
        swap_path
    }

    /// Scan returns `None` when the swap directory is empty or missing.
    #[test]
    fn scan_returns_none_when_no_candidates() {
        let swap_root = TempTree::with_prefix("ordex_scan_empty").expect("temp tree");
        let cwd = std::env::current_dir().expect("cwd");
        let prefix = unnamed_buffer_prefix();

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        assert!(result.is_none(), "empty dir should yield None");
    }

    /// Swap files belonging to a different CWD are silently ignored.
    #[test]
    fn scan_ignores_swap_files_from_different_cwd() {
        let swap_root = TempTree::with_prefix("ordex_scan_other_cwd").expect("temp tree");
        let other_cwd = PathBuf::from("/tmp/some/other/dir");
        let original_path = other_cwd.join(UNNAMED_BUFFER_MARKER);
        let prefix = unnamed_buffer_prefix();
        let cwd = std::env::current_dir().expect("cwd");

        write_unnamed_swap_file(
            swap_root.path(),
            &original_path,
            u32::MAX,
            &current_hostname().expect("hostname"),
            1,
            1,
            "other-cwd-body",
        );

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        assert!(
            result.is_none(),
            "swap from different CWD should be ignored"
        );
    }

    /// Non-unnamed swap files (regular named files) are silently ignored.
    #[test]
    fn scan_ignores_regular_file_swaps() {
        let swap_root = TempTree::with_prefix("ordex_scan_regular").expect("temp tree");
        let cwd = std::env::current_dir().expect("cwd");
        let regular_path = cwd.join("some_source_file.txt");
        let prefix = unnamed_buffer_prefix();

        write_unnamed_swap_file(
            swap_root.path(),
            &regular_path,
            u32::MAX,
            &current_hostname().expect("hostname"),
            1,
            1,
            "regular-body",
        );

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        assert!(result.is_none(), "regular file swap should be ignored");
    }

    /// Swap files from a dead process are returned as recoverable.
    #[test]
    fn scan_recovers_dead_process_swap() {
        let swap_root = TempTree::with_prefix("ordex_scan_dead").expect("temp tree");
        let cwd = std::env::current_dir().expect("cwd");
        let original_path = cwd.join(format!("{UNNAMED_BUFFER_MARKER}.99999"));
        let prefix = unnamed_buffer_prefix();

        write_unnamed_swap_file(
            swap_root.path(),
            &original_path,
            u32::MAX,
            &current_hostname().expect("hostname"),
            1,
            100,
            "dead-body",
        );

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        let Some(ExistingSwap::Recoverable(recovery)) = result else {
            panic!("expected recoverable swap, got {result:?}");
        };
        assert!(
            recovery.buffer.to_string().contains("dead-body"),
            "recovered buffer should contain the original body"
        );
    }

    /// Swap files from a live process on the same host are silently skipped.
    #[test]
    fn scan_returns_none_for_live_process_swap() {
        let swap_root = TempTree::with_prefix("ordex_scan_live").expect("temp tree");
        let cwd = std::env::current_dir().expect("cwd");
        // PID 1 (init) is always running.
        let original_path = cwd.join(format!("{UNNAMED_BUFFER_MARKER}.1"));
        let prefix = unnamed_buffer_prefix();

        write_unnamed_swap_file(
            swap_root.path(),
            &original_path,
            1,
            &current_hostname().expect("hostname"),
            1,
            100,
            "live-body",
        );

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        assert!(
            result.is_none(),
            "live-process swap should be silently skipped, got {result:?}"
        );
    }

    /// When two orphaned unnamed-buffer swap files exist, the most recently
    /// modified one is preferred for recovery.
    #[test]
    fn scan_picks_most_recently_modified_among_multiple_orphans() {
        let swap_root = TempTree::with_prefix("ordex_scan_multi").expect("temp tree");
        let cwd = std::env::current_dir().expect("cwd");
        let prefix = unnamed_buffer_prefix();

        let older_path = cwd.join(format!("{UNNAMED_BUFFER_MARKER}.88881"));
        let newer_path = cwd.join(format!("{UNNAMED_BUFFER_MARKER}.88882"));

        write_unnamed_swap_file(
            swap_root.path(),
            &older_path,
            u32::MAX,
            &current_hostname().expect("hostname"),
            1,
            10,
            "older-body",
        );
        write_unnamed_swap_file(
            swap_root.path(),
            &newer_path,
            u32::MAX,
            &current_hostname().expect("hostname"),
            2,
            100,
            "newer-body",
        );

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        let Some(ExistingSwap::Recoverable(recovery)) = result else {
            panic!("expected recoverable swap, got {result:?}");
        };
        assert!(
            recovery.buffer.to_string().contains("newer-body"),
            "scan should prefer the most recently refreshed swap"
        );
    }

    /// Swap files from a different host are returned as conflicts since they cannot
    /// be safely classified without a live process check.
    #[test]
    fn scan_returns_conflict_for_other_host_swap() {
        let swap_root = TempTree::with_prefix("ordex_scan_other_host").expect("temp tree");
        let cwd = std::env::current_dir().expect("cwd");
        let original_path = cwd.join(UNNAMED_BUFFER_MARKER);
        let prefix = unnamed_buffer_prefix();

        write_unnamed_swap_file(
            swap_root.path(),
            &original_path,
            12345,
            "other-host-xyz",
            1,
            100,
            "remote-body",
        );

        let result = scan_unnamed_swap_candidates(swap_root.path(), &cwd, &prefix).expect("scan");
        let Some(ExistingSwap::Conflicting(conflict)) = result else {
            panic!("expected conflicting swap, got {result:?}");
        };
        assert_eq!(conflict.state, SwapConflictState::OtherHost);
    }

    /// `unnamed_buffer_identity` must include the current PID so concurrent
    /// instances in the same CWD produce distinct swap paths.
    #[test]
    fn unnamed_buffer_identity_includes_pid() {
        let identity = unnamed_buffer_identity().expect("identity");
        let pid_str = current_pid().to_string();
        let file_name = identity
            .file_name()
            .and_then(|name| name.to_str())
            .expect("filename");
        assert!(
            file_name.contains(&pid_str),
            "identity filename {file_name:?} should embed current PID {pid_str}"
        );
        assert!(
            file_name.starts_with(UNNAMED_BUFFER_MARKER),
            "identity filename should begin with the unnamed marker"
        );
    }
}
