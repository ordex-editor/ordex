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
    MarkerBased(&'static [&'static str]),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LspServerDescriptor {
    pub(crate) id: LspServerId,
    pub(crate) display_name: &'static str,
    command: &'static [&'static str],
    supported_languages: &'static [LanguageId],
    project_detection: ProjectDetection,
    features: LspServerFeatures,
}

/// High-level request kind used when selecting server routes.
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
    ".git",
];
const RUFF_MARKERS: &[&str] = &["pyproject.toml", "ruff.toml", ".ruff.toml", ".git"];
const PYLSP_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "Pipfile",
    ".git",
];
const CLANGD_MARKERS: &[&str] = &[
    ".clangd",
    ".clang-tidy",
    ".clang-format",
    "compile_commands.json",
    "compile_flags.txt",
    "configure.ac",
    ".git",
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
    project_detection: ProjectDetection::MarkerBased(TY_MARKERS),
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
    project_detection: ProjectDetection::MarkerBased(RUFF_MARKERS),
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
    project_detection: ProjectDetection::MarkerBased(PYLSP_MARKERS),
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
    project_detection: ProjectDetection::MarkerBased(CLANGD_MARKERS),
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

impl LspServerDescriptor {
    /// Return the executable name used to spawn this server.
    pub(crate) fn command_program(self) -> &'static str {
        self.command[0]
    }

    /// Return the trailing command-line arguments used to spawn this server.
    pub(crate) fn command_args(self) -> &'static [&'static str] {
        &self.command[1..]
    }

    /// Return the project-root detection strategy for this server.
    pub(crate) fn project_detection(self) -> ProjectDetection {
        self.project_detection
    }

    /// Return whether this server supports the supplied syntax language.
    pub(crate) fn supports_language(self, language: LanguageId) -> bool {
        self.supported_languages.contains(&language)
    }

    /// Return whether this server should handle requests for `route`.
    pub(crate) fn supports_route(self, route: LspRouteKind) -> bool {
        match route {
            LspRouteKind::Sync => true,
            LspRouteKind::Navigation => self.features.navigation,
            LspRouteKind::Hover => self.features.hover,
            LspRouteKind::Rename => self.features.rename,
        }
    }

    /// Return the LSP `languageId` string used for one file path.
    pub(crate) fn lsp_language_id(self, path: &Path) -> Option<&'static str> {
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

/// Return the routed servers for `language` and request `kind`.
pub(crate) fn route_servers(
    language: LanguageId,
    kind: LspRouteKind,
) -> Vec<&'static LspServerDescriptor> {
    // Routing stays data-driven so Python can use multiple cooperating servers
    // without scattering policy decisions across the manager and session layers.
    servers_for_language(language)
        .iter()
        .copied()
        .filter(|server| server.supports_route(kind))
        .collect()
}

/// Return the user-facing project-root requirement text for one language.
pub(crate) fn supported_project_description(language: LanguageId) -> &'static str {
    match language {
        LanguageId::Rust => "a supported Rust project root (Cargo workspace or rust-project.json)",
        LanguageId::Python => {
            "a supported Python project root (ty.toml, pyproject.toml, setup.py, setup.cfg, requirements.txt, Pipfile, ruff.toml, .ruff.toml, or .git)"
        }
        LanguageId::C | LanguageId::Cpp => {
            "a supported C/C++ project root (.clangd, .clang-tidy, .clang-format, compile_commands.json, compile_flags.txt, configure.ac, or .git)"
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
            .into_iter()
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
