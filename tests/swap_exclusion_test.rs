mod config_test_support;
mod swap_test_support;

use std::time::Duration;
use test_utils::TempFile;

#[test]
fn excludes_matching_paths_from_swap_creation() {
    let file = TempFile::with_suffix(".gpg").expect("create temp file");
    file.write_all(b"secret").expect("seed file");
    let config = config_test_support::write_config(
        r#"
[swap]
exclude = ["*.gpg"]
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("ix").expect("edit excluded file");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "xsecret")
        })
        .expect("wait for edit");
    assert!(
        !swap_test_support::compute_swap_path(session.cache_root(), file.path()).exists(),
        "excluded path should not create a swap file"
    );
    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

#[test]
fn keeps_swap_creation_for_non_matching_paths() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"notes").expect("seed file");
    let config = config_test_support::write_config(
        r#"
[swap]
exclude = ["*.gpg"]
"#,
    );

    let mut session = config_test_support::open_session_with_config(&file, &config);
    config_test_support::wait_normal_mode(&mut session);
    session.send_text("ix").expect("edit included file");
    session.exit_to_normal_mode(Duration::from_secs(2));
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.row_trimmed_ends_with(1, "xnotes")
        })
        .expect("wait for edit");
    swap_test_support::wait_for_swap_file(session.cache_root(), file.path());
    swap_test_support::wait_for_swap_body(session.cache_root(), file.path(), "xnotes");
    session.send_text(":q!").expect("force quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
