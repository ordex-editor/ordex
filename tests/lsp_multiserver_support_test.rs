#![cfg_attr(target_os = "macos", expect(dead_code))]

use std::path::Path;
use std::process::Command;
use std::time::Duration;
use test_utils::{
    PtySession, PtySessionConfig, TempTree, command_path,
    filtered_path_with_real_binaries as build_filtered_path, spawn_lsp_session_with_config,
};

/// Store one parsed semantic version for external tool assertions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ToolVersion {
    /// Hold the major component of the parsed tool version.
    major: u32,
    /// Hold the minor component of the parsed tool version.
    minor: u32,
    /// Hold the patch component of the parsed tool version.
    patch: u32,
}

/// Return the compiled ordex binary path for PTY-backed LSP tests.
fn ordex_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ordex")
}

/// Assert that one required LSP binary exists on `PATH`.
#[track_caller]
fn assert_command_available(binary: &str) {
    assert!(
        command_path(binary).is_some(),
        "required LSP binary not found on PATH: {binary}"
    );
}

/// Parse one semver-looking token into a normalized tool version.
///
/// Returns `Some(version)` when `token` contains exactly three numeric semver
/// components after removing packaging wrappers. Returns `None` when `token`
/// does not contain a parseable `major.minor.patch` version.
fn parse_tool_version(token: &str) -> Option<ToolVersion> {
    // Debian packages may add an epoch and distro suffix, while upstream tools
    // often prefix releases with `v`, so strip those wrappers before parsing.
    let token_without_epoch = token.rsplit(':').next().unwrap_or(token);
    // Upstream `gopls` tags usually start with `v`, but distro metadata does not.
    let token_without_prefix = token_without_epoch
        .strip_prefix('v')
        .unwrap_or(token_without_epoch);
    // Debian revisions and distro suffixes start after `+` or `-`, so keep the
    // semver core before splitting it into numeric components.
    let core = token_without_prefix.split(['+', '-']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(ToolVersion {
        major,
        minor,
        patch,
    })
}

/// Run one command and return trimmed stdout when it succeeds.
///
/// Returns `Some(stdout)` when the command launches, exits successfully, and
/// emits UTF-8 stdout. Returns `None` when the command cannot be started, exits
/// with failure, or produces non-UTF-8 stdout.
fn command_stdout(binary: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(binary).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(stdout.trim().to_string())
}

/// Detect the installed `gopls` version from its own output or the Debian package.
///
/// Returns `Some(version)` when either source yields a parseable `gopls`
/// version. Returns `None` when neither the tool output nor the Debian package
/// metadata exposes a supported semver token.
fn detect_gopls_version() -> Option<ToolVersion> {
    // Ubuntu's `gopls version` output can report `(unknown)`, so search both the
    // tool output and the package metadata for the first semver token.
    let version_sources = [
        command_stdout("gopls", &["version"]),
        command_stdout("dpkg-query", &["-W", "-f=${Version}\n", "gopls"]),
    ];
    for source in version_sources.into_iter().flatten() {
        for token in source.split_whitespace() {
            if let Some(version) = parse_tool_version(token) {
                return Some(version);
            }
        }
    }
    None
}

/// Assert that the installed `gopls` version matches the standalone Go fixture.
#[track_caller]
fn assert_supported_gopls_version() {
    let installed = detect_gopls_version()
        .unwrap_or_else(|| panic!("required LSP binary version could not be determined: gopls"));
    let minimum = ToolVersion {
        major: 0,
        minor: 16,
        patch: 2,
    };
    let maximum = ToolVersion {
        major: 0,
        minor: 19,
        patch: 0,
    };
    assert!(
        installed >= minimum && installed < maximum,
        "unsupported gopls version for standalone Go LSP test: found {}.{}.{} but expected >= {}.{}.{} and < {}.{}.{}",
        installed.major,
        installed.minor,
        installed.patch,
        minimum.major,
        minimum.minor,
        minimum.patch,
        maximum.major,
        maximum.minor,
        maximum.patch
    );
}

/// Spawn Ordex for one file with the supplied `PATH` override.
fn spawn_session_with_path(file_path: &Path, current_dir: &Path, path_env: String) -> PtySession {
    spawn_lsp_session_with_config(
        ordex_bin(),
        &[file_path.to_path_buf()],
        PtySessionConfig {
            current_dir: Some(current_dir.to_path_buf()),
            env: vec![("PATH".to_string(), path_env)],
            ..Default::default()
        },
    )
    .expect("spawn ordex")
}

/// Build one standalone Python workspace without project markers.
fn standalone_python_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    tree.write_file(
        "main.py",
        "import os\nprint(missing_name)\n\ndef helper():\n    return 1\n\nhelper()\n",
    )
    .expect("write main.py");
    tree
}

