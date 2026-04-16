//! Built-in language-server descriptors and routing rules.

use crate::syntax::profile::LanguageId;
use crate::syntax::profiles::detect_language_details;
use std::path::Path;

/// Stable identifier for one built-in language-server integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LspServerId {
    RustAnalyzer,
    Ty,
    Ruff,
    Pylsp,
    Clangd,
}

/// Project-root detection strategy used by one built-in server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectDetection {
    RustWorkspace,
    MarkerBased {
        markers: &'static [&'static str],
        fallback_to_file_directory: bool,
    },
}

/// Feature flags describing which requests one server should own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LspServerFeatures {
    pub(crate) navigation: bool,
    pub(crate) hover: bool,
    pub(crate) rename: bool,
    pub(crate) diagnostics: bool,
}

/// One built-in language-server descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspServerDescriptor {
    pub(crate) id: LspServerId,
    pub(crate) display_name: &'static str,
    command: &'static [&'static str],
    supported_languages: &'static [LanguageId],
    project_detection: ProjectDetection,
    features: LspServerFeatures,
}

/// High-level editor action used to pick one ordered server route.
///
/// A server route is the built-in policy that maps one action such as hover or
/// document sync to the specific language servers that should receive it, in
/// the order Ordex should try them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LspRouteKind {
    Sync,
    Navigation,
    Hover,
    Rename,
}

const RUST_LANGUAGES: &[LanguageId] = &[LanguageId::Rust];
const PYTHON_LANGUAGES: &[LanguageId] = &[LanguageId::Python];
const C_FAMILY_LANGUAGES: &[LanguageId] = &[LanguageId::C, LanguageId::Cpp];

const TY_MARKERS: &[&str] = &[
    "ty.toml",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
];
const RUFF_MARKERS: &[&str] = &["pyproject.toml", "ruff.toml", ".ruff.toml"];
const PYLSP_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "Pipfile",
];
const CLANGD_MARKERS: &[&str] = &[
    ".clangd",
    ".clang-tidy",
    ".clang-format",
    "compile_commands.json",
    "compile_flags.txt",
    "configure.ac",
];

pub(crate) const RUST_ANALYZER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::RustAnalyzer,
    display_name: "rust-analyzer",
    command: &["rust-analyzer"],
    supported_languages: RUST_LANGUAGES,
    project_detection: ProjectDetection::RustWorkspace,
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
};

pub(crate) const TY: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Ty,
    display_name: "ty",
    command: &["ty", "server"],
    supported_languages: PYTHON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: TY_MARKERS,
        fallback_to_file_directory: false,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: false,
    },
};

pub(crate) const RUFF: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Ruff,
    display_name: "ruff",
    command: &["ruff", "server"],
    supported_languages: PYTHON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: RUFF_MARKERS,
        fallback_to_file_directory: false,
    },
    features: LspServerFeatures {
        navigation: false,
        hover: false,
        rename: false,
        diagnostics: true,
    },
};

pub(crate) const PYLSP: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Pylsp,
    display_name: "pylsp",
    command: &["pylsp"],
    supported_languages: PYTHON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: PYLSP_MARKERS,
        fallback_to_file_directory: false,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
};

pub(crate) const CLANGD: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Clangd,
    display_name: "clangd",
    command: &["clangd"],
    supported_languages: C_FAMILY_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: CLANGD_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
};

const RUST_SERVERS: &[&LspServerDescriptor] = &[&RUST_ANALYZER];
const PYTHON_SERVERS: &[&LspServerDescriptor] = &[&TY, &RUFF, &PYLSP];
const C_FAMILY_SERVERS: &[&LspServerDescriptor] = &[&CLANGD];
const PYTHON_NAVIGATION_SERVERS: &[&LspServerDescriptor] = &[&TY, &PYLSP];

