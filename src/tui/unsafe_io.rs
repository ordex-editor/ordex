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
