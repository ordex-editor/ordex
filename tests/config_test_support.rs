#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use test_utils::{PtySession, TempFile};

/// Return the test-built ordex binary path.
pub fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Build a unique temporary path for a test config file.
pub fn temp_config_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ordex_config_{}_{}_{}.cfg",
        std::process::id(),
        name,
        std::thread::current().name().unwrap_or("t")
    ))
}

/// Write one config file used by integration tests.
pub fn write_config(path: &Path, content: &str) {
    fs::write(path, content).expect("write config");
}

/// Spawn ordex with a specific config file and target file.
pub fn open_session_with_config(file: &TempFile, config: &Path) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.to_str().expect("config path utf8"),
            file.path().to_str().expect("file path utf8"),
        ],
        Default::default(),
    )
    .expect("spawn ordex with config")
}

/// Wait until the editor is ready in normal mode.
pub fn wait_normal_mode(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL |")
        })
        .expect("wait normal mode");
}
