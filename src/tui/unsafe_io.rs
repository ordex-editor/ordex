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
