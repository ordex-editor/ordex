//! Built-in language-server catalog and descriptor behavior.

use crate::cache_dirs;
use crate::syntax::profile::LanguageId;
use crate::syntax::profiles::detect_language_details;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Stable identifier for one built-in language-server integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LspServerId {
    RustAnalyzer,
    Ty,
    Ruff,
    Pylsp,
    Clangd,
    TypeScriptLanguageServer,
    Gopls,
    Jdtls,
    Phpactor,
    BashLanguageServer,
    HtmlLanguageServer,
    CssLanguageServer,
    JsonLanguageServer,
    YamlLanguageServer,
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
    pub(crate) features: LspServerFeatures,
    requires_workspace_data_dir: bool,
}

const RUST_LANGUAGES: &[LanguageId] = &[LanguageId::Rust];
const PYTHON_LANGUAGES: &[LanguageId] = &[LanguageId::Python];
const C_FAMILY_LANGUAGES: &[LanguageId] = &[LanguageId::C, LanguageId::Cpp];
const WEB_LANGUAGES: &[LanguageId] = &[LanguageId::JavaScript, LanguageId::TypeScript];
const GO_LANGUAGES: &[LanguageId] = &[LanguageId::Go];
const JAVA_LANGUAGES: &[LanguageId] = &[LanguageId::Java];
const PHP_LANGUAGES: &[LanguageId] = &[LanguageId::Php];
const SHELL_LANGUAGES: &[LanguageId] = &[
    LanguageId::Bash,
    LanguageId::Sh,
    LanguageId::Zsh,
    LanguageId::Fish,
];
const HTML_LANGUAGES: &[LanguageId] = &[LanguageId::Html, LanguageId::Xhtml];
const CSS_LANGUAGES: &[LanguageId] = &[LanguageId::Css, LanguageId::Scss, LanguageId::Less];
const JSON_LANGUAGES: &[LanguageId] = &[LanguageId::Json, LanguageId::JsonC];
const YAML_LANGUAGES: &[LanguageId] = &[LanguageId::Yaml];

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
const TYPESCRIPT_LANGUAGE_SERVER_MARKERS: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "bun.lock",
    "package.json",
    "tsconfig.json",
    "jsconfig.json",
    ".git",
];
const GOPLS_MARKERS: &[&str] = &["go.work", "go.mod", ".git"];
const JDTLS_MARKERS: &[&str] = &[
    "mvnw",
    "gradlew",
    "settings.gradle",
    "settings.gradle.kts",
    "build.xml",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    ".git",
];
const PHPACTOR_MARKERS: &[&str] = &[".git", "composer.json", ".phpactor.json", ".phpactor.yml"];
const BASH_LANGUAGE_SERVER_MARKERS: &[&str] = &[".git"];
const HTML_LANGUAGE_SERVER_MARKERS: &[&str] = &["package.json", ".git"];
const CSS_LANGUAGE_SERVER_MARKERS: &[&str] = &["package.json", ".git"];
const JSON_LANGUAGE_SERVER_MARKERS: &[&str] = &[".git"];
const YAML_LANGUAGE_SERVER_MARKERS: &[&str] = &[".git"];

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
    requires_workspace_data_dir: false,
};

pub(crate) const TY: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Ty,
    display_name: "ty",
    command: &["ty", "server"],
    supported_languages: PYTHON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: TY_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: false,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const RUFF: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Ruff,
    display_name: "ruff",
    command: &["ruff", "server"],
    supported_languages: PYTHON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: RUFF_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: false,
        hover: false,
        rename: false,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const PYLSP: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Pylsp,
    display_name: "pylsp",
    command: &["pylsp"],
    supported_languages: PYTHON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: PYLSP_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
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
    requires_workspace_data_dir: false,
};

pub(crate) const TYPESCRIPT_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::TypeScriptLanguageServer,
    display_name: "typescript-language-server",
    command: &["typescript-language-server", "--stdio"],
    supported_languages: WEB_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: TYPESCRIPT_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const GOPLS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Gopls,
    display_name: "gopls",
    command: &["gopls"],
    supported_languages: GO_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: GOPLS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const JDTLS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Jdtls,
    display_name: "jdtls",
    command: &["jdtls"],
    supported_languages: JAVA_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: JDTLS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
    requires_workspace_data_dir: true,
};

pub(crate) const PHPACTOR: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Phpactor,
    display_name: "phpactor",
    command: &["phpactor", "language-server"],
    supported_languages: PHP_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: PHPACTOR_MARKERS,
        fallback_to_file_directory: false,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: false,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const BASH_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::BashLanguageServer,
    display_name: "bash-language-server",
    command: &["bash-language-server", "start"],
    supported_languages: SHELL_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: BASH_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: true,
        hover: true,
        rename: true,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const HTML_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::HtmlLanguageServer,
    display_name: "vscode-html-language-server",
    command: &["vscode-html-language-server", "--stdio"],
    supported_languages: HTML_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: HTML_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: false,
        hover: true,
        rename: false,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const CSS_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::CssLanguageServer,
    display_name: "vscode-css-language-server",
    command: &["vscode-css-language-server", "--stdio"],
    supported_languages: CSS_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: CSS_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: false,
        hover: true,
        rename: false,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const JSON_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::JsonLanguageServer,
    display_name: "vscode-json-language-server",
    command: &["vscode-json-language-server", "--stdio"],
    supported_languages: JSON_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: JSON_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: false,
        hover: true,
        rename: false,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