impl LspServerDescriptor {
    /// Return the executable name used to spawn this server.
    pub(crate) fn command_program(&self) -> &'static str {
        self.command[0]
    }

    /// Return the trailing command-line arguments used to spawn this server.
    pub(crate) fn command_args(&self) -> &'static [&'static str] {
        &self.command[1..]
    }

    /// Return the project-root detection strategy for this server.
    pub(crate) fn project_detection(&self) -> ProjectDetection {
        self.project_detection
    }

    /// Return whether this server supports the supplied syntax language.
    pub(crate) fn supports_language(&self, language: LanguageId) -> bool {
        self.supported_languages.contains(&language)
    }

    /// Return the LSP `languageId` string used for one file path.
    pub(crate) fn lsp_language_id(&self, path: &Path) -> Option<&'static str> {
        let language = language_for_path(path)?;
        match language {
            LanguageId::Rust if self.supports_language(language) => Some("rust"),
            LanguageId::Python if self.supports_language(language) => Some("python"),
            LanguageId::C if self.supports_language(language) => Some("c"),
            LanguageId::Cpp if self.supports_language(language) => Some("cpp"),
            _ => None,
        }
    }
}

/// Detect the built-in syntax language for one path, if any.
pub(crate) fn language_for_path(path: &Path) -> Option<LanguageId> {
    detect_language_details(Some(path)).map(|(profile, _)| profile.id)
}

/// Return the built-in server list for one syntax language.
#[cfg(test)]
pub(crate) fn servers_for_language(
    language: LanguageId,
) -> &'static [&'static LspServerDescriptor] {
    match language {
        LanguageId::Rust => RUST_SERVERS,
        LanguageId::Python => PYTHON_SERVERS,
        LanguageId::C | LanguageId::Cpp => C_FAMILY_SERVERS,
        _ => &[],
    }
}

/// Return the ordered built-in server route for `language` and request `kind`.
///
/// Routes are static policy tables rather than a per-buffer cache: lookup is a
/// small constant-time match over the built-in server set, and project detection
/// remains the only file-specific work.
pub(crate) fn route_servers(
    language: LanguageId,
    kind: LspRouteKind,
) -> &'static [&'static LspServerDescriptor] {
    // Keep route lookup allocation-free and fully data-driven so per-request
    // routing stays cheap even when one language uses multiple servers.
    match (language, kind) {
        (LanguageId::Rust, _) => RUST_SERVERS,
        (LanguageId::Python, LspRouteKind::Sync) => PYTHON_SERVERS,
        (LanguageId::Python, LspRouteKind::Navigation) => PYTHON_NAVIGATION_SERVERS,
        (LanguageId::Python, LspRouteKind::Hover) => PYTHON_NAVIGATION_SERVERS,
        (LanguageId::Python, LspRouteKind::Rename) => PYTHON_NAVIGATION_SERVERS,
        (LanguageId::C | LanguageId::Cpp, _) => C_FAMILY_SERVERS,
        _ => &[],
    }
}

/// Return the user-facing project-root requirement text for one language.
pub(crate) fn supported_project_description(language: LanguageId) -> &'static str {
    // These descriptions appear in unsupported-project errors, so they should
    // describe the minimum root shape the built-in integration can use.
    match language {
        LanguageId::Rust => "a supported Rust project root (Cargo workspace or rust-project.json)",
        LanguageId::Python => {
            "a supported Python project root (ty.toml, pyproject.toml, setup.py, setup.cfg, requirements.txt, Pipfile, ruff.toml, or .ruff.toml)"
        }
        LanguageId::C | LanguageId::Cpp => {
            "the opened file directory or a supported C/C++ project root (.clangd, .clang-tidy, .clang-format, compile_commands.json, compile_flags.txt, or configure.ac)"
        }
        _ => "a supported project root",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify Python routing preserves the intended built-in ownership order.
    #[test]
    fn test_route_servers_for_python_match_feature_policies() {
        let navigation = route_servers(LanguageId::Python, LspRouteKind::Navigation)
            .iter()
            .copied()
            .map(|server| server.id)
            .collect::<Vec<_>>();
        let diagnostics = servers_for_language(LanguageId::Python)
            .iter()
            .copied()
            .filter(|server| server.features.diagnostics)
            .map(|server| server.id)
            .collect::<Vec<_>>();

        assert_eq!(navigation, vec![LspServerId::Ty, LspServerId::Pylsp]);
        assert_eq!(diagnostics, vec![LspServerId::Ruff, LspServerId::Pylsp]);
    }

    /// Verify clangd handles both C and C++ files with the matching language id.
    #[test]
    fn test_clangd_uses_language_specific_lsp_ids() {
        assert_eq!(CLANGD.lsp_language_id(Path::new("main.c")), Some("c"));
        assert_eq!(CLANGD.lsp_language_id(Path::new("main.cpp")), Some("cpp"));
    }
}
