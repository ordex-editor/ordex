use std::time::Duration;
use test_utils::{PtySession, TempFile};

/// Return the test-built ordex binary path.
pub fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Create and write one temporary config file for an integration test.
pub fn write_config(content: &str) -> TempFile {
    let file = TempFile::new().expect("create temp config");
    file.write_all(content.as_bytes()).expect("write config");
    file
}

/// Spawn ordex with a specific config file and target file.
pub fn open_session_with_config(file: &TempFile, config: &TempFile) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[
            "--config",
            config.path().to_str().expect("config path utf8"),
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
            s.status_line_contains("NORMAL ")
        })
        .expect("wait normal mode");
}
