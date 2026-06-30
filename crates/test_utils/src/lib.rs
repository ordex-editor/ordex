//! Test utilities for ordex
//!
//! Provides temporary files and a PTY-backed process harness for E2E tests.

mod lsp_ui;
mod process_env;

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

pub use lsp_ui::{
    StartupAnalysisWaitOptions, overlay_footer_hidden, overlay_footer_visible,
    wait_for_startup_analysis_to_settle,
};
pub use process_env::{EnvVarGuard, ProcessEnvLockGuard, lock_process_environment};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
static PTY_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// PTY input sequence used to send one backward-delete keystroke.
pub const PTY_BACKSPACE: &str = "\u{7f}";

fn pty_test_lock() -> &'static Mutex<()> {
    PTY_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

/// Acquire a mutex guard even when a prior panic poisoned the lock.
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        // This mutex only serializes PTY tests, so poison does not imply invalid shared state.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// A temporary file that is automatically deleted when dropped.
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    /// Create one temporary file with the default Ordex test name pattern.
    pub fn new() -> io::Result<Self> {
        Self::with_suffix("")
    }

    /// Create one temporary file whose name ends with `suffix`.
    pub fn with_suffix(suffix: &str) -> io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "ordex_test_{}_{}{}",
            std::process::id(),
            id,
            suffix
        ));
        File::create(&path)?;
        let canonical_path = path.canonicalize()?;
        Ok(Self {
            path: canonical_path,
        })
    }

    /// Return the filesystem path backing this temporary file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Replace the file contents with `data`.
    pub fn write_all(&self, data: &[u8]) -> io::Result<()> {
        fs::write(&self.path, data)
    }

    /// Append one UTF-8 line plus a trailing newline.
    pub fn writeln(&self, line: &str) -> io::Result<()> {
        let mut file = fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}", line)
    }

    /// Remove the file immediately while keeping the temp handle alive for drop-time cleanup.
    pub fn remove_now(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// A temporary directory tree that is automatically deleted when dropped.
pub struct TempTree {
    path: PathBuf,
}

impl TempTree {
    /// Create one temporary directory tree with the default Ordex test name pattern.
    pub fn new() -> io::Result<Self> {
        Self::with_prefix("ordex_test_tree")
    }

    /// Create one temporary directory tree whose name starts with `prefix`.
    pub fn with_prefix(prefix: &str) -> io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), id));
        fs::create_dir_all(&path)?;
        let canonical_path = path.canonicalize()?;
        Ok(Self {
            path: canonical_path,
        })
    }

    /// Return the filesystem path backing this temporary directory tree.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write one UTF-8 file at `relative_path`, creating parent directories first.
    pub fn write_file(&self, relative_path: &str, contents: &str) -> io::Result<()> {
        let path = self.path.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }
}

/// Return the filesystem path for `binary` when it exists on `PATH`.
pub fn command_path(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

/// Create one symlink to a real binary inside `bin_dir`.
pub fn link_real_binary(bin_dir: &Path, binary: &str) -> io::Result<()> {
    let target = command_path(binary).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("missing required binary on PATH: {binary}"),
        )
    })?;
    symlink(target, bin_dir.join(binary))
}

/// Build one PATH value that exposes only the selected real binaries.
pub fn filtered_path_with_real_binaries(tree: &TempTree, binaries: &[&str]) -> String {
    let bin_dir = tree.path().join("real-bin");
    fs::create_dir_all(&bin_dir).expect("create real-bin");
    // Symlink the real binaries so tests can remove one server from PATH without
    // substituting another executable or mutating the user's real toolchain.
    for binary in binaries {
        link_real_binary(&bin_dir, binary).expect("link real binary");
    }
    bin_dir.display().to_string()
}

