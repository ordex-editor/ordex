//! POSIX signal helpers for terminal resize and termination handling.
//!
//! This module contains all `unsafe` signal interaction so the rest of the
//! editor can stay safe Rust.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

const TERMINATION_SIGNALS: [libc::c_int; 4] =
    [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT];

static RESIZE_PENDING: AtomicBool = AtomicBool::new(false);
static TERMINATION_PENDING: AtomicI32 = AtomicI32::new(0);

/// Mark a resize as pending from the SIGWINCH handler.
extern "C" fn handle_sigwinch(_: libc::c_int) {
    RESIZE_PENDING.store(true, Ordering::Relaxed);
}

/// Record the first observed termination signal for the main loop.
extern "C" fn handle_termination(signal: libc::c_int) {
    let _ = TERMINATION_PENDING.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
}

/// Previously installed action for one signal.
struct InstalledSignalAction {
    signal: libc::c_int,
    old_action: libc::sigaction,
}

/// RAII guard that installs application signal handlers and restores prior ones.
pub(crate) struct SignalGuard {
    installed_actions: Vec<InstalledSignalAction>,
}

impl SignalGuard {
    /// Install resize and termination handlers and restore them on drop.
    pub(crate) fn install() -> io::Result<Self> {
        let mut installed_actions = Vec::with_capacity(1 + TERMINATION_SIGNALS.len());
        TERMINATION_PENDING.store(0, Ordering::Relaxed);
        RESIZE_PENDING.store(false, Ordering::Relaxed);

        // Install SIGWINCH first because the editor depends on resize tracking
        // throughout the rest of startup and the main loop.
        match install_handler(libc::SIGWINCH, handle_sigwinch) {
            Ok(installed) => installed_actions.push(installed),
            Err(error) => return Err(error),
        }

        // Install termination handlers one by one so we can roll back if any
        // registration fails instead of leaving a partially configured process.
        for signal in TERMINATION_SIGNALS {
            match install_handler(signal, handle_termination) {
                Ok(installed) => installed_actions.push(installed),
                Err(error) => {
                    restore_installed_actions(&installed_actions);
                    return Err(error);
                }
            }
        }

        Ok(Self { installed_actions })
    }

    /// Mark resize state dirty so the next event loop iteration refreshes dimensions.
    pub(crate) fn mark_resize_pending(&self) {
        RESIZE_PENDING.store(true, Ordering::Relaxed);
    }

    /// Return whether a resize was observed since the last check.
    ///
    /// Returns `true` when the next loop iteration should refresh terminal
    /// dimensions, and `false` when no resize signal is currently pending.
    pub(crate) fn take_resize_pending(&self) -> bool {
        RESIZE_PENDING.swap(false, Ordering::Relaxed)
    }

    /// Return the pending termination signal, if any, and clear it.
    pub(crate) fn take_termination_signal(&self) -> Option<libc::c_int> {
        let signal = TERMINATION_PENDING.swap(0, Ordering::Relaxed);
        (signal != 0).then_some(signal)
    }
}

impl Drop for SignalGuard {
    /// Restore every previously installed signal action.
    fn drop(&mut self) {
        restore_installed_actions(&self.installed_actions);
    }
}

/// Install one signal handler and capture the previously configured action.
fn install_handler(
    signal: libc::c_int,
    handler: extern "C" fn(libc::c_int),
) -> io::Result<InstalledSignalAction> {
    // SAFETY: `zeroed` is valid for `sigaction`, a plain C POD struct.
    let mut new_action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    // SAFETY: same rationale as above for storing the previous action value.
    let mut old_action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    new_action.sa_sigaction = handler as *const () as usize;
    new_action.sa_flags = 0;

    // SAFETY: `new_action.sa_mask` is a valid mutable pointer to a signal set.
    unsafe {
        libc::sigemptyset(&mut new_action.sa_mask);
    }

    // SAFETY: all pointers are valid for this call and outlive the FFI call.
    let rc = unsafe { libc::sigaction(signal, &new_action, &mut old_action) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(InstalledSignalAction { signal, old_action })
}

/// Restore a previously installed signal action.
fn restore_installed_action(installed_action: &InstalledSignalAction) {
    // SAFETY: restoring a previously returned `sigaction` for the same signal
    // matches the libc contract; pointers are valid for the duration of the call.
    unsafe {
        libc::sigaction(
            installed_action.signal,
            &installed_action.old_action,
            std::ptr::null_mut(),
        );
    }
}

/// Restore signal actions in reverse installation order.
fn restore_installed_actions(installed_actions: &[InstalledSignalAction]) {
    // Reverse order mirrors stack-style setup and avoids surprising interactions
    // if multiple handlers were installed for related signals during startup.
    for installed_action in installed_actions.iter().rev() {
        restore_installed_action(installed_action);
    }
}
