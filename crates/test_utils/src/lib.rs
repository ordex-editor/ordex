//! Test utilities for ordex
//!
//! Provides temporary files and a PTY-backed process harness for E2E tests.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A temporary file that is automatically deleted when dropped.
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub fn new() -> io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("ordex_test_{}_{}", std::process::id(), id));
        File::create(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn write_all(&self, data: &[u8]) -> io::Result<()> {
        fs::write(&self.path, data)
    }

    pub fn writeln(&self, line: &str) -> io::Result<()> {
        let mut file = fs::OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}", line)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// PTY session configuration for end-to-end TUI tests.
#[derive(Debug, Clone, Copy)]
pub struct PtySessionConfig {
    pub cols: u16,
    pub rows: u16,
}

impl Default for PtySessionConfig {
    fn default() -> Self {
        Self { cols: 100, rows: 30 }
    }
}

/// Semantic snapshot of a rendered terminal frame.
#[derive(Debug, Clone)]
pub struct ScreenSnapshot {
    raw: String,
    rows: Vec<String>,
}

impl ScreenSnapshot {
    pub fn row(&self, one_based_row: usize) -> Option<&str> {
        self.rows.get(one_based_row.saturating_sub(1)).map(String::as_str)
    }

    pub fn status_line(&self) -> Option<&str> {
        if self.rows.len() < 2 {
            return None;
        }
        self.row(self.rows.len() - 1)
    }

    pub fn message_line(&self) -> Option<&str> {
        self.row(self.rows.len())
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
}

impl PtySession {
    pub fn spawn(binary_path: &str, args: &[&str], config: PtySessionConfig) -> io::Result<Self> {
        let (master_fd, slave_fd) = open_pty(config.cols, config.rows)?;
        set_nonblocking(master_fd)?;

        let stdin_fd = duplicate_fd(slave_fd)?;
        let stdout_fd = duplicate_fd(slave_fd)?;
        let stderr_fd = duplicate_fd(slave_fd)?;

        let mut command = Command::new(binary_path);
        command
            .args(args)
            .env("TERM", "xterm-256color")
            .stdin(unsafe { Stdio::from(File::from_raw_fd(stdin_fd)) })
            .stdout(unsafe { Stdio::from(File::from_raw_fd(stdout_fd)) })
            .stderr(unsafe { Stdio::from(File::from_raw_fd(stderr_fd)) });

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
        })
    }

    pub fn send_text(&mut self, text: &str) -> io::Result<()> {
        for b in text.bytes() {
            self.master.write_all(&[b])?;
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    pub fn send_enter(&mut self) -> io::Result<()> {
        self.master.write_all(b"\n")
    }

    pub fn send_escape(&mut self) -> io::Result<()> {
        self.master.write_all(b"\x1b")
    }

    pub fn wait_until<F>(&mut self, timeout: Duration, mut condition: F) -> io::Result<ScreenSnapshot>
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

    pub fn wait_for_exit_success(&mut self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                if status.success() {
                    return Ok(());
                }
                return Err(io::Error::other(format!(
                    "process exited with non-zero status: {status}"
                )));
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
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
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
            if i >= bytes.len() || bytes[i] != b'[' {
                continue;
            }
            i += 1;

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
                        for row in &mut grid {
                            for c in row.iter_mut() {
                                *c = ' ';
                            }
                        }
                    }
                }
                'K' => {
                    if cursor_row > 0 && cursor_row <= rows {
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
                cursor_row = (cursor_row + 1).min(rows.max(1));
                cursor_col = 1;
            }
            b'\r' => {
                cursor_col = 1;
            }
            0x20..=0x7e => {
                if cursor_row > 0 && cursor_row <= rows && cursor_col > 0 && cursor_col <= cols {
                    grid[cursor_row - 1][cursor_col - 1] = b as char;
                }
                cursor_col = (cursor_col + 1).min(cols.max(1));
            }
            _ => {}
        }
        i += 1;
    }

    let rows: Vec<String> = grid
        .into_iter()
        .map(|line| {
            let mut s: String = line.into_iter().collect();
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
