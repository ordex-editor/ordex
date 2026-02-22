//! POSIX signal helpers for terminal resize handling.
//!
//! This module contains all `unsafe` signal interaction so the rest of the
//! editor can stay safe Rust.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

static RESIZE_PENDING: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_: libc::c_int) {
    RESIZE_PENDING.store(true, Ordering::Relaxed);
}

/// RAII guard that installs a SIGWINCH handler and restores the previous one.
pub(crate) struct SigwinchGuard {
    old_action: libc::sigaction,
}

impl SigwinchGuard {
    /// Install SIGWINCH handler and return a guard restoring prior state on drop.
    pub(crate) fn install() -> io::Result<Self> {
        // SAFETY: `zeroed` is valid for `sigaction`, a plain C POD struct.
        let mut new_action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        // SAFETY: same rationale as above for storing previous action value.
        let mut old_action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        new_action.sa_sigaction = handle_sigwinch as *const () as usize;
        new_action.sa_flags = 0;

        // SAFETY: `new_action.sa_mask` is a valid mutable pointer to a signal set.
        unsafe {
            libc::sigemptyset(&mut new_action.sa_mask);
        }

        // SAFETY: all pointers are valid for this call and outlive the FFI call.
        let rc = unsafe { libc::sigaction(libc::SIGWINCH, &new_action, &mut old_action) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self { old_action })
    }

    /// Mark resize state dirty so next event loop iteration refreshes dimensions.
    pub(crate) fn mark_pending(&self) {
        RESIZE_PENDING.store(true, Ordering::Relaxed);
    }

    /// Returns whether a resize was observed since last check.
    pub(crate) fn take_pending(&self) -> bool {
        RESIZE_PENDING.swap(false, Ordering::Relaxed)
    }
}

impl Drop for SigwinchGuard {
    fn drop(&mut self) {
        // SAFETY: restoring previously returned action for SIGWINCH with null oldact
        // matches the libc contract; pointers are valid for the duration of the call.
        unsafe {
            libc::sigaction(libc::SIGWINCH, &self.old_action, std::ptr::null_mut());
        }
    }
}
