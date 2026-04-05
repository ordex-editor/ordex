//! Swap-file lifecycle management.

pub(crate) mod format;
pub(crate) mod glob;
pub(crate) mod location;

use crate::swap::format::SwapMeta;
use crate::text_buffer::TextBuffer;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Create a fresh swap file for `source_path`.
    pub(crate) fn create(source_path: &Path) -> io::Result<Self> {
        let swap_dir = location::default_swap_dir()?;
        let swap_path = location::swap_path_for(source_path, &swap_dir);
        let now = unix_timestamp()?;
        let meta = SwapMeta {
            pid: current_pid(),
            hostname: current_hostname()?,
            original_path: source_path.to_path_buf(),
            opened_at: now,
            last_refreshed_at: now,
        };
        write_swap_from_source(&swap_path, &meta, source_path)?;
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
    let swap_path = location::swap_path_for(source_path, &location::default_swap_dir()?);
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
    let mut body = String::new();
    reader.read_to_string(&mut body)?;
    let buffer = TextBuffer::from_reader(std::io::Cursor::new(body.into_bytes()))?;
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

/// Write one swap file atomically from the current on-disk source file contents.
fn write_swap_from_source(swap_path: &Path, meta: &SwapMeta, source_path: &Path) -> io::Result<()> {
    atomic_write_file(swap_path, "tmp", |file| {
        meta.write_header(file)?;
        if source_path.exists() {
            let mut source = File::open(source_path)?;
            io::copy(&mut source, file)?;
        }
        Ok(())
    })
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
        return Err(io::Error::other(
            "swap path is missing its parent directory",
        ));
    };
    fs::create_dir_all(parent)?;
    let temp_path = temp_path_for(target_path, temp_suffix);

    // The temp file lives beside the final target so the rename stays atomic on
    // the same filesystem and never exposes a partially-written swap file.
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
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
fn temp_path_for(target_path: &Path, suffix: &str) -> PathBuf {
    let file_name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("swap target path must have a UTF-8 file name");
    target_path.with_file_name(format!("{file_name}.{suffix}"))
}

/// Return the current process identifier.
fn current_pid() -> u32 {
    unsafe { libc::getpid() as u32 }
}

/// Return the current hostname using libc.
fn current_hostname() -> io::Result<String> {
    let mut bytes = [0_u8; 256];
    let rc = unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let len = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    Ok(String::from_utf8_lossy(&bytes[..len]).into_owned())
}

/// Return the current Unix timestamp in seconds.
fn unix_timestamp() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::io::Write;

    /// Build one absolute temp file path unique to this process and test.
    fn temp_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("ordex_swap_test_{}_{}", std::process::id(), name))
    }

    /// Build one absolute swap target under a temp directory.
    fn temp_swap_path(name: &str) -> PathBuf {
        temp_path(name).join("file.swp")
    }

    #[test]
    fn deletes_missing_swap_as_success() {
        delete_swap_path(&temp_path("missing.swp")).expect("delete missing swap");
    }

    #[test]
    fn refresh_rewrites_swap_body_from_buffer() {
        let swap_root = temp_path("refresh_root");
        let source_path = temp_path("refresh_source.txt");
        let swap_path = location::swap_path_for(&source_path, &swap_root);
        let _ = fs::remove_dir_all(&swap_root);
        let _ = fs::remove_file(&source_path);
        fs::write(&source_path, "disk").expect("seed source");

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

        let _ = fs::remove_dir_all(&swap_root);
        let _ = fs::remove_file(&source_path);
    }

    #[test]
    fn atomic_write_replaces_target_without_leaving_temp_file() {
        let swap_path = temp_swap_path("atomic");
        let _ = fs::remove_dir_all(swap_path.parent().expect("swap parent"));

        atomic_write_file(&swap_path, "tmp", |file| file.write_all(b"first")).expect("write first");
        atomic_write_file(&swap_path, "tmp", |file| file.write_all(b"second"))
            .expect("write second");

        assert_eq!(fs::read_to_string(&swap_path).expect("read swap"), "second");
        assert!(!swap_path.with_file_name("file.swp.tmp").exists());

        let _ = fs::remove_dir_all(swap_path.parent().expect("swap parent"));
    }
}
