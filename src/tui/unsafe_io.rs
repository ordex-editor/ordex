use std::io::{self, Stdin};
use std::os::fd::AsRawFd;

/// Read a single byte from stdin using a raw libc call.
///
/// This bypasses higher-level buffering so we can interpret escape sequences
/// and standalone `Esc` promptly, which is essential for responsive key handling
/// over SSH/tmux where bytes can arrive with jitter. Use this when the TUI needs
/// precise, byte-by-byte control rather than line-buffered input.
pub(crate) fn read_byte(stdin: &Stdin) -> io::Result<u8> {
    let fd = stdin.as_raw_fd();
    let mut buf = [0_u8; 1];
    // SAFETY: `buf` is a valid 1-byte buffer, and `fd` is obtained from a live
    // `Stdin` handle, so it refers to the process stdin for the duration of this call.
    let read_result = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), 1) };
    match read_result {
        1 => Ok(buf[0]),
        0 => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stdin key stream ended",
        )),
        n if n < 0 => Err(io::Error::last_os_error()),
        _ => unreachable!("single-byte read returned unexpected length"),
    }
}

/// Attempt to read one byte from stdin without blocking.
///
/// Returns `Some(byte)` when a byte was immediately available, and `None` when
/// no data is ready.  Errors other than `EAGAIN`/`EWOULDBLOCK` are propagated.
///
/// On some platforms (notably macOS PTY slaves) `poll` can report `POLLIN` even
/// when no data is present, causing a subsequent blocking `read` to stall.
/// Callers that already verified readiness via `poll` should use this function
/// instead of `read_byte` so a spurious wakeup is treated as "no data" rather
/// than an indefinite block.
pub(crate) fn try_read_byte(stdin: &Stdin) -> io::Result<Option<u8>> {
    let fd = stdin.as_raw_fd();

    // Temporarily enable O_NONBLOCK so the read returns EAGAIN instead of
    // blocking when the `poll` wakeup was spurious (macOS PTY slave bug).
    // SAFETY: `fcntl` with F_GETFL/F_SETFL only reads or writes integer flags
    // and does not alias any Rust references.
    let old_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old_flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let set_result = unsafe { libc::fcntl(fd, libc::F_SETFL, old_flags | libc::O_NONBLOCK) };
    if set_result < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buf = [0_u8; 1];
    // SAFETY: same as `read_byte` above.
    let read_result = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), 1) };

    // Restore the original blocking mode before returning or propagating errors.
    unsafe { libc::fcntl(fd, libc::F_SETFL, old_flags) };

    match read_result {
        1 => Ok(Some(buf[0])),
        0 => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stdin key stream ended",
        )),
        n if n < 0 => {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                Ok(None)
            } else {
                Err(err)
            }
        }
        _ => unreachable!("single-byte read returned unexpected length"),
    }
}

#[cfg(test)]
pub(crate) use test_helpers::{
    PtyPair, StdinGuard, redirect_stdin_to_fd, set_raw_mode_fd, write_byte_to_fd,
};

#[cfg(test)]
mod test_helpers {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

    /// A PTY master/slave pair opened for use in unit tests.
    ///
    /// The master side is used by the test to write input bytes; the slave side
    /// is redirected onto fd 0 so `stdin()` calls inside the parser read from it.
    /// Both file descriptors are owned and closed automatically when this struct drops.
    pub(crate) struct PtyPair {
        /// Owned file descriptor for the master side of the PTY.
        pub(crate) master: OwnedFd,
        /// Owned file descriptor for the slave side of the PTY.
        pub(crate) slave: OwnedFd,
    }

