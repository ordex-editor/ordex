//! Common external-file fingerprints and per-buffer tracking state.

use crate::text_buffer::TextBuffer;
use std::fs::File;
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, BufReader, Read};
use std::path::Path;

/// One stable fingerprint for either on-disk file contents or a missing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileFingerprint {
    Missing,
    Present(ContentFingerprint),
}

/// Content-only fingerprint used to compare current and synced file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentFingerprint {
    /// Total byte length of the compared file contents.
    pub(crate) byte_len: u64,
    /// Stable hash of the compared file contents.
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

/// Compute one content fingerprint for the current in-memory buffer contents.
pub(crate) fn fingerprint_buffer_contents(buffer: &TextBuffer) -> FileFingerprint {
    fingerprint_text_chunks(buffer.chunks(), buffer.bytes_count() as u64)
}

/// Compute one content fingerprint for the exact bytes Ordex writes on save.
pub(crate) fn fingerprint_buffer_save_contents(buffer: &TextBuffer) -> FileFingerprint {
    let rope = buffer.clone_rope_for_save();

    // Save operations append a trailing newline when needed, so this fingerprint
    // must hash the save-policy rope rather than the live in-memory text buffer.
    fingerprint_text_chunks(rope.chunks(), rope.len() as u64)
}

/// Read the current file bytes from disk and return their comparison fingerprint.
pub(crate) fn read_fingerprint_from_disk(path: &Path) -> io::Result<FileFingerprint> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FileFingerprint::Missing);
        }
        Err(error) => return Err(error),
    };
    let mut reader = BufReader::new(file);
    let mut hasher = DefaultHasher::new();
    let mut byte_len = 0_u64;
    let mut buffer = [0_u8; 8192];

    // Stream the file in fixed-size chunks so large buffers do not require
    // reading the whole file into memory just to compare against the baseline.
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
        byte_len += read as u64;
    }

    Ok(FileFingerprint::Present(ContentFingerprint {
        byte_len,
        hash: hasher.finish(),
    }))
}

/// Hash one iterator of text chunks using the already known total byte length.
fn fingerprint_text_chunks<'a>(
    chunks: impl Iterator<Item = &'a str>,
    byte_len: u64,
) -> FileFingerprint {
    let mut hasher = DefaultHasher::new();

    // The rope already stores the exact bytes loaded from disk or queued for
    // save, so hashing its contiguous chunks preserves file identity without
    // assembling one large temporary string first.
    for chunk in chunks {
        hasher.write(chunk.as_bytes());
    }

    FileFingerprint::Present(ContentFingerprint {
        byte_len,
        hash: hasher.finish(),
    })
}
