//! External file-change tracking and monitoring helpers for editor buffers.

use crate::text_buffer::TextBuffer;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "linux")]
mod linux_inotify;

#[cfg(target_os = "linux")]
use linux_inotify::LinuxInotify;

const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// One stable fingerprint for either on-disk file contents or a missing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileFingerprint {
    Missing,
    Present(ContentFingerprint),
}

/// Content-only fingerprint used to compare current and synced file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentFingerprint {
    pub(crate) bytes: u64,
    pub(crate) hash: u64,
}

/// One pending external change that differs from the last synced disk state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingExternalChange {
    pub(crate) fingerprint: FileFingerprint,
    pub(crate) generation: u64,
    pub(crate) ignored: bool,
}

/// Per-buffer external file tracking state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ExternalFileState {
    pub(crate) synced: Option<FileFingerprint>,
    pub(crate) pending_change: Option<PendingExternalChange>,
    pub(crate) deferred_notice: Option<String>,
}

impl ExternalFileState {
    /// Replace the synced disk baseline with the current loaded buffer contents.
    pub(crate) fn sync_to_loaded_buffer(&mut self, buffer: &TextBuffer) {
        self.synced = Some(fingerprint_buffer_contents(buffer));
        self.pending_change = None;
        self.deferred_notice = None;
    }

    /// Replace the synced disk baseline with the current save-to-disk buffer contents.
    pub(crate) fn sync_to_saved_buffer(&mut self, buffer: &TextBuffer) {
        self.synced = Some(fingerprint_buffer_save_contents(buffer));
        self.pending_change = None;
        self.deferred_notice = None;
    }

    /// Replace the synced disk baseline with one missing-file snapshot.
    pub(crate) fn sync_to_missing_file(&mut self) {
        self.synced = Some(FileFingerprint::Missing);
        self.pending_change = None;
        self.deferred_notice = None;
    }

    /// Return whether the current buffer should show an external-change prompt.
    ///
    /// Returns `true` when an unresolved external change exists for this buffer,
    /// and `false` when there is no pending change or the user already ignored it.
    pub(crate) fn prompt_is_active(&self) -> bool {
        self.pending_change
            .as_ref()
            .is_some_and(|change| !change.ignored)
    }

    /// Record that the user explicitly ignored the currently pending change.
    pub(crate) fn mark_change_ignored(&mut self) {
        if let Some(change) = self.pending_change.as_mut() {
            change.ignored = true;
        }
    }

    /// Consume any deferred user-facing notice queued while the buffer was hidden.
    pub(crate) fn take_deferred_notice(&mut self) -> Option<String> {
        self.deferred_notice.take()
    }

    /// Update the pending external change for one newly observed disk fingerprint.
    pub(crate) fn update_pending_change(
        &mut self,
        fingerprint: FileFingerprint,
        next_generation: &mut u64,
    ) {
        // When the file matches the synced baseline again, the conflict is gone.
        if self
            .synced
            .as_ref()
            .is_some_and(|synced| synced == &fingerprint)
        {
            self.pending_change = None;
            return;
        }

        // Repeated notifications for the same on-disk contents keep the existing
        // ignored/visible disposition instead of re-prompting the user.
        if self
            .pending_change
            .as_ref()
            .is_some_and(|change| change.fingerprint == fingerprint)
        {
            return;
        }

        self.pending_change = Some(PendingExternalChange {
            fingerprint,
            generation: *next_generation,
            ignored: false,
        });
        *next_generation += 1;
    }
}

/// Session-wide file monitor that prefers native Linux notifications and falls back to polling.
#[derive(Debug)]
pub(crate) struct FileMonitor {
    backend: FileMonitorBackend,
    pending_warning: Option<String>,
}

