use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use test_utils::{PtySession, ScreenSnapshot, TempTree, spawn_lsp_session};

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Return one fixture path relative to the repository root.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Return whether the LSP progress footer is absent from the current screen.
fn overlay_footer_hidden(screen: &ScreenSnapshot) -> bool {
    (24..=27).all(|row| !screen.row_contains(row, "rust-analyzer"))
}

/// Wait until startup analysis has visibly settled for the active LSP session.
fn wait_for_startup_analysis_to_settle(session: &mut PtySession) {
    // Startup progress can begin after the first render, so accept both the
    // already-idle case and the ordinary visible-progress path.
    let _ = session.wait_until(Duration::from_secs(8), |screen| {
        (24..=27).any(|row| screen.row_contains(row, "rust-analyzer"))
    });
    // Rust-analyzer may briefly hide the footer between startup phases, so
    // require several consecutive idle samples before treating startup as done.
    for _ in 0..5 {
        session
            .wait_until(Duration::from_secs(12), |screen| {
                overlay_footer_hidden(screen) && !screen.status_line_contains("● ")
            })
            .expect("startup analysis should settle without diagnostics");
        thread::sleep(Duration::from_millis(200));
    }
}

/// Return whether one line shows an active diagnostic with the expected message.
///
/// Returns `true` when the gutter marker, status-line summary, and a matching
/// diagnostic message are all visible after the progress footer clears, and
/// `false` otherwise.
fn diagnostic_visible(screen: &ScreenSnapshot, line: usize, message_fragment: &str) -> bool {
    overlay_footer_hidden(screen)
        && screen.row_contains(line, "●")
        && screen.status_line_contains("● ")
        && screen.contains(message_fragment)
}

/// Return whether one line no longer shows any active diagnostics.
///
/// Returns `true` when the gutter marker and status-line summary are both gone
/// after the progress footer clears, and `false` otherwise.
fn diagnostic_cleared(screen: &ScreenSnapshot, line: usize) -> bool {
    overlay_footer_hidden(screen)
        && !screen.row_contains(line, "●")
        && !screen.status_line_contains("● ")
}

/// Return whether one line shows at least one visible diagnostic summary.
///
/// Returns `true` when the gutter marker and the status-line summary are both
/// visible after the progress footer clears, and `false` otherwise.
fn line_diagnostic_visible(screen: &ScreenSnapshot, line: usize) -> bool {
    overlay_footer_hidden(screen)
        && screen.row_contains(line, "●")
        && screen.status_line_contains("● ")
}

/// Return whether one rendered file line shows a diagnostic summary with the expected text.
///
/// Returns `true` when the screen already contains the rendered line text plus
/// the status-line summary, and `false` when either surface is still missing.
fn rendered_line_diagnostic_summary_visible(screen: &ScreenSnapshot, rendered_line: &str) -> bool {
    screen.contains(rendered_line) && screen.status_line_contains("● ")
}

/// Build one temporary Cargo workspace with two diagnostics that appear after save.
fn diagnostic_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"diag_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n    let _ = missing_one;\n    let _ = missing_two;\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Build one temporary Cargo workspace without startup diagnostics.
fn clean_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"clean_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n    let used = 1;\n    let _ = used;\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Build one temporary Cargo workspace that matches the trailing-expression reproducer.
fn hello_world_workspace() -> TempTree {
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

/// Build one temporary Cargo workspace for save-triggered semantic diagnostics.
fn semantic_diagnostics_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp workspace");
    // Keep the fixture minimal so save-triggered semantic diagnostics settle quickly.
    tree.write_file(
        "Cargo.toml",
        "[package]\nname = \"semantic_diag_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "use std::collections::HashMap;\n\nfn main() {\n    let used = 1;\n    let _ = used;\n    let _map: HashMap<String, String> = HashMap::new();\n}\n",
    )
    .expect("write main.rs");
    tree
}

/// Build one temporary Cargo workspace that reproduces the insert-then-save freeze.
fn startup_insert_save_freeze_workspace() -> TempTree {
    let source_root = fixture_path("tests/fixtures/lsp/workspace_one");
    let tree = TempTree::new().expect("temp workspace");
    tree.write_file(
        "Cargo.toml",
        &fs::read_to_string(source_root.join("Cargo.toml")).expect("read Cargo.toml"),
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "src/main.rs",
        "fn main() {\n  println!(\"Hello, world!\");\ngarbage\n}\n",
    )
    .expect("write main.rs");
    tree.write_file(
        "src/lib.rs",
        &fs::read_to_string(source_root.join("src/lib.rs")).expect("read lib.rs"),
    )
    .expect("write lib.rs");
    tree
}

