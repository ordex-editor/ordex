//! Platform-selected external file-change monitoring helpers.

mod common;
#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_inotify;
mod polling;

pub(crate) use common::{ExternalFileState, FileFingerprint, read_fingerprint_from_disk};
#[cfg(not(target_os = "linux"))]
pub(crate) use fallback::FileMonitor;
#[cfg(target_os = "linux")]
pub(crate) use linux::FileMonitor;