/// Return one temporary PATH entry that intentionally exposes no preinstalled binaries.
pub fn missing_server_path_env() -> (TempTree, String) {
    let tree = TempTree::new().expect("temp tree");
    let empty_bin_dir = tree.path().join("missing-bin");
    fs::create_dir_all(&empty_bin_dir).expect("create missing-bin");
    (tree, empty_bin_dir.display().to_string())
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Restore the previous current directory when the guard scope ends.
pub struct CurrentDirectoryGuard {
    previous: PathBuf,
}

impl CurrentDirectoryGuard {
    /// Change the process current directory for one scoped test section.
    pub fn change_to(path: &Path) -> Self {
        let previous = std::env::current_dir().expect("capture current directory");
        std::env::set_current_dir(path).expect("switch current directory");
        Self { previous }
    }
}

impl Drop for CurrentDirectoryGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.previous).expect("restore current directory");
    }
}

/// PTY session configuration for end-to-end TUI tests.
#[derive(Debug, Clone)]
pub struct PtySessionConfig {
    pub cols: u16,
    pub rows: u16,
    pub current_dir: Option<PathBuf>,
    pub cache_root: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Default for PtySessionConfig {
    fn default() -> Self {
        // A 160-column terminal is used so that error messages containing long temp
        // paths (as produced on macOS) fit on a single line without truncation.
        Self {
            cols: 160,
            rows: 30,
            current_dir: None,
            cache_root: None,
            env: Vec::new(),
        }
    }
}

/// Semantic snapshot of a rendered terminal frame.
#[derive(Debug, Clone)]
pub struct ScreenSnapshot {
    raw: String,
    rows: Vec<String>,
}

impl ScreenSnapshot {
    const RESERVED_TOP_ROWS: usize = 1;
    const RESERVED_BOTTOM_ROWS: usize = 2;

    /// Return the raw terminal transcript captured for this snapshot.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Return one raw terminal row without translating UI chrome offsets.
    fn terminal_row(&self, one_based_row: usize) -> Option<&str> {
        self.rows
            .get(one_based_row.saturating_sub(1))
            .map(String::as_str)
    }

    /// Return the persistent top-row tab strip.
    pub fn tab_line(&self) -> Option<&str> {
        self.terminal_row(1)
    }

    /// Return whether the tab strip contains `needle`.
    pub fn tab_line_contains(&self, needle: &str) -> bool {
        self.tab_line().is_some_and(|line| line.contains(needle))
    }

    /// Return the number of non-overlapping occurrences of `needle` in the tab strip.
    ///
    /// Useful for detecting duplicate buffers: each open buffer contributes exactly
    /// one tab, so a file name that appears more than once signals a duplicate.
    pub fn tab_line_count(&self, needle: &str) -> usize {
        self.tab_line()
            .map(|line| line.matches(needle).count())
            .unwrap_or(0)
    }

    /// Return one visible content row, excluding the tab, status, and message rows.
    pub fn row(&self, one_based_row: usize) -> Option<&str> {
        self.terminal_row(one_based_row + Self::RESERVED_TOP_ROWS)
    }

    /// Return whether one visible content row contains `needle`.
    pub fn row_contains(&self, one_based_row: usize, needle: &str) -> bool {
        self.row(one_based_row)
            .is_some_and(|line| line.contains(needle))
    }

    /// Return whether any visible content row contains `needle`.
    pub fn any_row_contains(&self, needle: &str) -> bool {
        self.rows.iter().any(|line| line.contains(needle))
    }

    /// Return whether one visible content row exactly matches `expected` after trimming trailing whitespace.
    ///
    /// Returns `true` when the requested row exists and `line.trim_end() == expected`.
    /// Returns `false` when the row is missing or the trimmed row content differs.
    pub fn row_trimmed_eq(&self, one_based_row: usize, expected: &str) -> bool {
        self.row(one_based_row)
            .is_some_and(|line| line.trim_end() == expected)
    }

    /// Return whether one visible content row ends with `expected` after trimming trailing whitespace.
    ///
    /// Returns `true` when the requested row exists and `line.trim_end().ends_with(expected)`.
    /// Returns `false` when the row is missing or the trimmed row does not end with `expected`.
    pub fn row_trimmed_ends_with(&self, one_based_row: usize, expected: &str) -> bool {
        self.row(one_based_row)
            .is_some_and(|line| line.trim_end().ends_with(expected))
    }