    impl PtyPair {
        /// Open a new PTY pair and return master and slave file descriptors.
        pub(crate) fn open() -> io::Result<Self> {
            let mut master: RawFd = -1;
            let mut slave: RawFd = -1;
            let mut winsize = libc::winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: `openpty` writes two valid file descriptors into `master`
            // and `slave`; the winsize pointer refers to a local stack variable
            // that outlives the call; null pointers for name and termios are
            // valid sentinel values accepted by `openpty`.  The resulting raw fds
            // are immediately wrapped in `OwnedFd`, transferring ownership to Rust.
            let rc = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null::<libc::termios>() as _,
                    &mut winsize as _,
                )
            };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `openpty` succeeded and wrote valid, uniquely-owned file
            // descriptors into `master` and `slave`.
            Ok(Self {
                master: unsafe { OwnedFd::from_raw_fd(master) },
                slave: unsafe { OwnedFd::from_raw_fd(slave) },
            })
        }
    }

    /// Write one byte to a raw file descriptor.
    ///
    /// Used in tests to send bytes to the master side of a PTY pair.  The
    /// caller is responsible for ensuring `fd` is a valid open file descriptor
    /// for the duration of the call.
    pub(crate) fn write_byte_to_fd(fd: RawFd, byte: u8) -> io::Result<()> {
        let b = [byte];
        // SAFETY: `fd` must be a valid open file descriptor supplied by the
        // caller; `b` is a valid 1-byte buffer whose lifetime covers this call.
        let n = unsafe { libc::write(fd, b.as_ptr().cast(), 1) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Put the terminal associated with `fd` into raw (non-canonical, no-echo) mode.
    ///
    /// In the default canonical mode the PTY line discipline buffers input
    /// until a newline, so individual bytes written to the master do not reach
    /// the slave reader until a newline is sent.  Raw mode disables that
    /// buffering so every byte is delivered immediately, which is required for
    /// the escape-sequence parser tests.
    pub(crate) fn set_raw_mode_fd(fd: &OwnedFd) -> io::Result<()> {
        // SAFETY: `tcgetattr` fills the supplied `termios` struct from the live
        // terminal associated with `fd`; `cfmakeraw` only mutates local memory;
        // `tcsetattr` with `TCSANOW` applies the settings immediately.  All
        // three calls are safe as long as `fd` is a valid terminal fd, which
        // is guaranteed by the caller (`PtyPair::open` creates it with `openpty`).
        unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            let rc = libc::tcgetattr(fd.as_raw_fd(), &mut termios);
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut termios);
            let rc = libc::tcsetattr(fd.as_raw_fd(), libc::TCSANOW, &termios);
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// RAII guard that restores fd 0 (stdin) to a saved original when dropped.
    ///
    /// Construct via `redirect_stdin_to_fd`, which saves the current fd 0,
    /// replaces it with a PTY slave, and returns this guard.  When the guard
    /// drops, whether on normal exit or on a test panic, the original fd 0
    /// is restored via `dup2` and the saved duplicate is closed by dropping the
    /// inner `OwnedFd`.
    pub(crate) struct StdinGuard {
        /// Duplicate of the original fd 0 saved before the redirect.
        saved: OwnedFd,
    }

    impl Drop for StdinGuard {
        /// Restore fd 0 from the saved duplicate.
        fn drop(&mut self) {
            // SAFETY: `dup2(saved, 0)` atomically restores the original stdin.
            // `saved` is a valid open fd created by `dup` in `redirect_stdin_to_fd`.
            // Ignoring the return value here is intentional: `drop` cannot
            // return an error, and a failed restore during unwinding would mask
            // the original panic rather than fix anything.
            unsafe { libc::dup2(self.saved.as_raw_fd(), 0) };
            // `self.saved` (OwnedFd) is closed automatically when this scope ends.
        }
    }

    /// Replace fd 0 (stdin) with `new_fd` and return a guard that restores it.
    ///
    /// The returned `StdinGuard` calls `dup2` to put the original fd 0 back when
    /// it drops, ensuring restoration on both normal return and test panics.
    pub(crate) fn redirect_stdin_to_fd(new_fd: &OwnedFd) -> io::Result<StdinGuard> {
        // SAFETY: `dup(0)` duplicates the current stdin descriptor into a new
        // fd.  Wrapping the result in `OwnedFd` transfers ownership to Rust so
        // it is closed exactly once when `StdinGuard` drops.
        let saved_raw = unsafe { libc::dup(0) };
        if saved_raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `dup` succeeded and returned a valid, uniquely-owned fd.
        let saved = unsafe { OwnedFd::from_raw_fd(saved_raw) };

        // SAFETY: `dup2(new_fd, 0)` atomically replaces fd 0 with a duplicate
        // of `new_fd`.  Both descriptors are valid open fds; the old fd 0 is
        // preserved in `saved`.
        let rc = unsafe { libc::dup2(new_fd.as_raw_fd(), 0) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
            // `saved` is dropped here, closing the duplicate automatically.
        }
        Ok(StdinGuard { saved })
    }
}
