//! rust-analyzer-specific LSP behavior helpers.

use std::path::Path;

/// Return whether `server_command` refers to the rust-analyzer binary.
///
/// Returns `true` when the configured server command resolves to
/// `rust-analyzer`, and `false` for every other language server.
pub(super) fn is_rust_analyzer(server_command: &Path) -> bool {
    server_command
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == "rust-analyzer")
}

/// Return whether `error` matches rust-analyzer's transient startup rename failure.
///
/// Returns `true` when rust-analyzer reports the known startup-time
/// "No references found at position" rename error, and `false` for all other
/// server messages.
pub(super) fn is_startup_missing_references_error(server_command: &Path, error: &str) -> bool {
    is_rust_analyzer(server_command)
        && error
            .to_ascii_lowercase()
            .contains("no references found at position")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_startup_missing_references_error_is_rust_analyzer_specific() {
        assert!(is_startup_missing_references_error(
            &PathBuf::from("rust-analyzer"),
            "No references found at position"
        ));
        assert!(!is_startup_missing_references_error(
            &PathBuf::from("pylsp"),
            "No references found at position"
        ));
        assert!(!is_startup_missing_references_error(
            &PathBuf::from("rust-analyzer"),
            "different error"
        ));
    }
}
