use std::path::Path;
use std::time::Duration;
use test_utils::{PtySession, PtySessionConfig, TempFile};

/// Return the test-built ordex binary path.
pub fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Spawn ordex for one file while optionally reusing `cache_root`.
pub fn open_session(file: &TempFile, cache_root: Option<&Path>) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("file path utf8")],
        PtySessionConfig {
            cache_root: cache_root.map(Path::to_path_buf),
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Wait until the editor is ready in normal mode.
pub fn wait_normal_mode(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait normal mode");
}