    /// Return the status line above the message line.
    pub fn status_line(&self) -> Option<&str> {
        if self.rows.len() < Self::RESERVED_BOTTOM_ROWS {
            return None;
        }
        self.terminal_row(self.rows.len() - 1)
    }

    /// Return whether the status line contains `needle`.
    pub fn status_line_contains(&self, needle: &str) -> bool {
        self.status_line().is_some_and(|line| line.contains(needle))
    }

    /// Return the bottom message line.
    pub fn message_line(&self) -> Option<&str> {
        self.terminal_row(self.rows.len())
    }

    /// Return whether the bottom message line contains `needle`.
    pub fn message_line_contains(&self, needle: &str) -> bool {
        self.message_line()
            .is_some_and(|line| line.contains(needle))
    }

    /// Return whether any visible screen row contains `needle`.
    ///
    /// Returns `true` when `needle` appears in any row of the parsed terminal
    /// grid, and `false` when it is absent from all rows.
    pub fn contains(&self, needle: &str) -> bool {
        self.raw.contains(needle) || self.rows.iter().any(|r| r.contains(needle))
    }

    /// Return all raw terminal bytes directed at one visible content row.
    ///
    /// Returns the concatenation of every terminal sequence emitted while the
    /// cursor was positioned on `one_based_row` (offset by the reserved top
    /// rows), including ANSI style escapes, gutter content, and row text.
    /// Returns an empty string when no output was directed at that row.
    pub fn raw_for_row(&self, one_based_row: usize) -> String {
        let terminal_row = one_based_row + Self::RESERVED_TOP_ROWS;
        extract_raw_for_terminal_row(&self.raw, terminal_row)
    }
}

/// Wait for the initial Normal-mode frame after spawning Ordex.
pub fn wait_for_initial_render(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(2), |snapshot| {
            snapshot.status_line_contains("NORMAL ")
        })
        .expect("wait for initial render");
}

/// A spawned process attached to a virtual PTY.
pub struct PtySession {
    child: Child,
    master: File,
    transcript: Vec<u8>,
    cols: usize,
    rows: usize,
    cache_root: PathBuf,
    _owned_cache_root: Option<TempTree>,
    lock_guard: Option<MutexGuard<'static, ()>>,
}

impl PtySession {
    /// Spawn a PTY-backed ordex process for end-to-end testing.
    pub fn spawn(binary_path: &str, args: &[&str], config: PtySessionConfig) -> io::Result<Self> {
        // PTY-backed tests can interfere with each other when run concurrently.
        // Serialize PTY session lifetimes within a test binary to reduce flakiness.
        let lock_guard = Some(lock_unpoisoned(pty_test_lock()));
        let (cache_root, owned_cache_root) = match config.cache_root.clone() {
            Some(path) => {
                fs::create_dir_all(&path)?;
                (path, None)
            }
            None => {
                let tree = TempTree::with_prefix("ordex_test_cache")?;
                (tree.path().to_path_buf(), Some(tree))
            }
        };

        let (master_fd, slave_fd) = open_pty(config.cols, config.rows)?;
        set_nonblocking(master_fd)?;

        let stdin_fd = duplicate_fd(slave_fd)?;
        let stdout_fd = duplicate_fd(slave_fd)?;
        let stderr_fd = duplicate_fd(slave_fd)?;

        let mut command = Command::new(binary_path);
        command
            .args(args)
            .env("TERM", "xterm-256color")
            .env_remove("COLORTERM")
            .env("ORDEX_DISABLE_DEFAULT_CONFIG", "1")
            .env("ORDEX_NO_WARNING_PAUSE", "1")
            .env("XDG_CACHE_HOME", &cache_root)
            .stdin(unsafe { Stdio::from(File::from_raw_fd(stdin_fd)) })
            .stdout(unsafe { Stdio::from(File::from_raw_fd(stdout_fd)) })
            .stderr(unsafe { Stdio::from(File::from_raw_fd(stderr_fd)) });
        if let Some(current_dir) = config.current_dir.as_ref() {
            command.current_dir(current_dir);
        }
        // Allow end-to-end tests to inject feature-specific environment overrides
        // without changing the shared defaults every PTY-backed test relies on.
        for (key, value) in &config.env {
            command.env(key, value);
        }

        let child = command.spawn()?;

        unsafe {
            libc::close(slave_fd);
        }

        Ok(Self {
            child,
            master: unsafe { File::from_raw_fd(master_fd) },
            transcript: Vec::new(),
            cols: config.cols as usize,
            rows: config.rows as usize,
            cache_root,
            _owned_cache_root: owned_cache_root,
            lock_guard,
        })
    }

