use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig};

/// Return the compiled ordex binary path for PTY-backed integration tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// One temporary directory tree cleaned up automatically after the test.
struct TempTree {
    path: PathBuf,
}

impl TempTree {
    /// Create one unique temporary directory for a file-picker test.
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "ordex_file_picker_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// Return the root path of the temporary directory tree.
    fn path(&self) -> &Path {
        &self.path
    }

    /// Write one UTF-8 file at `relative_path`, creating parent directories first.
    fn write_file(&self, relative_path: &str, contents: &str) -> io::Result<()> {
        let path = self.path.join(relative_path);
        if let Some(parent) = path.parent() {
            // Nested test fixtures create directories lazily so each scenario can
            // describe only the paths it actually needs.
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Verify that the async file picker lists visible files, filters results, and opens a selection.
#[test]
fn test_file_picker_filters_visible_files_and_opens_selection() {
    let tree = TempTree::new().expect("create temp tree");
    tree.write_file("src/main.rs", "fn main() {}\n")
        .expect("write visible source file");
    tree.write_file("notes.txt", "notes\n")
        .expect("write visible text file");
    tree.write_file(".secret", "hidden\n")
        .expect("write hidden file");
    tree.write_file("src/.cache/ignored.txt", "hidden nested\n")
        .expect("write hidden nested file");
    tree.write_file(".gitignore", "ignored.log\n")
        .expect("write gitignore");
    tree.write_file("ignored.log", "ignored\n")
        .expect("write ignored file");

    Command::new("git")
        .current_dir(tree.path())
        .args(["init", "-q"])
        .status()
        .expect("run git init");

    let mut session = PtySession::spawn(
        ordex_bin(),
        &[],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            ..Default::default()
        },
    )
    .expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait for startup frame");

    session
        .send_text(" fmain")
        .expect("open file picker and type filter");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.status_line_contains("NORMAL ")
                && s.contains("src/main.rs")
                && !s.contains("ignored.log")
                && !s.contains(".secret")
                && !s.contains(".gitignore")
        })
        .expect("wait for async file-picker results");

    session.send_enter().expect("confirm file picker selection");
    session
        .wait_until(Duration::from_secs(3), |s| {
            s.row_contains(1, "fn main() {}")
        })
        .expect("open selected file");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
