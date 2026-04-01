//! Test utilities for ordex
//!
//! Provides temporary files and a PTY-backed process harness for E2E tests.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
static PTY_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
        Ok(Self { path })
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
        Ok(Self { path })
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

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// PTY session configuration for end-to-end TUI tests.
#[derive(Debug, Clone)]
pub struct PtySessionConfig {
    pub cols: u16,
    pub rows: u16,
    pub current_dir: Option<PathBuf>,
}

impl Default for PtySessionConfig {
    fn default() -> Self {
        Self {
            cols: 100,
            rows: 30,
            current_dir: None,
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

    /// Return one visible content row, excluding the tab, status, and message rows.
    pub fn row(&self, one_based_row: usize) -> Option<&str> {
        self.terminal_row(one_based_row + Self::RESERVED_TOP_ROWS)
    }

    /// Return whether one visible content row contains `needle`.
    pub fn row_contains(&self, one_based_row: usize, needle: &str) -> bool {
        self.row(one_based_row)
            .is_some_and(|line| line.contains(needle))
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

    pub fn contains(&self, needle: &str) -> bool {
        self.raw.contains(needle) || self.rows.iter().any(|r| r.contains(needle))
    }
}

/// A spawned process attached to a virtual PTY.
pub struct PtySession {
    child: Child,
    master: File,
    transcript: Vec<u8>,
    cols: usize,
    rows: usize,
    lock_guard: Option<MutexGuard<'static, ()>>,
}

impl PtySession {
    /// Spawn a PTY-backed ordex process for end-to-end testing.
    pub fn spawn(binary_path: &str, args: &[&str], config: PtySessionConfig) -> io::Result<Self> {
        // PTY-backed tests can interfere with each other when run concurrently.
        // Serialize PTY session lifetimes within a test binary to reduce flakiness.
        let lock_guard = Some(lock_unpoisoned(pty_test_lock()));

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
            .stdin(unsafe { Stdio::from(File::from_raw_fd(stdin_fd)) })
            .stdout(unsafe { Stdio::from(File::from_raw_fd(stdout_fd)) })
            .stderr(unsafe { Stdio::from(File::from_raw_fd(stderr_fd)) });
        if let Some(current_dir) = config.current_dir.as_ref() {
            command.current_dir(current_dir);
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
        self.send_escape()
            .expect("send escape to exit to normal mode");
        self.wait_until(timeout, |s| s.status_line_contains("NORMAL "))
            .expect("wait for normal mode after escape");
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

    pub fn clear_transcript(&mut self) {
        self.transcript.clear();
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
            std::ptr::null(),
            &mut winsize,
        )
    };

    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((master, slave))
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