pub(crate) const YAML_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::YamlLanguageServer,
    display_name: "yaml-language-server",
    command: &["yaml-language-server", "--stdio"],
    supported_languages: YAML_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: YAML_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: LspServerFeatures {
        navigation: false,
        hover: true,
        rename: false,
        diagnostics: true,
    },
    requires_workspace_data_dir: false,
};

impl LspServerDescriptor {
    /// Return the executable name used to spawn this server.
    pub(crate) fn command_program(&self) -> &'static str {
        self.command[0]
    }

    /// Return the command-line arguments used to spawn this server for one workspace.
    pub(crate) fn command_args(&self, workspace_root: &Path) -> io::Result<Vec<String>> {
        let mut args = self.command[1..]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        if self.requires_workspace_data_dir {
            let data_dir = self.workspace_data_dir(workspace_root)?;
            args.push("-data".to_string());
            args.push(data_dir.display().to_string());
        }
        Ok(args)
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
        let language = detect_language_details(Some(path))?.0.id;
        if !self.supports_language(language) {
            return None;
        }
        // Shared servers still need language-specific identifiers so upstream
        // routing stays correct for mixed families such as JS/TS and CSS/SCSS.
        match language {
            LanguageId::Rust => Some("rust"),
            LanguageId::Python => Some("python"),
            LanguageId::C => Some("c"),
            LanguageId::Cpp => Some("cpp"),
            LanguageId::JavaScript => Some("javascript"),
            LanguageId::TypeScript => Some("typescript"),
            LanguageId::Go => Some("go"),
            LanguageId::Java => Some("java"),
            LanguageId::Php => Some("php"),
            LanguageId::Bash | LanguageId::Sh | LanguageId::Zsh | LanguageId::Fish => {
                Some("shellscript")
            }
            LanguageId::Html | LanguageId::Xhtml => Some("html"),
            LanguageId::Css => Some("css"),
            LanguageId::Scss => Some("scss"),
            LanguageId::Less => Some("less"),
            LanguageId::Json => Some("json"),
            LanguageId::JsonC => Some("jsonc"),
            LanguageId::Yaml => Some("yaml"),
            _ => None,
        }
    }

    /// Resolve the per-workspace data directory required by servers such as `jdtls`.
    fn workspace_data_dir(&self, workspace_root: &Path) -> io::Result<PathBuf> {
        let base_dir = cache_dirs::default_ordex_cache_subdir("lsp-data")?;
        let workspace_component = sanitized_workspace_component(workspace_root);
        let data_dir = base_dir.join(workspace_component);
        fs::create_dir_all(&data_dir)?;
        Ok(data_dir)
    }
}

/// Convert one workspace path into a filesystem-safe cache directory component.
fn sanitized_workspace_component(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut component = String::with_capacity(raw.len());

    // Cache directories should remain readable while still avoiding path
    // separators or shell-special characters that would fragment the workspace
    // key across nested directories.
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            component.push(ch);
        } else {
            component.push('_');
        }
    }

    if component.is_empty() {
        return "workspace".to_string();
    }
    component
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify clangd handles both C and C++ files with the matching language id.
    #[test]
    fn test_clangd_uses_language_specific_lsp_ids() {
        assert_eq!(CLANGD.lsp_language_id(Path::new("main.c")), Some("c"));
        assert_eq!(CLANGD.lsp_language_id(Path::new("main.cpp")), Some("cpp"));
    }

    /// Verify added built-in integrations expose the expected LSP language ids.
    #[test]
    fn test_added_servers_use_expected_language_ids() {
        // Each newly added descriptor should translate the editor's syntax id
        // into the exact `languageId` string expected by the server.
        assert_eq!(
            TYPESCRIPT_LANGUAGE_SERVER.lsp_language_id(Path::new("main.js")),
            Some("javascript")
        );
        assert_eq!(
            TYPESCRIPT_LANGUAGE_SERVER.lsp_language_id(Path::new("main.ts")),
            Some("typescript")
        );
        assert_eq!(GOPLS.lsp_language_id(Path::new("main.go")), Some("go"));
        assert_eq!(JDTLS.lsp_language_id(Path::new("Main.java")), Some("java"));
        assert_eq!(PHPACTOR.lsp_language_id(Path::new("main.php")), Some("php"));
        assert_eq!(
            BASH_LANGUAGE_SERVER.lsp_language_id(Path::new("script.sh")),
            Some("shellscript")
        );
        assert_eq!(
            HTML_LANGUAGE_SERVER.lsp_language_id(Path::new("index.html")),
            Some("html")
        );
        assert_eq!(
            CSS_LANGUAGE_SERVER.lsp_language_id(Path::new("style.scss")),
            Some("scss")
        );
        assert_eq!(
            JSON_LANGUAGE_SERVER.lsp_language_id(Path::new("config.jsonc")),
            Some("jsonc")
        );
        assert_eq!(
            YAML_LANGUAGE_SERVER.lsp_language_id(Path::new("config.yaml")),
            Some("yaml")
        );
    }

    /// Verify `jdtls` receives one stable per-workspace data directory argument.
    #[test]
    fn test_jdtls_command_args_include_workspace_data_dir() {
        let args = JDTLS
            .command_args(Path::new("/tmp/java-workspace"))
            .expect("jdtls args");

        assert_eq!(args[0], "-data");
        assert!(args[1].contains("ordex"));
        assert!(args[1].contains("lsp-data"));
        assert!(args[1].contains("java-workspace"));
    }

    /// Verify workspace path sanitization avoids nested cache directories.
    #[test]
    fn test_workspace_component_sanitizes_path_separators() {
        let component = sanitized_workspace_component(Path::new("/tmp/project with spaces"));

        assert_eq!(component, "_tmp_project_with_spaces");
    }
}