    /// Send literal text bytes to the PTY with a short pacing delay.
    pub fn send_text(&mut self, text: &str) -> io::Result<()> {
        for b in text.bytes() {
            self.master.write_all(&[b])?;
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    /// Send one raw byte slice to the PTY without pacing delays.
    pub fn send_raw_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.master.write_all(bytes)
    }

    /// Send Enter to the PTY.
    pub fn send_enter(&mut self) -> io::Result<()> {
        self.master.write_all(b"\n")
    }

    /// Send Escape to the PTY.
    pub fn send_escape(&mut self) -> io::Result<()> {
        self.master.write_all(b"\x1b")
    }

    #[track_caller]
    pub fn exit_to_normal_mode(&mut self, timeout: Duration) {
        // When the retry loop sends more than one ESC byte, the input parser can
        // absorb the second ESC as a continuation byte of the first ESC sequence
        // and push it back into the pending-byte queue.  The NORMAL render that
        // satisfies `wait_until` comes from the first ESC being dispatched; the
        // queued second ESC has not been consumed yet.  If the caller immediately
        // sends a keystroke (e.g. `O`), the editor will read the pending ESC
        // first, enter `parse_escape_sequence`, and consume that keystroke as the
        // continuation byte of the queued ESC sequence rather than as a fresh
        // normal-mode command.
        //
        // Sleeping for slightly longer than the ESC-sequence timeout (50 ms)
        // gives the editor time to drain the pending ESC through its own
        // `read_input_event` call: `parse_escape_sequence` will poll for a
        // continuation byte, find nothing within 50 ms, and return `Key::Esc`
        // as a no-op in NORMAL mode.  Only after that is the input path clear
        // for the next intentional keystroke.
        const ESC_SEQUENCE_TIMEOUT_MS: u64 = 50;
        const ESC_SEQUENCE_DRAIN_MARGIN: Duration =
            Duration::from_millis(ESC_SEQUENCE_TIMEOUT_MS + 10);
        const ESCAPE_SETTLE_WAIT: Duration = Duration::from_millis(250);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.send_escape()
                .expect("send escape to exit to normal mode");
            // PTY-backed tests can race editor-side background polling while one
            // insert popup or startup overlay is still settling, so retry Escape
            // until the requested timeout instead of assuming one byte is enough.
            if self
                .wait_until(ESCAPE_SETTLE_WAIT, |s| s.status_line_contains("NORMAL "))
                .is_ok()
            {
                thread::sleep(ESC_SEQUENCE_DRAIN_MARGIN);
                return;
            }
        }
        self.send_escape()
            .expect("send final escape to exit to normal mode");
        self.wait_until(ESCAPE_SETTLE_WAIT, |s| s.status_line_contains("NORMAL "))
            .expect("wait for normal mode after escape");
        thread::sleep(ESC_SEQUENCE_DRAIN_MARGIN);
    }

