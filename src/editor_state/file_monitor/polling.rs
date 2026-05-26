//! Metadata-polling backend used directly on non-Linux platforms and as Linux fallback.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Lightweight metadata signature used by the polling backend to notice candidate changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MetadataFingerprint {
    Missing,
    Present {
        /// Last modification timestamp reported by the filesystem, when available.
        modified: Option<SystemTime>,
        /// File length in bytes reported by the filesystem metadata.
        len: u64,
    },
}

/// Metadata-polling backend for platforms without a native watcher in this implementation.
#[derive(Debug, Default)]
pub(super) struct PollingFileMonitor {
    tracked: HashMap<PathBuf, MetadataFingerprint>,
    next_poll_at: Option<Instant>,
}

impl PollingFileMonitor {
    /// Synchronize the tracked metadata snapshot set with the current open file paths.
    pub(super) fn sync_paths(&mut self, paths: &[PathBuf]) {
        let desired = paths.iter().cloned().collect::<HashSet<_>>();
        self.tracked.retain(|path, _| desired.contains(path));

        // New paths snapshot current metadata immediately so the first fallback
        // poll reports only subsequent changes, not the initial open state.
        for path in desired {
            self.tracked
                .entry(path.clone())
                .or_insert_with(|| metadata_fingerprint(&path));
        }

        self.next_poll_at =
            (!self.tracked.is_empty()).then(|| Instant::now() + FALLBACK_POLL_INTERVAL);
    }

    /// Return the paths whose metadata changed since the last fallback poll.
    pub(super) fn poll_changed_paths(&mut self) -> Vec<PathBuf> {
        if self
            .next_poll_at
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return Vec::new();
        }

        let mut changed = Vec::new();

        // Polling is only a candidate detector; the caller still re-reads the
        // file contents before deciding whether the buffer truly diverged.
        for (path, previous) in &mut self.tracked {
            let current = metadata_fingerprint(path);
            if *previous != current {
                *previous = current;
                changed.push(path.clone());
            }
        }

        self.next_poll_at =
            (!self.tracked.is_empty()).then(|| Instant::now() + FALLBACK_POLL_INTERVAL);
        changed
    }
}

/// Build one polling metadata fingerprint for `path`.
fn metadata_fingerprint(path: &Path) -> MetadataFingerprint {
    match fs::metadata(path) {
        Ok(metadata) => MetadataFingerprint::Present {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        },
        Err(_) => MetadataFingerprint::Missing,
    }
}
