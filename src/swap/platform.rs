//! Platform-specific swap helpers.

use std::io;

/// Return the current hostname using libc.
pub(crate) fn current_hostname() -> io::Result<String> {
    let mut bytes = [0_u8; 256];
    // SAFETY: `bytes` is a valid writable buffer for the full reported length,
    // and `gethostname` writes at most that many bytes into the provided pointer.
    let rc = unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let len = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    Ok(String::from_utf8_lossy(&bytes[..len]).into_owned())
}
