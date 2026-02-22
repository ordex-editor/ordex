//! Test utilities for ordex
//!
//! Provides temporary files and a PTY-backed process harness for E2E tests.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use virtual_tty_pty::PtyAdapter;

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
    fn from_snapshot(snapshot: String) -> Self {
        dbg!(&snapshot);
        Self {
            raw: snapshot.clone(),
            rows: snapshot
                .lines()
                .map(|line| line.trim_end().to_string())
                .collect(),
        }
    }

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
    pty: PtyAdapter,
    child: Child,
}

impl PtySession {
    pub fn spawn(binary_path: &str, args: &[&str], config: PtySessionConfig) -> io::Result<Self> {
        let mut pty = PtyAdapter::new(config.cols as usize, config.rows as usize);
        let mut cmd = Command::new(binary_path);
        cmd.args(args).env("TERM", "xterm-256color");
        let child = pty.spawn_command(&mut cmd)?;
        Ok(Self { pty, child })
    }

    pub fn send_text(&mut self, text: &str) -> io::Result<()> {
        for b in text.bytes() {
            self.pty.send_input(&[b])?;
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    pub fn send_enter(&mut self) -> io::Result<()> {
        self.pty.send_input_str("\n")
    }

    pub fn send_escape(&mut self) -> io::Result<()> {
        self.pty.send_input_str("\x1b")
    }

    pub fn wait_until<F>(&mut self, timeout: Duration, mut condition: F) -> io::Result<ScreenSnapshot>
    where
        F: FnMut(&ScreenSnapshot) -> bool,
    {
        let deadline = Instant::now() + timeout;
        let mut last_snapshot = self.snapshot();
        while Instant::now() < deadline {
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
        ScreenSnapshot::from_snapshot(self.pty.get_snapshot())
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
