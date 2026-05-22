use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use test_utils::{PtySession, PtySessionConfig, TempFile, TempTree};

/// Return the test-built Ordex binary path.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Write one executable shell script into `tree/bin`.
fn write_stub_script(tree: &TempTree, name: &str, body: &str) -> PathBuf {
    let path = tree.path().join("bin").join(name);
    fs::create_dir_all(path.parent().expect("stub parent")).expect("create stub directory");
    fs::write(&path, body).expect("write stub script");
    let mut permissions = fs::metadata(&path).expect("stub metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("set stub permissions");
    path
}

/// Create one stub `wl-copy` command that stores stdin into the requested clipboard file.
fn write_wayland_copy_stub(tree: &TempTree) {
    write_stub_script(
        tree,
        "wl-copy",
        r#"#!/bin/sh
set -eu
target="${CLIPBOARD_TEST_PLUS:?}"
if [ "${1-}" = "--primary" ]; then
  target="${CLIPBOARD_TEST_PRIMARY:?}"
fi
/bin/cat > "$target"
"#,
    );
}

/// Create one stub `wl-paste` command that reads the requested clipboard file.
fn write_wayland_paste_stub(tree: &TempTree) {
    write_stub_script(
        tree,
        "wl-paste",
        r#"#!/bin/sh
set -eu
target="${CLIPBOARD_TEST_PLUS:?}"
for arg in "$@"; do
  if [ "$arg" = "--primary" ]; then
    if [ "${CLIPBOARD_TEST_PRIMARY_UNSUPPORTED:-0}" = "1" ]; then
      echo "primary unsupported" >&2
      exit 1
    fi
    target="${CLIPBOARD_TEST_PRIMARY:?}"
  fi
done
/bin/cat "$target"
"#,
    );
}

/// Create one stub `xclip` command that reads or writes the selected clipboard file.
fn write_xclip_stub(tree: &TempTree) {
    write_stub_script(
        tree,
        "xclip",
        r#"#!/bin/sh
set -eu
mode="write"
selection="clipboard"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      mode="read"
      ;;
    -selection)
      shift
      selection="$1"
      ;;
  esac
  shift
done
target="${CLIPBOARD_TEST_PLUS:?}"
if [ "$selection" = "primary" ]; then
  target="${CLIPBOARD_TEST_PRIMARY:?}"
fi
if [ "$mode" = "read" ]; then
  /bin/cat "$target"
else
  /bin/cat > "$target"
fi
"#,
    );
}

/// Return one environment list that points Ordex at the stub clipboard backend.
fn clipboard_env(
    tree: &TempTree,
    plus_path: &Path,
    primary_path: &Path,
    session_type: &str,
) -> Vec<(String, String)> {
    vec![
        (
            "PATH".to_string(),
            tree.path().join("bin").display().to_string(),
        ),
        ("XDG_SESSION_TYPE".to_string(), session_type.to_string()),
        (
            "WAYLAND_DISPLAY".to_string(),
            if session_type == "wayland" {
                "wayland-0".to_string()
            } else {
                String::new()
            },
        ),
        (
            "DISPLAY".to_string(),
            if session_type == "x11" {
                ":99".to_string()
            } else {
                String::new()
            },
        ),
        (
            "CLIPBOARD_TEST_PLUS".to_string(),
            plus_path.display().to_string(),
        ),
        (
            "CLIPBOARD_TEST_PRIMARY".to_string(),
            primary_path.display().to_string(),
        ),
    ]
}

