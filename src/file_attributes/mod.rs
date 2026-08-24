//! On-disk attribute preservation for atomic file replacement.
//!
//! Replacing a file through a temp file and a rename gives the saved document
//! the attributes of the freshly created temp file. The helpers here capture the
//! attributes of the file being replaced and restore them onto the replacement
//! before the rename makes it visible.

mod platform;

use std::fs;
use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::path::Path;

use platform::FileOwner;

/// Attributes captured from the file that one atomic write replaces.
pub(crate) struct PreservedAttributes {
    /// Permission bits of the replaced file, including setuid, setgid and sticky.
    permissions: Permissions,
    /// Owner of the replaced file.
    owner: FileOwner,
}

impl PreservedAttributes {
    /// Capture the attributes that must survive replacing `target_path`.
    ///
    /// Returns `None` when no readable file exists at `target_path`, meaning the
    /// save creates a brand-new file that keeps the process defaults.
    pub(crate) fn capture(target_path: &Path) -> Option<Self> {
        let metadata = fs::metadata(target_path).ok()?;
        Some(Self {
            permissions: metadata.permissions(),
            owner: FileOwner::from_metadata(&metadata),
        })
    }

    /// Restore these attributes onto the replacement `file` before it is renamed.
    pub(crate) fn restore_onto(&self, file: &File) -> io::Result<()> {
        // Ownership goes first: changing it clears the setuid and setgid bits that
        // the permission restore then puts back.
        self.owner.restore_onto(file)?;
        file.set_permissions(self.permissions.clone())
    }
}

/// Open one new temp file that will receive the replacement contents.
///
/// `replaced` restricts the initial permissions to the owner, so the contents of
/// a private file are never readable through the temp file that replaces it.
/// `create_new` refuses to reuse any pre-existing sibling path, so a stale temp
/// name from another process cannot be truncated and mistaken for this write.
pub(crate) fn create_replacement_file(
    temp_path: &Path,
    replaced: Option<&PreservedAttributes>,
) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    if replaced.is_some() {
        platform::restrict_new_file_to_owner(&mut options);
    }
    options.open(temp_path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use test_utils::TempTree;

    /// Verify executable permission bits survive a capture and restore round-trip.
    #[test]
    fn restores_executable_permission_bits() {
        let tree = TempTree::new().expect("create temp tree");
        let target_path = tree.path().join("script.sh");
        fs::write(&target_path, "#!/bin/sh\n").expect("seed target file");
        fs::set_permissions(&target_path, Permissions::from_mode(0o755))
            .expect("mark target executable");

        let attributes = PreservedAttributes::capture(&target_path).expect("capture attributes");
        let temp_path = tree.path().join("script.sh.tmp");
        let file =
            create_replacement_file(&temp_path, Some(&attributes)).expect("create replacement");
        attributes.restore_onto(&file).expect("restore attributes");

        let mode = fs::metadata(&temp_path)
            .expect("read replacement")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    /// Verify a replacement file stays owner-only while it is being written.
    #[test]
    fn replacement_file_starts_private() {
        let tree = TempTree::new().expect("create temp tree");
        let target_path = tree.path().join("secret.txt");
        fs::write(&target_path, "secret\n").expect("seed target file");
        fs::set_permissions(&target_path, Permissions::from_mode(0o644))
            .expect("set target permissions");

        let attributes = PreservedAttributes::capture(&target_path).expect("capture attributes");
        let temp_path = tree.path().join("secret.txt.tmp");
        create_replacement_file(&temp_path, Some(&attributes)).expect("create replacement");

        let mode = fs::metadata(&temp_path)
            .expect("read replacement")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// Verify no attributes are captured for a path with no existing file.
    #[test]
    fn captures_nothing_for_a_new_file() {
        let tree = TempTree::new().expect("create temp tree");
        assert!(PreservedAttributes::capture(&tree.path().join("missing.txt")).is_none());
    }
}
