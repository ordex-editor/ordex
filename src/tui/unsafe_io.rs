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
    PtyPair, redirect_stdin_to_fd, restore_stdin, set_raw_mode_fd, write_byte_to_fd,
};

#[cfg(test)]
mod test_helpers {
    use std::io;
    use std::os::fd::RawFd;

    /// A PTY master/slave pair opened for use in unit tests.
    ///
    /// The master side is used by the test to write input bytes; the slave side
    /// is redirected onto fd 0 so `stdin()` calls inside the parser read from it.
    pub(crate) struct PtyPair {
        /// File descriptor for the master side of the PTY.
        pub(crate) master: RawFd,
        /// File descriptor for the slave side of the PTY.
        pub(crate) slave: RawFd,
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
            // valid sentinel values accepted by `openpty`.
            let rc = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null::<libc::termios>() as _,
                    &mut winsize,
                )
            };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { master, slave })
        }
    }

    impl Drop for PtyPair {
        /// Close both file descriptors when the pair is dropped.
        fn drop(&mut self) {
            // SAFETY: `master` and `slave` are valid open file descriptors
            // created by `openpty`.  Closing them here is safe because `PtyPair`
            // is the sole owner and `drop` runs exactly once.
            unsafe {
                libc::close(self.master);
                libc::close(self.slave);
            }
        }
    }

    /// Write one byte to any raw file descriptor.
    ///
    /// Used in tests to send bytes to the master side of a PTY pair.
    pub(crate) fn write_byte_to_fd(fd: RawFd, byte: u8) -> io::Result<()> {
        let b = [byte];
        // SAFETY: `fd` must be a valid open file descriptor; `b` is a valid
        // 1-byte buffer whose lifetime covers this call.
        let n = unsafe { libc::write(fd, b.as_ptr().cast(), 1) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Put `fd` into raw (non-canonical, no-echo) terminal mode.
    ///
    /// In the default canonical mode the PTY line discipline buffers input
    /// until a newline, so individual bytes written to the master do not reach
    /// the slave reader until a newline is sent.  Raw mode disables that
    /// buffering so every byte is delivered immediately, which is required for
    /// the escape-sequence parser tests.
    pub(crate) fn set_raw_mode_fd(fd: RawFd) -> io::Result<()> {
        // SAFETY: `tcgetattr` fills the supplied `termios` struct from the live
        // terminal associated with `fd`; `cfmakeraw` only mutates local memory;
        // `tcsetattr` with `TCSANOW` applies the settings immediately.  All
        // three calls are safe as long as `fd` is a valid terminal fd, which
        // is guaranteed by the caller (`PtyPair::open` creates it with `openpty`).
        unsafe {
            let mut termios: libc::termios = std::mem::zeroed();
            let rc = libc::tcgetattr(fd, &mut termios);
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut termios);
            let rc = libc::tcsetattr(fd, libc::TCSANOW, &termios);
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Replace fd 0 (stdin) with `new_fd` and return the saved original fd 0.
    ///
    /// The caller must restore the original fd 0 via `restore_stdin` before
    /// the test exits to avoid corrupting the process stdin for other tests.
    pub(crate) fn redirect_stdin_to_fd(new_fd: RawFd) -> io::Result<RawFd> {
        // SAFETY: `dup(0)` duplicates the current stdin descriptor; the returned
        // fd is a valid independent handle to the same underlying file.
        let saved = unsafe { libc::dup(0) };
        if saved < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `dup2(new_fd, 0)` atomically replaces fd 0 with a duplicate of
        // `new_fd`.  Both descriptors are valid open fds; the old fd 0 content is
        // preserved in `saved`.
        let rc = unsafe { libc::dup2(new_fd, 0) };
        if rc < 0 {
            unsafe { libc::close(saved) };
            return Err(io::Error::last_os_error());
        }
        Ok(saved)
    }

    /// Restore fd 0 (stdin) from `saved_fd` and close `saved_fd`.
    ///
    /// This undoes the redirection performed by `redirect_stdin_to_fd`.
    pub(crate) fn restore_stdin(saved_fd: RawFd) -> io::Result<()> {
        // SAFETY: `dup2(saved_fd, 0)` restores the original stdin; `close(saved_fd)`
        // releases the temporary duplicate.  Both descriptors are valid fds opened
        // earlier in the same test.
        let rc = unsafe { libc::dup2(saved_fd, 0) };
        unsafe { libc::close(saved_fd) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
