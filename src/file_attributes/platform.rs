//! Platform-specific file-attribute helpers.
//!
//! Every raw ownership syscall lives here so the rest of the crate stays free of
//! `unsafe`.

#[cfg(unix)]
mod unix {
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    /// Owning user and group of one file.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct FileOwner {
        /// Numeric id of the owning user.
        user_id: u32,
        /// Numeric id of the owning group.
        group_id: u32,
    }

    impl FileOwner {
        /// Read the owning user and group recorded in `metadata`.
        pub(crate) fn from_metadata(metadata: &Metadata) -> Self {
            Self {
                user_id: metadata.uid(),
                group_id: metadata.gid(),
            }
        }

        /// Restore this owner onto one replacement file.
        ///
        /// Handing ownership to another user is privileged, so a kernel refusal is
        /// accepted: an unprivileged save cannot do better than the owner the
        /// replacement already has, and failing the write would be worse than
        /// saving under a different owner.
        pub(crate) fn restore_onto(self, file: &File) -> io::Result<()> {
            // SAFETY: `file` keeps the descriptor alive for the whole call, and
            // `fchown` only reads the two owner ids passed by value.
            let result = unsafe { libc::fchown(file.as_raw_fd(), self.user_id, self.group_id) };
            if result == 0 {
                return Ok(());
            }

            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(code) if code == libc::EPERM || code == libc::EINVAL => Ok(()),
                _ => Err(error),
            }
        }
    }

    /// Limit one about-to-be-created file to owner-only access.
    pub(crate) fn restrict_new_file_to_owner(options: &mut OpenOptions) {
        options.mode(0o600);
    }
}

#[cfg(windows)]
mod windows {
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;

    /// Owning user and group of one file.
    ///
    /// Windows has no numeric owner comparable to a unix uid/gid pair, so there is
    /// nothing to capture and nothing to restore.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct FileOwner;

    impl FileOwner {
        /// Read the owning user and group recorded in `metadata`.
        pub(crate) fn from_metadata(_metadata: &Metadata) -> Self {
            Self
        }

        /// Restore this owner onto one replacement file.
        pub(crate) fn restore_onto(self, _file: &File) -> io::Result<()> {
            Ok(())
        }
    }

    /// Limit one about-to-be-created file to owner-only access.
    ///
    /// File creation carries no mode bits here, so the options stay unchanged.
    pub(crate) fn restrict_new_file_to_owner(_options: &mut OpenOptions) {}
}

#[cfg(unix)]
pub(crate) use unix::{FileOwner, restrict_new_file_to_owner};

#[cfg(windows)]
pub(crate) use windows::{FileOwner, restrict_new_file_to_owner};
