use std::time::Duration;
use test_utils::{PtySession, TempFile};

/// Return the test-built ordex binary path.
pub fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Spawn ordex for one file without an explicit config file.
pub fn open_session(file: &TempFile) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("file path utf8")],
        Default::default(),
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
