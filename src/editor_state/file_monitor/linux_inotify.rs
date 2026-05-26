//! Linux inotify wrapper used by the external file monitor backend.

use crate::unsafe_io::poll_fd;
use std::ffi::{CString, OsString};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

const READ_BUFFER_LEN: usize = 16 * 1024;

/// One parsed inotify event emitted by the kernel.
#[derive(Debug)]
pub(super) struct InotifyEvent {
    pub(super) watch_descriptor: i32,
    pub(super) mask: u32,
    pub(super) name: Option<OsString>,
}

/// Nonblocking inotify instance used by the Linux file monitor backend.
#[derive(Debug)]
pub(super) struct LinuxInotify {
    fd: OwnedFd,
    read_buffer: [u8; READ_BUFFER_LEN],
}

impl LinuxInotify {
    /// Create one nonblocking inotify instance with close-on-exec enabled.
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: `inotify_init1` has no Rust aliasing requirements and returns a
        // new owned file descriptor on success for the requested flag bitmask.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: `fd` is freshly returned by `inotify_init1`, is owned by this
        // function on success, and is not wrapped by any other file-descriptor type.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            fd,
            read_buffer: [0; READ_BUFFER_LEN],
        })
    }

    /// Add one parent-directory watch that reports writes, renames, deletes, and metadata changes.
    pub(super) fn add_directory_watch(&self, path: &Path) -> io::Result<i32> {
        let raw_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "directory path contains an interior NUL byte: {}",
                    path.display()
                ),
            )
        })?;
        let mask = libc::IN_ATTRIB
            | libc::IN_CLOSE_WRITE
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_MOVED_FROM
            | libc::IN_MOVED_TO
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF
            | libc::IN_ONLYDIR;

        // SAFETY: `self.fd` is a live inotify descriptor, `raw_path` is a
        // NUL-terminated C string, and the kernel only reads the provided bytes.
        let watch_descriptor = unsafe {
            libc::inotify_add_watch(self.fd.as_fd().as_raw_fd(), raw_path.as_ptr(), mask)
        };
        if watch_descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(watch_descriptor)
    }

    /// Remove one previously registered watch descriptor.
    pub(super) fn remove_watch(&self, watch_descriptor: i32) -> io::Result<()> {
        // SAFETY: `self.fd` is a live inotify descriptor and `watch_descriptor` came from a
        // prior successful `inotify_add_watch` call or an equivalent kernel event.
        let rc = unsafe { libc::inotify_rm_watch(self.fd.as_fd().as_raw_fd(), watch_descriptor) };
        if rc == 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::EINVAL)) {
            // `EINVAL` means the kernel no longer recognizes this watch descriptor,
            // which happens after self-delete or ignored-watch races where removal
            // is already complete by the time Ordex asks to drop it explicitly.
            return Ok(());
        }
        Err(error)
    }

    /// Return whether the inotify fd has readable events queued right now.
    pub(super) fn poll_ready(&self) -> io::Result<bool> {
        let outcome = poll_fd(self, 0)?;
        Ok(outcome.ready && (outcome.revents & libc::POLLIN) != 0)
    }

    /// Read and parse every currently queued inotify event without blocking.
    pub(super) fn read_events(&mut self) -> io::Result<Vec<InotifyEvent>> {
        let mut all_events = Vec::new();

        loop {
            // SAFETY: `self.read_buffer` is a valid writable byte slice for `read`,
            // and the inotify fd writes at most `self.read_buffer.len()` bytes into it.
            let read = unsafe {
                libc::read(
                    self.fd.as_fd().as_raw_fd(),
                    self.read_buffer.as_mut_ptr().cast(),
                    self.read_buffer.len(),
                )
            };

            if read == 0 {
                break;
            }
            if read < 0 {
                let error = io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(libc::EAGAIN)) {
                    break;
                }
                return Err(error);
            }

            // The kernel packs one or more variable-length records into the read
            // buffer, so parsing walks the byte slice record-by-record.
            parse_inotify_records(&self.read_buffer[..read as usize], &mut all_events);
        }

        Ok(all_events)
    }
}

impl AsFd for LinuxInotify {
    /// Borrow the underlying inotify file descriptor for readiness polling.
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Parse one buffer of raw inotify records and append them to `events`.
fn parse_inotify_records(buffer: &[u8], events: &mut Vec<InotifyEvent>) {
    let mut offset = 0;

    // Each record starts with a fixed header followed by `len` bytes of an
    // optional NUL-padded filename payload.
    while offset + std::mem::size_of::<libc::inotify_event>() <= buffer.len() {
        // SAFETY: the bounds check above guarantees the header bytes are present,
        // and `read_unaligned` tolerates the packed C layout inside the byte slice.
        let event = unsafe {
            std::ptr::read_unaligned(buffer[offset..].as_ptr().cast::<libc::inotify_event>())
        };
        let record_len = std::mem::size_of::<libc::inotify_event>() + event.len as usize;
        if offset + record_len > buffer.len() {
            break;
        }

        let name = if event.len == 0 {
            None
        } else {
            let bytes =
                &buffer[offset + std::mem::size_of::<libc::inotify_event>()..offset + record_len];
            let trimmed = bytes
                .iter()
                .position(|byte| *byte == 0)
                .map(|len| &bytes[..len])
                .unwrap_or(bytes);
            Some(OsString::from_vec(trimmed.to_vec()))
        };
        events.push(InotifyEvent {
            watch_descriptor: event.wd,
            mask: event.mask,
            name,
        });
        offset += record_len;
    }
}