/// Build one standalone C++ workspace without project markers.
fn standalone_cpp_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    tree.write_file("main.cpp", "int main() {\n    return missing_name;\n}\n")
        .expect("write main.cpp");
    tree
}

/// Build one standalone JavaScript workspace for `typescript-language-server`.
fn standalone_javascript_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    // A package marker exercises the shared JS/TS root-detection path instead
    // of the standalone-directory fallback used by other integration tests.
    tree.write_file("package.json", "{\n  \"name\": \"ordex-js-test\"\n}\n")
        .expect("write package.json");
    tree.write_file(
        "main.js",
        "function helper() {\n    console.log(\"ok\");\n}\n\nhelper();\n",
    )
    .expect("write main.js");
    tree
}

/// Build one standalone TypeScript workspace for `typescript-language-server`.
fn standalone_typescript_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    // The TypeScript server should see an ordinary project root and a typed
    // source file without needing any editor-specific configuration.
    tree.write_file(
        "tsconfig.json",
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\"\n  }\n}\n",
    )
    .expect("write tsconfig.json");
    tree.write_file(
        "main.ts",
        "function helper(): void {\n    console.log(\"ok\");\n}\n\nhelper();\n",
    )
    .expect("write main.ts");
    tree
}

/// Build one standalone Go workspace for `gopls`.
fn standalone_go_workspace() -> TempTree {
    let tree = TempTree::new().expect("temp tree");
    // A real module root keeps the fixture aligned with `gopls` defaults while
    // still keeping the navigation scenario small and deterministic.
    tree.write_file("go.mod", "module example.com/ordex-test\n\ngo 1.20\n")
        .expect("write go.mod");
    tree.write_file(
        "main.go",
        "package main\n\nfunc helper() {\n}\n\nfunc main() {\n    helper()\n}\n",
    )
    .expect("write main.go");
    tree
}

/// Build a PATH directory that exposes only the selected real binaries.
fn filtered_path_with_real_binaries(tree: &TempTree, binaries: &[&str]) -> String {
    build_filtered_path(tree, binaries)
}

/// Move the cursor to the Python `helper()` call so `gd` resolves its definition.
fn focus_python_helper_call(session: &mut PtySession) {
    session.send_text("/helper\\(\\)").expect("search helper");
    session.send_enter().expect("confirm helper search");
    // The first match is the function definition, so advance once to the call site.
    session.send_text("n").expect("move to helper call");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("7/7:1")
        })
        .expect("wait for helper call");
}

/// Move the cursor to one unique helper call located by a literal search.
fn focus_unique_helper_call(session: &mut PtySession, search: &str, expected_status: &str) {
    session.send_text(search).expect("search helper call");
    session.send_enter().expect("confirm helper search");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains(expected_status)
        })
        .expect("wait for helper call");
}

/// Move the cursor to the second `helper()` match so `gd` resolves the definition.
fn focus_second_helper_call(session: &mut PtySession, expected_status: &str) {
    session.send_text("/helper\\(\\)").expect("search helper");
    session.send_enter().expect("confirm helper search");
    // The declaration appears before the call site, so advance once after the
    // search to put the cursor on the invocation used for navigation.
    session.send_text("n").expect("move to helper call");
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains(expected_status)
        })
        .expect("wait for helper call");
}

/// Verify a standalone Python file uses real `ty` navigation and real `ruff` diagnostics.
#[test]
fn test_standalone_python_file_uses_real_ty_and_ruff() {
    assert_command_available("ty");
    assert_command_available("ruff");

    let workspace = standalone_python_workspace();
    let main_py = workspace.path().join("main.py");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_py, workspace.path(), path_env);

    // Wait for the standalone file to open before checking background diagnostics.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "import os")
        })
        .expect("wait for main.py");

    // The merged diagnostics view should include findings from both real servers.
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(1, "●") && screen.row_contains(2, "●")
        })
        .expect("wait for standalone Python diagnostics");
    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker");
    session.send_enter().expect("execute diagnostics command");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.contains("Ruff") && screen.contains("ty") && screen.contains("missing_name")
        })
        .expect("diagnostics picker should show Ruff and ty entries");
    session.exit_to_normal_mode(Duration::from_secs(2));

    // The same buffer should also resolve go-to-definition through ty.
    focus_python_helper_call(&mut session);
    session.send_text("gd").expect("request Python definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_trimmed_ends_with(4, "def helper():") && screen.status_line_contains("4/7:5")
        })
        .expect("definition should jump to helper");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone Python file falls back to real `pylsp` navigation when `ty` is absent.