/// Build one temporary workspace with Ordex's real source tree to match repo-sized indexing.
fn repo_sized_workspace() -> TempTree {
    /// Copy every file under `source_dir` into the temp tree using repo-relative paths.
    fn copy_tree(tree: &TempTree, repo_root: &std::path::Path, source_dir: &std::path::Path) {
        for entry in fs::read_dir(source_dir).expect("read source directory") {
            let entry = entry.expect("read source entry");
            let path = entry.path();
            if path.is_dir() {
                // Recurse through the full module tree so rust-analyzer indexes the
                // same source layout as the checked-out repository.
                copy_tree(tree, repo_root, &path);
                continue;
            }
            let relative = path
                .strip_prefix(repo_root)
                .expect("strip repo prefix")
                .to_str()
                .expect("utf8 source path");
            let Ok(contents) = fs::read_to_string(&path) else {
                // Test fixtures can contain binary snapshots that are irrelevant to
                // `cargo check`, so copy only UTF-8 sources into the temp workspace.
                continue;
            };
            tree.write_file(relative, &contents)
                .expect("write repo source file");
        }
    }

    let tree = TempTree::new().expect("temp workspace");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    tree.write_file(
        "Cargo.toml",
        &fs::read_to_string(repo_root.join("Cargo.toml")).expect("read repo Cargo.toml"),
    )
    .expect("write Cargo.toml");
    tree.write_file(
        "Cargo.lock",
        &fs::read_to_string(repo_root.join("Cargo.lock")).expect("read repo Cargo.lock"),
    )
    .expect("write Cargo.lock");
    copy_tree(&tree, &repo_root, &repo_root.join("src"));
    copy_tree(&tree, &repo_root, &repo_root.join("crates/test_utils"));
    copy_tree(&tree, &repo_root, &repo_root.join("tests"));
    tree
}

/// Verify save-triggered diagnostics render, list in the picker, and support navigation.
#[test]
fn test_lsp_diagnostics_render_list_and_navigate() {
    let workspace = diagnostic_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");
    wait_for_startup_analysis_to_settle(&mut session);
    session.send_text(":w").expect("save diagnostic fixture");
    session.send_enter().expect("confirm save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(2, "●")
                && screen.row_contains(3, "●")
                && screen.contains("missing_one")
        })
        .expect("saved diagnostics should render");

    session.send_text("]d").expect("jump to first diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("2:13") && screen.contains("missing_one")
        })
        .expect("next diagnostic should jump to missing_one");

    session.send_text("]d").expect("jump to second diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("3:13") && screen.contains("missing_two")
        })
        .expect("next diagnostic should jump to missing_two");

    session
        .send_text("[d")
        .expect("jump back to first diagnostic");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.status_line_contains("2:13") && screen.contains("missing_one")
        })
        .expect("previous diagnostic should jump back to missing_one");

    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker command");
    session.send_enter().expect("confirm diagnostics command");
    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.contains("Diagnostics")
                && screen.contains("missing_one")
                && screen.contains("missing_two")
        })
        .expect("diagnostics picker should list both saved diagnostics");

    session.exit_to_normal_mode(Duration::from_secs(2));
}

/// Verify live `didChange` updates remove diagnostics after in-memory edits.
#[test]
fn test_lsp_diagnostics_refresh_after_edit() {
    let workspace = clean_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    session
        .send_text("GkOlet broken = ;")
        .expect("insert one parse error before closing brace");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save broken file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(20), |screen| {
            diagnostic_visible(screen, 4, "expected expression")
        })
        .expect("save should surface diagnostics before the local fix");

    session.send_text("dd").expect("delete invalid line");
    session
        .wait_until(Duration::from_secs(12), |screen| {
            diagnostic_cleared(screen, 4)
        })
        .expect("live diagnostics should disappear after the local edit");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify one saved trailing expression still surfaces diagnostics.