/// Spawn Ordex with the provided clipboard-test environment.
fn spawn_clipboard_session(
    file: &TempFile,
    tree: &TempTree,
    env: Vec<(String, String)>,
) -> PtySession {
    PtySession::spawn(
        ordex_bin(),
        &[file.path().to_str().expect("utf8 file path")],
        PtySessionConfig {
            current_dir: Some(tree.path().to_path_buf()),
            env,
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Wait until Ordex finishes its initial Normal-mode render.
fn wait_normal_mode(session: &mut PtySession) {
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ")
        })
        .expect("wait normal mode");
}

/// Verify `<Space>-p` pastes from the Wayland clipboard register.
#[test]
fn test_space_dash_p_pastes_wayland_clipboard_after_cursor() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\n").expect("seed file");
    let tree = TempTree::new().expect("create temp tree");
    let plus_path = tree.path().join("clipboard.txt");
    let primary_path = tree.path().join("primary.txt");
    fs::write(&plus_path, "XYZ").expect("seed clipboard");
    fs::write(&primary_path, "").expect("seed primary");
    write_wayland_copy_stub(&tree);
    write_wayland_paste_stub(&tree);

    let env = clipboard_env(&tree, &plus_path, &primary_path, "wayland");
    let mut session = spawn_clipboard_session(&file, &tree, env);
    wait_normal_mode(&mut session);

    session
        .send_text("l -p")
        .expect("paste clipboard after cursor");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.status_line_contains("NORMAL ") && s.row_contains(1, "abXYZ")
        })
        .expect("clipboard paste rendered");

    session.send_text(":wq").expect("save and quit");
    session.send_enter().expect("execute wq");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("exit cleanly");

    assert_eq!(
        fs::read_to_string(file.path()).expect("read saved file"),
        "abXYZ\n"
    );
}

/// Verify `\"+yy` writes the yanked line into the Wayland clipboard register.
#[test]
fn test_quote_plus_yy_writes_wayland_clipboard() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");
    let tree = TempTree::new().expect("create temp tree");
    let plus_path = tree.path().join("clipboard.txt");
    let primary_path = tree.path().join("primary.txt");
    fs::write(&plus_path, "").expect("seed clipboard");
    fs::write(&primary_path, "").expect("seed primary");
    write_wayland_copy_stub(&tree);
    write_wayland_paste_stub(&tree);

    let env = clipboard_env(&tree, &plus_path, &primary_path, "wayland");
    let mut session = spawn_clipboard_session(&file, &tree, env);
    wait_normal_mode(&mut session);

    session
        .send_text("\"+yy")
        .expect("yank into clipboard register");
    session
        .wait_until(Duration::from_secs(2), |_| {
            fs::read_to_string(&plus_path)
                .ok()
                .is_some_and(|text| text == "alpha\n")
        })
        .expect("clipboard write completed");
}

/// Verify `\"*p` reports the requested explicit Wayland primary-selection failure.
#[test]
fn test_quote_star_p_reports_wayland_primary_selection_error() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"ab\n").expect("seed file");
    let tree = TempTree::new().expect("create temp tree");
    let plus_path = tree.path().join("clipboard.txt");
    let primary_path = tree.path().join("primary.txt");
    fs::write(&plus_path, "XYZ").expect("seed clipboard");
    fs::write(&primary_path, "PRIMARY").expect("seed primary");
    write_wayland_copy_stub(&tree);
    write_wayland_paste_stub(&tree);

    let mut env = clipboard_env(&tree, &plus_path, &primary_path, "wayland");
    env.push((
        "CLIPBOARD_TEST_PRIMARY_UNSUPPORTED".to_string(),
        "1".to_string(),
    ));
    let mut session = spawn_clipboard_session(&file, &tree, env);
    wait_normal_mode(&mut session);

    session.send_text("\"*p").expect("paste primary selection");
    session
        .wait_until(Duration::from_secs(2), |s| {
            s.message_line_contains("Wayland primary selection")
        })
        .expect("primary-selection error surfaced");
}

/// Verify the X11 backend writes the `\"*` register through `xclip`.
#[test]
fn test_quote_star_yy_writes_x11_primary_selection() {
    let file = TempFile::new().expect("create temp file");
    file.write_all(b"alpha\nbeta\n").expect("seed file");
    let tree = TempTree::new().expect("create temp tree");
    let plus_path = tree.path().join("clipboard.txt");
    let primary_path = tree.path().join("primary.txt");
    fs::write(&plus_path, "").expect("seed clipboard");
    fs::write(&primary_path, "").expect("seed primary");
    write_xclip_stub(&tree);

    let env = clipboard_env(&tree, &plus_path, &primary_path, "x11");
    let mut session = spawn_clipboard_session(&file, &tree, env);
    wait_normal_mode(&mut session);

    session
        .send_text("\"*yy")
        .expect("yank into primary register");
    session
        .wait_until(Duration::from_secs(2), |_| {
            fs::read_to_string(&primary_path)
                .ok()
                .is_some_and(|text| text == "alpha\n")
        })
        .expect("primary clipboard write completed");
}
