//! Resolve language-profile-driven corresponding file paths.

use crate::syntax::profiles::{corresponding_extensions_for, detect_language_details};
use std::path::{Path, PathBuf};

/// Why corresponding-file resolution failed for the current buffer path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CorrespondingFileError {
    /// The active buffer does not have a file-system name.
    NoFileName,
    /// The active file has no extension or no built-in profile.
    UnsupportedFileType,
    /// The active profile does not define a corresponding extension rule.
    UnsupportedExtension,
    /// At least one corresponding extension exists, but no candidate file exists on disk.
    NotFound,
}

impl CorrespondingFileError {
    /// Return the status-line message that should be shown to the user.
    pub(crate) fn status_message(&self) -> &'static str {
        match self {
            Self::NoFileName => "No file name",
            Self::UnsupportedFileType | Self::UnsupportedExtension => {
                "No corresponding file mapping for this file type"
            }
            Self::NotFound => "No corresponding file found",
        }
    }
}

/// Resolve one existing same-directory corresponding file path.
pub(crate) fn find_corresponding_file_path(
    current_file: &Path,
) -> Result<PathBuf, CorrespondingFileError> {
    if current_file.file_name().is_none() {
        return Err(CorrespondingFileError::NoFileName);
    }
    let extension = current_file
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or(CorrespondingFileError::UnsupportedFileType)?;
    let stem = current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or(CorrespondingFileError::UnsupportedFileType)?;
    let (profile, _) = detect_language_details(Some(current_file))
        .ok_or(CorrespondingFileError::UnsupportedFileType)?;
    let target_extensions = corresponding_extensions_for(profile, extension)
        .ok_or(CorrespondingFileError::UnsupportedExtension)?;

    // Keep lookup scoped to the active file's directory so behavior is
    // predictable and independent of project-specific folder conventions.
    let candidates = build_same_directory_candidates(current_file, stem, target_extensions);
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or(CorrespondingFileError::NotFound)
}

/// Build ordered same-directory candidate paths for one `stem`.
fn build_same_directory_candidates(
    current_file: &Path,
    stem: &str,
    target_extensions: &[&str],
) -> Vec<PathBuf> {
    // Preserve the current file's parent path exactly; this supports both
    // absolute and cwd-relative buffer paths with no path normalization side effects.
    let parent = current_file.parent().unwrap_or_else(|| Path::new(""));
    target_extensions
        .iter()
        .map(|extension| parent.join(format!("{stem}.{extension}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempTree;

    /// Corresponding lookup should resolve `.c` to an existing `.h` sibling.
    #[test]
    fn test_find_corresponding_file_path_resolves_c_to_h() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("main.c", "int main(void) { return 0; }\n")
            .expect("write source");
        dir.write_file("main.h", "#pragma once\n")
            .expect("write header");
        let source = dir.path().join("main.c");
        let header = dir.path().join("main.h");

        let resolved = find_corresponding_file_path(&source).expect("resolve corresponding header");

        assert_eq!(resolved, header);
    }

    /// Corresponding lookup should resolve `.mli` to an existing `.ml` sibling.
    #[test]
    fn test_find_corresponding_file_path_resolves_mli_to_ml() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("module.mli", "val run : unit -> unit\n")
            .expect("write interface");
        dir.write_file("module.ml", "let run () = ()\n")
            .expect("write implementation");
        let interface = dir.path().join("module.mli");
        let implementation = dir.path().join("module.ml");

        let resolved =
            find_corresponding_file_path(&interface).expect("resolve corresponding implementation");

        assert_eq!(resolved, implementation);
    }

    /// When multiple target extensions exist, ordered preference should pick the first existing file.
    #[test]
    fn test_find_corresponding_file_path_uses_profile_order_for_cpp_targets() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("widget.cpp", "int run();\n")
            .expect("write source");
        dir.write_file("widget.h", "#pragma once\n")
            .expect("write first header");
        dir.write_file("widget.hh", "#pragma once\n")
            .expect("write second header");
        let source = dir.path().join("widget.cpp");
        let first_choice = dir.path().join("widget.h");

        let resolved = find_corresponding_file_path(&source).expect("resolve corresponding header");

        assert_eq!(resolved, first_choice);
    }

    /// Corresponding lookup from `.h` should prefer an existing `.cc` implementation.
    #[test]
    fn test_find_corresponding_file_path_prefers_cc_for_h_headers() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("widget.h", "#pragma once\n")
            .expect("write header");
        dir.write_file("widget.cc", "int run();\n")
            .expect("write cc source");
        dir.write_file("widget.c", "int run(void) { return 0; }\n")
            .expect("write c fallback");
        let header = dir.path().join("widget.h");
        let preferred = dir.path().join("widget.cc");

        // `.h` should choose C++ source first when multiple candidates exist.
        let resolved =
            find_corresponding_file_path(&header).expect("resolve preferred C++ implementation");

        assert_eq!(resolved, preferred);
    }

    /// Corresponding lookup from `.h` should fall back to `.cpp` when `.cc` is missing.
    #[test]
    fn test_find_corresponding_file_path_falls_back_to_cpp_for_h_headers() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("widget.h", "#pragma once\n")
            .expect("write header");
        dir.write_file("widget.cpp", "int run();\n")
            .expect("write cpp source");
        let header = dir.path().join("widget.h");
        let expected = dir.path().join("widget.cpp");

        // The second C++ candidate should be selected when `.cc` is unavailable.
        let resolved =
            find_corresponding_file_path(&header).expect("resolve C++ fallback implementation");

        assert_eq!(resolved, expected);
    }

    /// Corresponding lookup from `.h` should fall back to `.cxx` when earlier C++ targets are absent.
    #[test]
    fn test_find_corresponding_file_path_falls_back_to_cxx_for_h_headers() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("widget.h", "#pragma once\n")
            .expect("write header");
        dir.write_file("widget.cxx", "int run();\n")
            .expect("write cxx source");
        let header = dir.path().join("widget.h");
        let expected = dir.path().join("widget.cxx");

        // The ordered lookup should continue through C++ extensions until a hit.
        let resolved =
            find_corresponding_file_path(&header).expect("resolve C++ fallback implementation");

        assert_eq!(resolved, expected);
    }

    /// Corresponding lookup from `.h` should fall back to `.c` after exhausting C++ targets.
    #[test]
    fn test_find_corresponding_file_path_falls_back_to_c_for_h_headers() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("widget.h", "#pragma once\n")
            .expect("write header");
        dir.write_file("widget.c", "int run(void) { return 0; }\n")
            .expect("write c source");
        let header = dir.path().join("widget.h");
        let expected = dir.path().join("widget.c");

        // C source remains a valid last-resort implementation target.
        let resolved = find_corresponding_file_path(&header).expect("resolve C fallback");

        assert_eq!(resolved, expected);
    }

    /// Unsupported extensions should report a typed mapping error.
    #[test]
    fn test_find_corresponding_file_path_rejects_unsupported_extension() {
        let err = find_corresponding_file_path(Path::new("notes.txt"))
            .expect_err("unsupported extension should fail");

        assert_eq!(err, CorrespondingFileError::UnsupportedFileType);
    }

    /// Missing counterpart files should report a not-found error.
    #[test]
    fn test_find_corresponding_file_path_reports_missing_counterpart() {
        let dir = TempTree::new().expect("create temp tree");
        dir.write_file("main.py", "def run() -> int:\n    return 1\n")
            .expect("write source");
        let source = dir.path().join("main.py");

        let err = find_corresponding_file_path(&source).expect_err("missing counterpart");

        assert_eq!(err, CorrespondingFileError::NotFound);
    }
}