#[test]
fn test_lsp_diagnostics_appear_after_saved_trailing_expression_edit() {
    let workspace = hello_world_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    // Insert an incomplete trailing expression inside `main`, then save it.
    // A parser error is stable here, while the unresolved-name variant depends
    // on slower semantic analysis that does not publish reliably in CI.
    session
        .send_text("ggjA\n1 +")
        .expect("insert one incomplete trailing expression");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save edited file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            screen.row_contains(3, "1 +")
                && screen.status_line_contains("● 1")
                && overlay_footer_hidden(screen)
        })
        .expect("saved diagnostics should appear for the trailing expression");
    // Restart from the first column so diagnostic navigation reaches whichever
    // line rust-analyzer reports for the saved parser error.
    session
        .send_text("gg0]d")
        .expect("jump to the saved trailing-expression diagnostic");
    session
        .wait_until(Duration::from_secs(12), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(3, "1 +")
                && screen.status_line_contains("● 1")
                && screen.contains("expected expression")
                && (screen.row_contains(3, "●") || screen.row_contains(4, "●"))
        })
        .expect("diagnostic navigation should surface the trailing-expression error");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify save-triggered diagnostics disappear after a saved fix.
#[test]
fn test_lsp_diagnostics_refresh_after_save_fix() {
    let workspace = clean_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    session
        .send_text("GkOlet broken = ;")
        .expect("insert one parse error before closing brace");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save fix");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(12), |screen| {
            screen.row_contains(4, "●") && overlay_footer_hidden(screen)
        })
        .expect("save-triggered diagnostics should appear");

    session.send_text("dd").expect("delete invalid line");
    session.send_text(":w").expect("save repaired file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for second write confirmation");

    session
        .wait_until(Duration::from_secs(12), |screen| {
            overlay_footer_hidden(screen) && !screen.row_contains(4, "●")
        })
        .expect("save-triggered diagnostics should clear after the fix");

    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker command");
    session.send_enter().expect("confirm diagnostics command");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("No diagnostics in active buffer")
        })
        .expect("diagnostics picker should report an empty buffer");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify save-triggered diagnostics appear and remain visible after progress clears.
#[test]
fn test_lsp_diagnostics_appear_after_save_and_persist_after_analysis() {
    let workspace = clean_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    session
        .send_text("GkOlet broken = ;")
        .expect("insert one parse error before closing brace");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save new warning");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            diagnostic_visible(screen, 4, "expected expression")
        })
        .expect("save-triggered diagnostics should remain visible after analysis");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify one saved `unused_mut` warning appears quickly for a small Rust file.
#[test]
fn test_lsp_diagnostics_warning_appears_quickly_after_save() {
    let workspace = semantic_diagnostics_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "use std::collections::HashMap;")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    // Save one semantic warning without introducing a second unused-variable warning.
    session
        .send_text("GkO    let mut value = true;\n    let _ = value;")
        .expect("insert one saved warning");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save warning");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(15), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(7, "●")
                && screen.status_line_contains("● 1")
        })
        .expect("saved warning should appear quickly");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify one saved `let mut value = 10;` warning appears quickly after startup settles.
#[test]
fn test_lsp_diagnostics_warning_appears_quickly_after_save_in_hello_world() {
    for _ in 0..3 {
        // Each fresh workspace forces rust-analyzer through the same save pipeline
        // so the flaky post-save warning path has to behave reliably every time.
        let workspace = hello_world_workspace();
        let main_rs = workspace.path().join("src/main.rs");
        let mut session =
            spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
            })
            .expect("wait for main.rs");

        wait_for_startup_analysis_to_settle(&mut session);

        // Save the exact warning reproducer after rust-analyzer becomes idle.
        session
            .send_text("GkO    let mut")
            .expect("insert warning prefix");
        // Split the edit across multiple pauses so background sync can advance the
        // tracked protocol version before the final save request is queued.
        thread::sleep(Duration::from_millis(250));
        session
            .send_text(" value =")
            .expect("insert warning middle");
        thread::sleep(Duration::from_millis(250));
        session.send_text(" 10;").expect("insert warning suffix");
        thread::sleep(Duration::from_millis(250));
        session.exit_to_normal_mode(Duration::from_secs(2));
        session.send_text(":w").expect("save warning reproducer");
        session.send_enter().expect("execute save");
        session
            .wait_until(Duration::from_secs(4), |screen| {
                screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
            })
            .expect("wait for write confirmation");

        session
            .wait_until(Duration::from_secs(2), |screen| {
                line_diagnostic_visible(screen, 3)
            })
            .expect("saved hello-world warning should appear quickly");
        thread::sleep(Duration::from_secs(3));
        session
            .wait_until(Duration::from_secs(1), |screen| {
                line_diagnostic_visible(screen, 3)
            })
            .expect("saved hello-world warning should stay visible");

        session.send_text(":q!").expect("quit");
        session.send_enter().expect("execute quit");
        session
            .wait_for_exit_success(Duration::from_secs(2))
            .expect("quit cleanly");
    }
}

