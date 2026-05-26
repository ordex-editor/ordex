//! Polling-only file monitor API used on non-Linux platforms.

use super::polling::PollingFileMonitor;
use std::path::PathBuf;

/// Session-wide file monitor backed by periodic metadata polling.
#[derive(Debug, Default)]
pub(crate) struct FileMonitor {
    backend: PollingFileMonitor,
}

impl FileMonitor {
    /// Create one file monitor using metadata polling.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Synchronize the monitored path set with the currently open named buffers.
    pub(crate) fn sync_paths(&mut self, paths: &[PathBuf]) {
        self.backend.sync_paths(paths);
    }

    /// Return the paths whose on-disk metadata changed since the last monitor poll.
    pub(crate) fn poll_changed_paths(&mut self) -> Vec<PathBuf> {
        self.backend.poll_changed_paths()
    }

    /// Take one pending backend warning, if any.
    pub(crate) fn take_warning(&mut self) -> Option<String> {
        None
    }
}
