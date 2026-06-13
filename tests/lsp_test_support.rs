use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use test_utils::{PtySession, TempTree, overlay_footer_hidden};

/// Copy one fixture workspace into a unique temporary tree for test isolation.
#[allow(dead_code)]
pub fn isolated_fixture_workspace(relative_workspace: &str) -> TempTree {
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_workspace);
    let tree = TempTree::with_prefix("ordex_lsp_fixture_copy").expect("create fixture copy root");
    copy_workspace_tree(&source, tree.path()).expect("copy fixture workspace");
    tree
}

/// Copy one workspace directory recursively into the provided destination root.
fn copy_workspace_tree(source: &Path, destination: &Path) -> io::Result<()> {
    // Create the destination root before descending so nested entries can be
    // copied with direct path joins and without repeated existence checks.
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        let entry_name = entry.file_name();
        let destination_path = destination.join(&entry_name);

        if entry.file_type()?.is_dir() {
            // Recurse into each child directory so the isolated copy preserves
            // fixture-local Cargo metadata, source files, and build artifacts.
            copy_workspace_tree(&entry_path, &destination_path)?;
        } else {
            // Copy source files and lockfiles byte-for-byte so the isolated
            // workspace preserves the same LSP behavior as the fixture root.
            fs::copy(&entry_path, &destination_path)?;
        }
    }
    Ok(())
}

/// Move to `helper_value()` and wait until one hover request succeeds.
#[allow(dead_code)]
pub fn warm_up_helper_value_hover(session: &mut PtySession) {
    // CI can start these PTY tests while the language server is still building
    // the initial workspace graph, so the warmup tolerates one cold-start pass.
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        session
            .send_text("/helper_value\\(\\)")
            .expect("search for warmup symbol");
        session.send_enter().expect("confirm warmup search");
        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.status_line_contains("4/15:13")
            })
            .expect("cursor should land on the warmup helper_value call");
        session.send_text("K").expect("request warmup hover");
        if session
            .wait_until(Duration::from_secs(4), |screen| {
                screen.contains("Hover") && screen.contains("fn helper_value() -> i32")
            })
            .is_ok()
        {
            session.send_text("j").expect("dismiss warmup hover");
            session
                .wait_until(Duration::from_secs(2), |screen| {
                    screen.row_trimmed_ends_with(5, "    let _ = local_value();")
                        && screen.status_line_contains("5/15:13")
                })
                .expect("warmup hover should dismiss before moving down");
            wait_for_lsp_progress_to_finish(session);
            return;
        }
        // Retry the hover request until the workspace analysis is ready enough to
        // answer symbol lookups reliably for the shared fixture project.
        assert!(Instant::now() < deadline, "warmup hover should succeed");
        thread::sleep(Duration::from_millis(100));
    }
}

/// Wait until one PTY condition stays true for a short stability window.
pub fn wait_until_stable<F>(
    session: &mut PtySession,
    timeout: Duration,
    stable_for: Duration,
    mut condition: F,
) -> io::Result<()>
where
    F: FnMut(&test_utils::ScreenSnapshot) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut first_match_at = None;
    let mut last_snapshot = session.snapshot();
    while Instant::now() < deadline {
        // Pull fresh PTY output before checking the next rendered frame.
        session.read_available()?;
        let snapshot = session.snapshot();
        last_snapshot = snapshot.clone();
        if condition(&snapshot) {
            let matched_at = *first_match_at.get_or_insert_with(Instant::now);
            // Only accept the state after it has remained visible long enough to
            // outlive one transient redraw or in-flight async completion refresh.
            if Instant::now().saturating_duration_since(matched_at) >= stable_for {
                return Ok(());
            }
        } else {
            first_match_at = None;
        }
        thread::sleep(Duration::from_millis(10));
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "condition not stable before timeout; snapshot:\n{}",
            last_snapshot.raw()
        ),
    ))
}

/// Wait until the LSP progress overlay is no longer visible.
pub fn wait_for_lsp_progress_to_finish(session: &mut PtySession) {
    wait_until_stable(
        session,
        Duration::from_secs(10),
        Duration::from_millis(100),
        overlay_footer_hidden,
    )
    .expect("LSP progress overlay should clear");
}
