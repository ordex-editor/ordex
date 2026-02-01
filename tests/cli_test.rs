use std::process::Command;
use std::io::Write;

#[test]
fn test_no_arguments_shows_usage() {
    let output = Command::new("cargo")
        .args(&["run", "--quiet", "--"])
        .output()
        .expect("Failed to run binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage:"));
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_nonexistent_file_error() {
    let output = Command::new("cargo")
        .args(&["run", "--quiet", "--", "/nonexistent/file.txt"])
        .output()
        .expect("Failed to run binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Error:") || stderr.contains("not found"));
    assert_ne!(output.status.code(), Some(0));
}

#[test]
fn test_loads_existing_file() {
    // Create a temporary test file
    let mut file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    writeln!(file, "Test line 1").expect("Failed to write");
    writeln!(file, "Test line 2").expect("Failed to write");

    let output = Command::new("cargo")
        .args(&["run", "--quiet", "--", file.path().to_str().unwrap()])
        .output()
        .expect("Failed to run binary");

    // Terminal operations may fail in test environment without TTY
    // Accept either success or "inappropriate ioctl" error
    let stderr = String::from_utf8_lossy(&output.stderr);
    let has_tty_error = stderr.contains("Inappropriate ioctl") ||
                        stderr.contains("not a tty") ||
                        stderr.contains("ENOTTY");

    // Should either succeed or fail with expected TTY error
    assert!(
        output.status.code() == Some(0) || has_tty_error,
        "Unexpected failure: stderr={}", stderr
    );
}