    /// Resize the PTY and notify the child with `SIGWINCH`.
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        let mut winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &mut winsize) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }

        self.cols = cols as usize;
        self.rows = rows as usize;

        let pid = self.child.id() as libc::pid_t;
        let kill_rc = unsafe { libc::kill(pid, libc::SIGWINCH) };
        if kill_rc < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    /// Send a Unix signal to the spawned child process.
    pub fn send_signal(&self, signal: libc::c_int) -> io::Result<()> {
        let pid = self.child.id() as libc::pid_t;
        let rc = unsafe { libc::kill(pid, signal) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Wait until the parsed PTY screen satisfies `condition`.
    pub fn wait_until<F>(
        &mut self,
        timeout: Duration,
        mut condition: F,
    ) -> io::Result<ScreenSnapshot>
    where
        F: FnMut(&ScreenSnapshot) -> bool,
    {
        let deadline = Instant::now() + timeout;
        let mut last_snapshot = self.snapshot();
        while Instant::now() < deadline {
            self.read_available()?;
            let snapshot = self.snapshot();
            last_snapshot = snapshot.clone();
            if condition(&snapshot) {
                return Ok(snapshot);
            }
            thread::sleep(Duration::from_millis(10));
        }

        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "condition not met before timeout; snapshot:\n{}",
                last_snapshot.rows.join("\n")
            ),
        ))
    }

    pub fn snapshot(&self) -> ScreenSnapshot {
        parse_ansi_screen(&self.transcript, self.cols, self.rows)
    }

    /// Discard all bytes that have accumulated in the transcript so far and
    /// drain any bytes the child process has already written into the PTY
    /// master buffer.  Subsequent reads see only output produced after this
    /// call, which prevents stale render frames from corrupting assertions that
    /// inspect the raw byte stream for specific escape sequences.
    pub fn clear_transcript(&mut self) {
        self.transcript.clear();
        // Drain the kernel PTY buffer so bytes from renders that completed
        // before this call are not mixed into the next snapshot.
        let mut buf = [0_u8; 8192];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }

    /// Return the isolated XDG cache root used by this spawned process.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn read_available(&mut self) -> io::Result<()> {
        let mut buf = [0_u8; 8192];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.transcript.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    if e.raw_os_error() == Some(libc::EIO) {
                        break;
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Wait for the child to exit and return its final status.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            // Keep draining PTY output while the process is shutting down so tests can
            // assert against the terminal cleanup bytes emitted just before exit.
            self.read_available()?;
            if let Some(status) = self.child.try_wait()? {
                self.read_available()?;
                self.lock_guard = None;
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(10));
        }

        let snapshot = self.snapshot();
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "process did not exit before timeout; snapshot:\n{}",
                snapshot.rows.join("\n")
            ),
        ))
    }

    /// Wait for a successful process exit.
    pub fn wait_for_exit_success(&mut self, timeout: Duration) -> io::Result<()> {
        let status = self.wait_for_exit(timeout)?;
        if status.success() {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "process exited with non-zero status: {status}"
        )))
    }
}

/// Spawn one PTY-backed Ordex session for the provided CLI arguments.
pub fn spawn_ordex(
    binary_path: &str,
    args: &[&str],
    config: PtySessionConfig,
) -> io::Result<PtySession> {
    PtySession::spawn(binary_path, args, config)
}

/// Spawn one PTY-backed Ordex session for one or more file paths.
pub fn spawn_lsp_session(binary_path: &str, file_paths: &[PathBuf]) -> io::Result<PtySession> {
    spawn_lsp_session_with_config(binary_path, file_paths, PtySessionConfig::default())
}

/// Spawn one PTY-backed Ordex session for one or more file paths with custom PTY settings.
pub fn spawn_lsp_session_with_config(
    binary_path: &str,
    file_paths: &[PathBuf],
    config: PtySessionConfig,
) -> io::Result<PtySession> {
    let args = file_paths
        .iter()
        .map(|path| path.to_str().expect("utf8 fixture path"))
        .collect::<Vec<_>>();
    spawn_ordex(binary_path, &args, config)
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.lock_guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::lock_unpoisoned;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::Mutex;

    /// Recovers the PTY serialization lock after an earlier panic poisoned it.
    #[test]
    fn lock_unpoisoned_recovers_after_panic() {
        let mutex = Mutex::new(());

        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = mutex.lock().expect("lock mutex for poison setup");
            panic!("poison the mutex");
        }));

        let _guard = lock_unpoisoned(&mutex);
    }
}