#[derive(Debug)]
enum FileMonitorBackend {
    #[cfg(target_os = "linux")]
    Linux(LinuxFileMonitor),
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
        #[cfg(target_os = "linux")]
        {
            match LinuxFileMonitor::new() {
                Ok(backend) => Self {
                    backend: FileMonitorBackend::Linux(backend),
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

        #[cfg(not(target_os = "linux"))]
        {
            Self {
                backend: FileMonitorBackend::Polling(PollingFileMonitor::default()),
                pending_warning: None,
            }
        }
    }

    /// Synchronize the monitored path set with the currently open named buffers.
    pub(crate) fn sync_paths(&mut self, paths: &[PathBuf]) {
        #[cfg(target_os = "linux")]
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
        #[cfg(target_os = "linux")]
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
            #[cfg(target_os = "linux")]
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

/// Compute one content fingerprint for the current in-memory buffer contents.
pub(crate) fn fingerprint_buffer_contents(buffer: &TextBuffer) -> FileFingerprint {
    let mut hasher = DefaultHasher::new();

    // The rope already stores the exact bytes loaded from disk, so hashing its
    // contiguous chunks preserves the current file identity without allocation.
    for chunk in buffer.chunks() {
        hasher.write(chunk.as_bytes());
    }

    FileFingerprint::Present(ContentFingerprint {
        bytes: buffer.bytes_count() as u64,
        hash: hasher.finish(),
    })
}

/// Compute one content fingerprint for the exact bytes Ordex writes on save.
pub(crate) fn fingerprint_buffer_save_contents(buffer: &TextBuffer) -> FileFingerprint {
    let rope = buffer.clone_rope_for_save();
    let mut hasher = DefaultHasher::new();

    // Save operations append a trailing newline when needed, so this fingerprint
    // must hash the save-policy rope rather than the live in-memory text buffer.
    for chunk in rope.chunks() {
        hasher.write(chunk.as_bytes());
    }

    FileFingerprint::Present(ContentFingerprint {
        bytes: rope.len() as u64,
        hash: hasher.finish(),
    })
}

/// Read the current file bytes from disk and return their comparison fingerprint.
pub(crate) fn read_fingerprint_from_disk(path: &Path) -> io::Result<FileFingerprint> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FileFingerprint::Missing);
        }
        Err(error) => return Err(error),
    };
    let mut hasher = DefaultHasher::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 8192];

    // Stream the file in fixed-size chunks so large buffers do not require
    // reading the whole file into memory just to compare against the baseline.
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
        bytes += read as u64;
    }

    Ok(FileFingerprint::Present(ContentFingerprint {
        bytes,
        hash: hasher.finish(),
    }))
}

/// Lightweight metadata signature used by the polling fallback to notice candidate changes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MetadataFingerprint {
    Missing,
    Present {
        modified: Option<SystemTime>,
        len: u64,
    },
}

/// Metadata-polling backend for platforms without a native watcher in this implementation.
#[derive(Debug, Default)]
struct PollingFileMonitor {
    tracked: HashMap<PathBuf, MetadataFingerprint>,
    next_poll_at: Option<Instant>,
}

impl PollingFileMonitor {
    /// Synchronize the tracked metadata snapshot set with the current open file paths.
    fn sync_paths(&mut self, paths: &[PathBuf]) {
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
    fn poll_changed_paths(&mut self) -> Vec<PathBuf> {
        if self
            .next_poll_at
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return Vec::new();
        }

        let mut changed = Vec::new();
        let paths = self.tracked.keys().cloned().collect::<Vec<_>>();

        // Polling is only a candidate detector; the caller still re-reads the
        // file contents before deciding whether the buffer truly diverged.
        for path in paths {
            let current = metadata_fingerprint(&path);
            let Some(previous) = self.tracked.get_mut(&path) else {
                continue;
            };
            if *previous != current {
                *previous = current;
                changed.push(path);
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
        Err(error) if error.kind() == io::ErrorKind::NotFound => MetadataFingerprint::Missing,
        Err(_) => MetadataFingerprint::Missing,
    }
}

#[cfg(target_os = "linux")]
/// Linux inotify-backed backend for native path change notifications.
#[derive(Debug)]
struct LinuxFileMonitor {
    inotify: LinuxInotify,
    tracked_paths: HashSet<PathBuf>,
    watched_dirs: HashMap<PathBuf, WatchedDirectory>,
    watch_lookup: HashMap<i32, PathBuf>,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct WatchedDirectory {
    wd: i32,
    file_names: HashSet<std::ffi::OsString>,
}

#[cfg(target_os = "linux")]
impl LinuxFileMonitor {
    /// Create one Linux inotify backend with an initialized nonblocking fd.
    fn new() -> io::Result<Self> {
        Ok(Self {
            inotify: LinuxInotify::new()?,
            tracked_paths: HashSet::new(),
            watched_dirs: HashMap::new(),
            watch_lookup: HashMap::new(),
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

        // Queue overflow means we lost precision, so every tracked file must be
        // rechecked against disk to rebuild a coherent external-change picture.
        for event in events {
            if event.mask & libc::IN_Q_OVERFLOW != 0 {
                return Ok(self.tracked_paths());
            }

            let Some(directory) = self.watch_lookup.get(&event.wd).cloned() else {
                continue;
            };

            if event.mask & (libc::IN_IGNORED | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF) != 0 {
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
        let wd = self.inotify.add_directory_watch(dir)?;
        let dir = dir.to_path_buf();
        self.watch_lookup.insert(wd, dir.clone());
        self.watched_dirs.insert(
            dir,
            WatchedDirectory {
                wd,
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

        let _ = self.inotify.remove_watch(watched.wd);
        self.watch_lookup.remove(&watched.wd);
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

#[cfg(target_os = "linux")]
/// Group the tracked file paths by parent directory for inotify directory watches.
fn desired_directory_map(
    paths: &HashSet<PathBuf>,
) -> HashMap<PathBuf, HashSet<std::ffi::OsString>> {
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