#[test]
#[ignore]
fn test_standalone_python_file_falls_back_to_real_pylsp() {
    assert_command_available("pylsp");
    assert_command_available("ruff");

    let workspace = standalone_python_workspace();
    let main_py = workspace.path().join("main.py");
    let path_env = filtered_path_with_real_binaries(&workspace, &["pylsp", "ruff"]);
    let mut session = spawn_session_with_path(&main_py, workspace.path(), path_env);

    // Wait for the standalone file to open before issuing navigation requests.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ") && screen.row_trimmed_ends_with(1, "import os")
        })
        .expect("wait for main.py");

    // `ty` is absent from PATH here, so the definition lookup should fall back to pylsp.
    focus_python_helper_call(&mut session);
    session.send_text("gd").expect("request Python definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_trimmed_ends_with(4, "def helper():") && screen.status_line_contains("4/7:5")
        })
        .expect("definition should jump to helper through pylsp");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone C++ file uses real `clangd` without project markers.
#[test]
fn test_standalone_cpp_file_uses_real_clangd() {
    assert_command_available("clangd");

    let workspace = standalone_cpp_workspace();
    let main_cpp = workspace.path().join("main.cpp");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_cpp, workspace.path(), path_env);

    // Wait for the standalone C++ file to open before checking diagnostics.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "int main() {")
        })
        .expect("wait for main.cpp");

    // A real clangd diagnostic confirms the fallback root is sufficient to start the server.
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_contains(2, "●")
        })
        .expect("wait for clangd diagnostics");
    session
        .send_text(":diagnostics")
        .expect("open diagnostics picker");
    session.send_enter().expect("execute diagnostics command");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.contains("Use of undeclared identifier") && screen.contains("clang")
        })
        .expect("diagnostics picker should show clangd output");
    session.exit_to_normal_mode(Duration::from_secs(2));

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone JavaScript file uses real `typescript-language-server`.
#[test]
#[ignore]
fn test_standalone_javascript_file_uses_real_typescript_language_server() {
    assert_command_available("typescript-language-server");

    let workspace = standalone_javascript_workspace();
    let main_js = workspace.path().join("main.js");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_js, workspace.path(), path_env);

    // The fixture should open cleanly before the search and navigation steps
    // start driving the live language server through the PTY session.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "function helper() {")
        })
        .expect("wait for main.js");

    focus_unique_helper_call(&mut session, "/helper\\(\\);", "5/5:1");
    session
        .send_text("gd")
        .expect("request JavaScript definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_trimmed_ends_with(1, "function helper() {")
                && screen.status_line_contains("1/5:10")
        })
        .expect("definition should jump to helper through typescript-language-server");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone TypeScript file uses real `typescript-language-server`.
#[test]
#[ignore]
fn test_standalone_typescript_file_uses_real_typescript_language_server() {
    assert_command_available("typescript-language-server");

    let workspace = standalone_typescript_workspace();
    let main_ts = workspace.path().join("main.ts");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_ts, workspace.path(), path_env);

    // This mirrors the JavaScript test but keeps TypeScript-specific parsing in
    // the loop so the shared server route is exercised for both syntax ids.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "function helper(): void {")
        })
        .expect("wait for main.ts");

    focus_unique_helper_call(&mut session, "/helper\\(\\);", "5/5:1");
    session
        .send_text("gd")
        .expect("request TypeScript definition");
    session
        .wait_until(Duration::from_secs(20), |screen| {
            screen.row_trimmed_ends_with(1, "function helper(): void {")
                && screen.status_line_contains("1/5:10")
        })
        .expect("definition should jump to helper through typescript-language-server");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}

/// Verify a standalone Go file uses real `gopls`.
// NOTE: the macOS CI does not have the correct version of gopls, so we ignore this test on macOS.
#[cfg(not(target_os = "macos"))]
#[test]
#[ignore]
fn test_standalone_go_file_uses_real_gopls() {
    assert_command_available("gopls");
    // This fixture is only validated against the `gopls` window that reliably
    // builds package metadata for a tiny standalone module rooted by `go.mod`.
    // The guard fails fast if CI upgrades outside that tested range.
    assert_supported_gopls_version();

    let workspace = standalone_go_workspace();
    let main_go = workspace.path().join("main.go");
    let path_env = std::env::var("PATH").expect("read PATH");
    let mut session = spawn_session_with_path(&main_go, workspace.path(), path_env);

    // A small module-scoped fixture is enough to prove that the new Go route
    // starts `gopls`, synchronizes the file, and resolves definitions.
    session
        .wait_until(Duration::from_secs(2), |screen| {
            screen.status_line_contains("NORMAL ")
                && screen.row_trimmed_ends_with(1, "package main")
        })
        .expect("wait for main.go");

    focus_second_helper_call(&mut session, "7/8:5");
    session.send_text("gd").expect("request Go definition");
    session
        .wait_until(Duration::from_secs(45), |screen| {
            screen.row_trimmed_ends_with(3, "func helper() {")
                && screen.status_line_contains("3/8:6")
        })
        .expect("definition should jump to helper through gopls");

    session.send_text(":q!").expect("quit");
    session.send_enter().expect("execute quit");
    session
        .wait_for_exit_success(Duration::from_secs(2))
        .expect("quit cleanly");
}