fn duplicate_fd(fd: RawFd) -> io::Result<RawFd> {
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(dup_fd)
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn open_pty(cols: u16, rows: u16) -> io::Result<(RawFd, RawFd)> {
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null::<libc::termios>() as _,
            &mut winsize,
        )
    };

    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((master, slave))
}

/// Collect all raw bytes directed at `terminal_row` (1-based) from a terminal transcript.
///
/// Scans the raw ANSI byte stream for cursor-goto sequences that move the cursor
/// to `terminal_row` and accumulates every byte written while the cursor remains
/// on that row. The result includes style escapes and text characters in the order
/// they were emitted, which allows assertions on per-row styling.
fn extract_raw_for_terminal_row(raw: &str, terminal_row: usize) -> String {
    let bytes = raw.as_bytes();
    let mut result: Vec<u8> = Vec::new();
    let mut cursor_row = 1_usize;
    let mut i = 0_usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            // Skip OSC sequences (cursor-color updates and similar).
            if bytes[i] == b']' {
                i += 1;
                i = skip_osc_sequence(bytes, i);
                continue;
            }
            if bytes[i] != b'[' {
                // Non-CSI escape: not a goto, skip the introducer byte.
                continue;
            }
            i += 1;

            // Collect the CSI parameter string up to the command byte.
            let esc_start = i - 2; // points to the leading ESC
            while i < bytes.len() && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'@') {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }

            let final_byte = bytes[i] as char;
            i += 1;
            let esc_end = i;

            if matches!(final_byte, 'H' | 'f') {
                // Cursor-goto sequence: update the tracked row without accumulating
                // the goto itself into any row's raw output.
                let params = std::str::from_utf8(&bytes[esc_start + 2..esc_end - 1]).unwrap_or("");
                let mut parts = params.split(';');
                let row = parts
                    .next()
                    .and_then(|p| p.parse::<usize>().ok())
                    .unwrap_or(1);
                cursor_row = row.max(1);
            } else if cursor_row == terminal_row {
                // Non-goto CSI sequence emitted while on the target row: include it.
                result.extend_from_slice(&bytes[esc_start..esc_end]);
            }
            continue;
        }

        // Accumulate ordinary bytes while on the target row.
        if cursor_row == terminal_row {
            result.push(b);
        }
        i += 1;
    }

    String::from_utf8_lossy(&result).into_owned()
}

