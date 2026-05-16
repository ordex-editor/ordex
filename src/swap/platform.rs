//! Platform-specific swap helpers.

use std::io;

/// Return the current hostname for the running machine.
#[cfg(unix)]
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

/// Return the current hostname for the running machine.
#[cfg(windows)]
pub(crate) fn current_hostname() -> io::Result<String> {
    std::env::var("COMPUTERNAME").map_err(|error| io::Error::new(io::ErrorKind::NotFound, error))
}

/// Return whether `pid` currently refers to a live process on this host.
///
/// Returns `true` when the operating system reports the process still exists,
/// and `false` when it definitively does not exist anymore.
#[cfg(unix)]
pub(crate) fn process_is_running(pid: u32) -> io::Result<bool> {
    if pid == 0 {
        return Ok(false);
    }
    if pid > libc::pid_t::MAX as u32 {
        return Ok(false);
    }
    // SAFETY: `kill(pid, 0)` does not signal the process. It only asks the
    // kernel to validate whether the process exists and whether this process
    // may signal it, using the provided integer pid value.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(false),
        Some(code) if code == libc::EPERM => Ok(true),
        _ => Err(error),
    }
}

#[cfg(windows)]
type Handle = *mut std::ffi::c_void;

#[cfg(windows)]
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

#[cfg(windows)]
const SYNCHRONIZE: u32 = 0x0010_0000;

#[cfg(windows)]
const STILL_ACTIVE: u32 = 259;

#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;

#[cfg(windows)]
const ERROR_INVALID_PARAMETER: i32 = 87;

#[cfg(windows)]
unsafe extern "system" {
    fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
    fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

/// Return whether `pid` currently refers to a live process on this host.
///
/// Returns `true` when the operating system reports the process still exists,
/// and `false` when it definitively does not exist anymore.
#[cfg(windows)]
pub(crate) fn process_is_running(pid: u32) -> io::Result<bool> {
    if pid == 0 {
        return Ok(false);
    }
    // SAFETY: The Windows API accepts any integer pid here. A null handle
    // indicates failure, and successful handles are always released below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(ERROR_INVALID_PARAMETER) => Ok(false),
            Some(ERROR_ACCESS_DENIED) => Ok(true),
            _ => Err(error),
        };
    }

    let mut exit_code = 0;
    // SAFETY: `handle` is a live process handle returned by `OpenProcess`, and
    // `exit_code` is a valid writable out-parameter for the API call.
    let rc = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    // SAFETY: `handle` came from `OpenProcess` above and is closed exactly once.
    let _ = unsafe { CloseHandle(handle) };
    if rc == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(exit_code == STILL_ACTIVE)
}
