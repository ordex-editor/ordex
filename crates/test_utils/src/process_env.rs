//! Process-environment test helpers that isolate the unsafe mutation surface.

use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

static PROCESS_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Guard proving the current test has exclusive access to process-global environment state.
pub struct ProcessEnvLockGuard {
    _guard: MutexGuard<'static, ()>,
}

/// Guard that restores one process environment variable when it drops.
pub struct EnvVarGuard<'a> {
    name: &'static str,
    previous: Option<OsString>,
    _lock: &'a ProcessEnvLockGuard,
}

/// Return the shared mutex that serializes test access to process-global environment state.
fn process_env_lock() -> &'static Mutex<()> {
    PROCESS_ENV_LOCK.get_or_init(|| Mutex::new(()))
}

/// Acquire exclusive access to process-global environment state for one test scope.
pub fn lock_process_environment() -> ProcessEnvLockGuard {
    let guard = match process_env_lock().lock() {
        Ok(guard) => guard,
        // This lock only serializes test access to process-global state, so a
        // poisoned mutex does not imply the environment itself is invalid.
        Err(poisoned) => poisoned.into_inner(),
    };
    ProcessEnvLockGuard { _guard: guard }
}

impl<'a> EnvVarGuard<'a> {
    /// Set one process environment variable for the lifetime of this guard.
    pub fn set(lock: &'a ProcessEnvLockGuard, name: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(name);
        // SAFETY: `ProcessEnvLockGuard` serializes all test-time environment
        // mutations performed through this helper, and `EnvVarGuard` borrows that
        // lock for its full lifetime so the mutation and restoration stay ordered.
        unsafe {
            std::env::set_var(name, &value);
        }
        Self {
            name,
            previous,
            _lock: lock,
        }
    }
}

impl Drop for EnvVarGuard<'_> {
    /// Restore the saved environment value when the guard scope ends.
    fn drop(&mut self) {
        // SAFETY: the borrowed `ProcessEnvLockGuard` proves this guard still holds
        // exclusive access to process-global environment mutation while restoring
        // the previous value captured by `EnvVarGuard::set`.
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.name, previous);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }
}
