//! Test utilities for ordex
//!
//! Provides RAII-based temporary file handling for tests.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A temporary file that is automatically deleted when dropped.
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub fn new() -> io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "ordex_test_{}_{}", std::process::id(), id
        ));
        File::create(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn write_all(&self, data: &[u8]) -> io::Result<()> {
        fs::write(&self.path, data)
    }

    pub fn writeln(&self, line: &str) -> io::Result<()> {
        let mut file = fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}", line)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