/// Verify the exact one-shot save repro shows the warning quickly and keeps it visible.
#[test]
fn test_lsp_diagnostics_warning_appears_quickly_after_immediate_save_in_hello_world() {
    for _ in 0..5 {
        // Each fresh workspace repeats the reported save path without giving the
        // background sync loop extra time to advance before the write happens.
        let workspace = hello_world_workspace();
        let main_rs = workspace.path().join("src/main.rs");
        let mut session =
            spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
            })
            .expect("wait for main.rs");

        wait_for_startup_analysis_to_settle(&mut session);

        // Match the manual repro closely: insert the whole binding in one shot and
        // save immediately after leaving insert mode.
        session
            .send_text("GkO    let mut value = 10;")
            .expect("insert warning reproducer");
        session.exit_to_normal_mode(Duration::from_secs(2));
        session.send_text(":w").expect("save warning reproducer");
        session.send_enter().expect("execute save");
        session
            .wait_until(Duration::from_secs(4), |screen| {
                screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
            })
            .expect("wait for write confirmation");

        session
            .wait_until(Duration::from_secs(5), |screen| {
                line_diagnostic_visible(screen, 3)
            })
            .expect("saved hello-world warning should appear quickly after immediate save");
        thread::sleep(Duration::from_secs(3));
        session
            .wait_until(Duration::from_secs(1), |screen| {
                line_diagnostic_visible(screen, 3)
            })
            .expect("saved hello-world warning should stay visible after immediate save");

        session.send_text(":q!").expect("quit");
        session.send_enter().expect("execute quit");
        session
            .wait_for_exit_success(Duration::from_secs(2))
            .expect("quit cleanly");
    }
}

/// Verify the immediate-save warning repro stays fast in a repo-sized workspace.
#[test]
fn test_lsp_diagnostics_warning_appears_quickly_after_immediate_save_in_repo_sized_workspace() {
    let workspace = repo_sized_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "#![allow")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    // Match the reported repro in the heavier workspace without interleaved pauses.
    session
        .send_text("GkO    let mut value = 10;")
        .expect("insert warning reproducer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save warning reproducer");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    let warning_wait_started = std::time::Instant::now();
    session
        .wait_until(Duration::from_secs(15), |screen| {
            rendered_line_diagnostic_summary_visible(screen, "● 39     let mut value = 10;")
        })
        .expect("repo-sized warning should eventually appear after immediate save");
    let warning_latency = warning_wait_started.elapsed();
    assert!(
        warning_latency <= Duration::from_secs(5),
        "repo-sized warning appeared too slowly after immediate save: {warning_latency:?}"
    );
    thread::sleep(Duration::from_secs(3));
    session
        .wait_until(Duration::from_secs(5), |screen| {
            overlay_footer_hidden(screen)
                && rendered_line_diagnostic_summary_visible(screen, "● 39     let mut value = 10;")
        })
        .expect("repo-sized warning should stay visible after immediate save");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify whether one no-op warmup save changes the repo-sized save-warning latency.