fn parse_ansi_screen(bytes: &[u8], cols: usize, rows: usize) -> ScreenSnapshot {
    let mut grid = vec![vec![' '; cols]; rows];
    let mut cursor_row = 1_usize;
    let mut cursor_col = 1_usize;
    let mut i = 0_usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            if bytes[i] == b']' {
                // Cursor-color updates use OSC sequences rather than CSI. Skip
                // their payload entirely so snapshot assertions only see the
                // rendered screen content, not the terminal control data.
                i += 1;
                i = skip_osc_sequence(bytes, i);
                continue;
            }
            if bytes[i] != b'[' {
                continue;
            }
            i += 1;

            // Collect the CSI parameters until the command byte. The parser only
            // implements the subset of escape sequences that Ordex currently uses
            // for screen painting and cursor placement in tests.
            let start = i;
            while i < bytes.len() && !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'@') {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }

            let final_byte = bytes[i] as char;
            let params = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            i += 1;

            match final_byte {
                'H' | 'f' => {
                    // Cursor-positioning sequences use 1-based terminal
                    // coordinates, so keep the internal cursor in the same space.
                    let mut parts = params.split(';');
                    let row = parts
                        .next()
                        .and_then(|p| p.parse::<usize>().ok())
                        .unwrap_or(1);
                    let col = parts
                        .next()
                        .and_then(|p| p.parse::<usize>().ok())
                        .unwrap_or(1);
                    cursor_row = row.clamp(1, rows.max(1));
                    cursor_col = col.clamp(1, cols.max(1));
                }
                'J' => {
                    if params == "2" {
                        // Full-screen clear resets the snapshot grid without
                        // changing the current cursor position.
                        for row in &mut grid {
                            for c in row.iter_mut() {
                                *c = ' ';
                            }
                        }
                    }
                }
                'K' => {
                    if cursor_row > 0 && cursor_row <= rows {
                        // Clear-to-end-of-line is enough for Ordex because it
                        // rewrites each row from left to right.
                        let row = &mut grid[cursor_row - 1];
                        let start_col = cursor_col.saturating_sub(1).min(cols);
                        for c in row.iter_mut().skip(start_col) {
                            *c = ' ';
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        match b {
            b'\n' => {
                // Newline advances the row and returns to column 1, matching the
                // terminal behavior that Ordex relies on in tests.
                cursor_row = (cursor_row + 1).min(rows.max(1));
                cursor_col = 1;
                i += 1;
            }
            b'\r' => {
                // Carriage return only resets the horizontal position.
                cursor_col = 1;
                i += 1;
            }
            0x20..=0x7e => {
                // Printable ASCII bytes occupy exactly one cell in the snapshot.
                if cursor_row > 0 && cursor_row <= rows && cursor_col > 0 && cursor_col <= cols {
                    grid[cursor_row - 1][cursor_col - 1] = b as char;
                }
                cursor_col = (cursor_col + 1).min(cols.max(1));
                i += 1;
            }
            _ => {
                if let Some((ch, len)) = decode_utf8_char(bytes, i) {
                    if !ch.is_control()
                        && cursor_row > 0
                        && cursor_row <= rows
                        && cursor_col > 0
                        && cursor_col <= cols
                    {
                        // Non-ASCII text is decoded one Unicode scalar at a time
                        // so row-based assertions can see wrapped Unicode content.
                        grid[cursor_row - 1][cursor_col - 1] = ch;
                        cursor_col = (cursor_col + 1).min(cols.max(1));
                    }
                    i += len;
                } else {
                    // Skip invalid or unsupported bytes rather than failing the
                    // whole snapshot parse.
                    i += 1;
                }
            }
        }
    }

    let rows: Vec<String> = grid
        .into_iter()
        .map(|line| {
            let mut s: String = line.into_iter().collect();
            // Trimming trailing spaces keeps assertions focused on meaningful
            // rendered content instead of terminal fill characters.
            while s.ends_with(' ') {
                s.pop();
            }
            s
        })
        .collect();

    ScreenSnapshot {
        raw: String::from_utf8_lossy(bytes).to_string(),
        rows,
    }
}

/// Skip one OSC sequence and return the index of the next unconsumed byte.
fn skip_osc_sequence(bytes: &[u8], mut start: usize) -> usize {
    while start < bytes.len() {
        match bytes[start] {
            b'\x07' => return start + 1,
            b'\x1b' if bytes.get(start + 1) == Some(&b'\\') => return start + 2,
            _ => start += 1,
        }
    }
    start
}

/// Decode one UTF-8 character from `bytes[start..]`.
fn decode_utf8_char(bytes: &[u8], start: usize) -> Option<(char, usize)> {
    for len in 1..=4 {
        let end = start.saturating_add(len);
        let slice = bytes.get(start..end)?;
        if let Ok(text) = std::str::from_utf8(slice) {
            let mut chars = text.chars();
            if let Some(ch) = chars.next() {
                return Some((ch, len));
            }
        }
    }
    None
}

/// Wait until one `:w` command reports success in the PTY status area.
pub fn wait_for_write_confirmation(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");
}

/// Build one temporary Cargo workspace that matches the trailing-expression reproducer.
pub fn hello_world_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"hello_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n    println!(\"Hello, world!\");\n}\n",
    )
    .expect("write main.rs");
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ansi_screen_ignores_osc_cursor_color_sequences() {
        let snapshot = parse_ansi_screen(b"\x1b]12;#7287fd\x07\x1b[1;1HX", 4, 2);
        assert_eq!(snapshot.row(1), Some("X"));
        assert!(snapshot.raw().contains("\x1b]12;#7287fd\x07"));
    }
}
