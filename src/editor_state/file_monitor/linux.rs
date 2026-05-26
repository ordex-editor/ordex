//! Linux file monitor API backed by inotify with metadata-polling fallback.

use super::linux_inotify::LinuxInotify;
use super::polling::PollingFileMonitor;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Session-wide file monitor that prefers native Linux notifications and falls back to polling.
#[derive(Debug)]
pub(crate) struct FileMonitor {
    backend: FileMonitorBackend,
    pending_warning: Option<String>,
}

#[derive(Debug)]
enum FileMonitorBackend {
    Linux(Box<LinuxFileMonitor>),
    Polling(PollingFileMonitor),
}

impl Default for FileMonitor {
    /// Create one file monitor using the best backend available on this platform.
    fn default() -> Self {
        Self::new()
    }
}

impl FileMonitor {
    /// Create one file monitor using Linux inotify when available, otherwise polling.
    pub(crate) fn new() -> Self {
        match LinuxFileMonitor::new() {
            Ok(backend) => Self {
                backend: FileMonitorBackend::Linux(Box::new(backend)),
                pending_warning: None,
            },
            Err(error) => Self {
                backend: FileMonitorBackend::Polling(PollingFileMonitor::default()),
                pending_warning: Some(format!(
                    "File watcher unavailable; using metadata polling: {error}"
                )),
            },
        }
    }

    /// Synchronize the monitored path set with the currently open named buffers.
    pub(crate) fn sync_paths(&mut self, paths: &[PathBuf]) {
        if let FileMonitorBackend::Linux(backend) = &mut self.backend {
            if backend.sync_paths(paths).is_err() {
                self.fallback_to_polling(paths);
            }
            return;
        }

        if let FileMonitorBackend::Polling(backend) = &mut self.backend {
            backend.sync_paths(paths);
        }
    }

    /// Return the paths whose on-disk metadata changed since the last monitor poll.
    pub(crate) fn poll_changed_paths(&mut self) -> Vec<PathBuf> {
        if let FileMonitorBackend::Linux(backend) = &mut self.backend {
            match backend.poll_changed_paths() {
                Ok(paths) => return paths,
                Err(_) => {
                    let tracked = backend.tracked_paths();
                    self.fallback_to_polling(&tracked);
                }
            }
        }

        match &mut self.backend {
            FileMonitorBackend::Polling(backend) => backend.poll_changed_paths(),
            FileMonitorBackend::Linux(_) => Vec::new(),
        }
    }

    /// Take one pending backend fallback warning, if any.
    pub(crate) fn take_warning(&mut self) -> Option<String> {
        self.pending_warning.take()
    }

    /// Replace the active backend with metadata polling after a Linux watcher failure.
    fn fallback_to_polling(&mut self, paths: &[PathBuf]) {
        // Polling preserves functional correctness when a native watcher cannot
        // be initialized or maintained, even though notifications become periodic.
        let mut backend = PollingFileMonitor::default();
        backend.sync_paths(paths);
        self.backend = FileMonitorBackend::Polling(backend);
        self.pending_warning = Some("File watcher unavailable; using metadata polling".to_string());
    }
}

/// Linux inotify-backed backend for native path change notifications.
#[derive(Debug)]
struct LinuxFileMonitor {
    inotify: LinuxInotify,
    tracked_paths: HashSet<PathBuf>,
    watched_dirs: HashMap<PathBuf, WatchedDirectory>,
    watch_descriptors: HashMap<i32, PathBuf>,
}

#[derive(Debug)]
struct WatchedDirectory {
    watch_descriptor: i32,
    file_names: HashSet<OsString>,
}

impl LinuxFileMonitor {
    /// Create one Linux inotify backend with an initialized nonblocking fd.
    fn new() -> io::Result<Self> {
        Ok(Self {
            inotify: LinuxInotify::new()?,
            tracked_paths: HashSet::new(),
            watched_dirs: HashMap::new(),
            watch_descriptors: HashMap::new(),
        })
    }