#[test]
fn test_lsp_diagnostics_warning_appears_quickly_after_warmup_save_in_repo_sized_workspace() {
    let workspace = repo_sized_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_contains(1, "#![allow")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    // Warm the save-driven rustc path once before measuring the exact repro.
    session.send_text(":w").expect("warm startup save");
    session.send_enter().expect("confirm warm startup save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for warm save confirmation");

    session
        .send_text("GkO    let mut value = 10;")
        .expect("insert warning reproducer");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save warning reproducer");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    let warning_wait_started = std::time::Instant::now();
    session
        .wait_until(Duration::from_secs(15), |screen| {
            rendered_line_diagnostic_summary_visible(screen, "● 39     let mut value = 10;")
        })
        .expect("repo-sized warning should eventually appear after warmup save");
    let warning_latency = warning_wait_started.elapsed();
    assert!(
        warning_latency <= Duration::from_secs(5),
        "repo-sized warning appeared too slowly after warmup save: {warning_latency:?}"
    );

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify one saved `HashMap::new()` error appears quickly and clears after removal.
#[test]
fn test_lsp_diagnostics_error_clears_quickly_after_saved_removal() {
    let workspace = semantic_diagnostics_workspace();
    let main_rs = workspace.path().join("src/main.rs");
    let mut session =
        spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_contains(1, "use std::collections::HashMap;")
        })
        .expect("wait for main.rs");

    wait_for_startup_analysis_to_settle(&mut session);

    // Save the semantic error directly so the regression focuses on gutter clearing.
    session
        .send_text("GkO    let value = HashMap::new();")
        .expect("insert error");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session.send_text(":w").expect("save warning and error");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for write confirmation");

    session
        .wait_until(Duration::from_secs(8), |screen| {
            overlay_footer_hidden(screen)
                && screen.row_contains(7, "●")
                && screen.status_line_contains("● 1")
        })
        .expect("saved error should appear quickly");

    session.send_text("gg0]d").expect("jump to saved error");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("7:9")
        })
        .expect("cursor should land on saved error");
    session.send_text("dd").expect("delete saved error line");
    session.send_text(":w").expect("save repaired file");
    session.send_enter().expect("execute save");
    session
        .wait_until(Duration::from_secs(4), |screen| {
            screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
        })
        .expect("wait for second write confirmation");

    session
        .wait_until(Duration::from_secs(4), |screen| {
            overlay_footer_hidden(screen)
                && !screen.row_contains(7, "HashMap::new()")
                && !screen.row_contains(7, "●")
                && !screen.status_line_contains("● ")
        })
        .expect("saved error should clear quickly after removal");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify the reported `jj`, `O`, `<Space>w` sequence stays responsive after startup settles.
#[test]
fn test_open_line_above_after_startup_settles_and_save_completes() {
    for _ in 0..5 {
        // Use a fresh workspace each time so rust-analyzer goes through its startup
        // analysis cycle before the edit and save sequence.
        let workspace = startup_insert_save_freeze_workspace();
        let main_rs = workspace.path().join("src/main.rs");
        let mut session =
            spawn_lsp_session(ordex_bin(), std::slice::from_ref(&main_rs)).expect("spawn ordex");

        // Save once so the initial diagnostic appears before waiting for the
        // background startup work to settle like the manual reproduction does.
        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.status_line_contains("NORMAL ") && screen.row_contains(1, "fn main() {")
            })
            .expect("wait for main.rs");
        wait_for_startup_analysis_to_settle(&mut session);
        session
            .send_text(":w")
            .expect("save startup diagnostic fixture");
        session.send_enter().expect("confirm save");
        session
            .wait_until(Duration::from_secs(4), |screen| {
                screen.message_line_contains("written") && screen.status_line_contains("NORMAL ")
            })
            .expect("wait for write confirmation");
        session
            .wait_until(Duration::from_secs(8), |screen| {
                screen.row_contains(3, "●") && screen.status_line_contains("● 1")
            })
            .expect("saved diagnostic should render");
        let _ = session.wait_until(Duration::from_secs(8), |screen| {
            (24..=27).any(|row| screen.row_contains(row, "rust-analyzer"))
        });
        session
            .wait_until(Duration::from_secs(8), |screen| {
                (24..=27).all(|row| !screen.row_contains(row, "rust-analyzer"))
            })
            .expect("startup analysis should finish");

        // Reproduce the exact editor interaction after startup analysis completes.
        // This intentionally avoids extra waits between Escape and save so the
        // save path overlaps the same rapid key sequence as the manual repro.
        session.send_text("jj").expect("move to garbage line");
        session
            .send_text("Olet val: i32 = String::new();")
            .expect("insert line above garbage");
        session.send_escape().expect("leave insert mode");
        session
            .send_text(" w")
            .expect("save through normal binding");
        session
            .wait_until(Duration::from_secs(2), |screen| {
                screen.contains("written") && !screen.status_line_contains("[+]")
            })
            .expect("save should complete without freezing");

        session.send_text(":q!").expect("quit");
        session.send_enter().expect("execute quit");
        session
            .wait_for_exit_success(Duration::from_secs(2))
            .expect("quit cleanly");
    }
}
