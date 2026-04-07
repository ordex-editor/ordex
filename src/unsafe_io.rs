//! Shared low-level I/O helpers that wrap raw file descriptors safely.

use std::io;
use std::os::fd::{AsFd, AsRawFd};

/// One raw `poll` result for a single file descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PollOutcome {
    /// Whether `poll` reported at least one ready descriptor before the timeout.
    pub(crate) ready: bool,
    /// Raw readiness flags written by `poll`.
    pub(crate) revents: i16,
}

/// Poll one borrowed file descriptor and return the readiness summary.
pub(crate) fn poll_fd(fd: &impl AsFd, timeout_ms: i32) -> io::Result<PollOutcome> {
    let mut poll_fd = libc::pollfd {
        fd: fd.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `fd.as_fd()` yields a borrowed descriptor tied to the caller's
    // live handle, the pollfd slice length is exactly one element, and libc
    // only writes readiness bits back into the supplied `pollfd` struct.
    let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
    if ready < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PollOutcome {
        ready: ready > 0,
        revents: poll_fd.revents,
    })
}