    /// Synchronize the directory watches with the currently open named file paths.
    fn sync_paths(&mut self, paths: &[PathBuf]) -> io::Result<()> {
        let desired_paths = paths.iter().cloned().collect::<HashSet<_>>();
        let desired_dirs = desired_directory_map(&desired_paths);
        let removed_dirs = self
            .watched_dirs
            .keys()
            .filter(|dir| !desired_dirs.contains_key(*dir))
            .cloned()
            .collect::<Vec<_>>();

        // Remove obsolete watches first so subsequent add-watch calls can reuse
        // kernel watch slots without leaking stale directory registrations.
        for dir in removed_dirs {
            self.remove_directory_watch(&dir);
        }

        self.tracked_paths = desired_paths;
        for (dir, file_names) in desired_dirs {
            self.ensure_directory_watch(&dir)?;
            if let Some(watched) = self.watched_dirs.get_mut(&dir) {
                watched.file_names = file_names;
            }
        }

        Ok(())
    }

    /// Return the set of tracked paths, used when falling back to polling.
    fn tracked_paths(&self) -> Vec<PathBuf> {
        self.tracked_paths.iter().cloned().collect()
    }

    /// Return the paths whose directories produced matching inotify events.
    fn poll_changed_paths(&mut self) -> io::Result<Vec<PathBuf>> {
        if self.tracked_paths.is_empty() || !self.inotify.poll_ready()? {
            return Ok(Vec::new());
        }

        let mut changed = HashSet::new();
        let events = self.inotify.read_events()?;

        for event in events {
            // Queue overflow means we lost precision, so every tracked file must
            // be rechecked against disk to rebuild a coherent external-change picture.
            if event_is_queue_overflow(event.mask) {
                return Ok(self.tracked_paths());
            }

            let Some(directory) = self.watch_descriptors.get(&event.watch_descriptor).cloned()
            else {
                continue;
            };

            if event_invalidates_directory_watch(event.mask) {
                changed.extend(self.paths_for_directory(&directory));
                self.remove_directory_watch(&directory);
                continue;
            }

            if let Some(name) = event.name {
                let path = directory.join(name);
                if self.tracked_paths.contains(&path) {
                    changed.insert(path);
                }
            }
        }

        Ok(changed.into_iter().collect())
    }

    /// Ensure the directory watch for `dir` exists before the next poll.
    fn ensure_directory_watch(&mut self, dir: &Path) -> io::Result<()> {
        if self.watched_dirs.contains_key(dir) || !dir.exists() {
            return Ok(());
        }

        // Parent-directory watches stay valid across atomic replace writes and
        // surface create/delete/close-write events for the watched filename.
        let watch_descriptor = self.inotify.add_directory_watch(dir)?;
        let dir = dir.to_path_buf();
        self.watch_descriptors.insert(watch_descriptor, dir.clone());
        self.watched_dirs.insert(
            dir,
            WatchedDirectory {
                watch_descriptor,
                file_names: HashSet::new(),
            },
        );
        Ok(())
    }

    /// Remove one directory watch and forget its reverse lookup entry.
    fn remove_directory_watch(&mut self, dir: &Path) {
        let Some(watched) = self.watched_dirs.remove(dir) else {
            return;
        };

        let _ = self.inotify.remove_watch(watched.watch_descriptor);
        self.watch_descriptors.remove(&watched.watch_descriptor);
    }

    /// Return the tracked file paths that currently belong to `dir`.
    fn paths_for_directory(&self, dir: &Path) -> Vec<PathBuf> {
        let Some(watched) = self.watched_dirs.get(dir) else {
            return Vec::new();
        };
        watched
            .file_names
            .iter()
            .map(|name| dir.join(name))
            .collect()
    }
}

/// Return whether `mask` reports an inotify queue overflow.
fn event_is_queue_overflow(mask: u32) -> bool {
    mask_has_any(mask, libc::IN_Q_OVERFLOW)
}

/// Return whether `mask` invalidates the current directory watch.
fn event_invalidates_directory_watch(mask: u32) -> bool {
    mask_has_any(
        mask,
        libc::IN_IGNORED | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF,
    )
}

/// Return whether `mask` contains any bit from `flags`.
fn mask_has_any(mask: u32, flags: u32) -> bool {
    mask & flags != 0
}

/// Group the tracked file paths by parent directory for inotify directory watches.
fn desired_directory_map(paths: &HashSet<PathBuf>) -> HashMap<PathBuf, HashSet<OsString>> {
    let mut grouped = HashMap::new();

    // Directory watches let Ordex survive atomic rename saves because the path,
    // not the original inode, remains the thing we care about across changes.
    for path in paths {
        let Some(parent) = path.parent() else {
            continue;
        };
        let Some(name) = path.file_name() else {
            continue;
        };
        grouped
            .entry(parent.to_path_buf())
            .or_insert_with(HashSet::new)
            .insert(name.to_os_string());
    }

    grouped
}
