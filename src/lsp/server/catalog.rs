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
    CsharpLs,
    TypeScriptLanguageServer,
    Gopls,
    Jdtls,
    Phpactor,
    BashLanguageServer,
    Marksman,
    Taplo,
    HtmlLanguageServer,
    CssLanguageServer,
    JsonLanguageServer,
    YamlLanguageServer,
    Lemminx,
    GraphqlLanguageService,
    DockerLanguageServer,
    TerraformLs,
    Nil,
    LuaLanguageServer,
    Solargraph,
    SourcekitLsp,
    KotlinLsp,
    Metals,
    RLanguageServer,
    Sqls,
    Zls,
    JuliaLanguageServer,
    HaskellLanguageServer,
    OcamlLsp,
    FsAutocomplete,
    DartLanguageServer,
    PerlNavigator,
    CmakeLanguageServer,
    ElmLanguageServer,
    ErlangLs,
    CueLsp,
    SolidityLanguageServer,
    QmlLs,
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

const FULL_SERVER_FEATURES: LspServerFeatures = LspServerFeatures {
    navigation: true,
    hover: true,
    rename: true,
    diagnostics: true,
};
const NAVIGATION_SERVER_FEATURES: LspServerFeatures = LspServerFeatures {
    navigation: true,
    hover: true,
    rename: true,
    diagnostics: false,
};
const HOVER_AND_DIAGNOSTIC_SERVER_FEATURES: LspServerFeatures = LspServerFeatures {
    navigation: false,
    hover: true,
    rename: false,
    diagnostics: true,
};
const NAVIGATION_HOVER_DIAGNOSTIC_SERVER_FEATURES: LspServerFeatures = LspServerFeatures {
    navigation: true,
    hover: true,
    rename: false,
    diagnostics: true,
};
const NAVIGATION_AND_HOVER_SERVER_FEATURES: LspServerFeatures = LspServerFeatures {
    navigation: true,
    hover: true,
    rename: false,
    diagnostics: false,
};

const RUST_LANGUAGES: &[LanguageId] = &[LanguageId::Rust];
const PYTHON_LANGUAGES: &[LanguageId] = &[LanguageId::Python];
const C_FAMILY_LANGUAGES: &[LanguageId] = &[LanguageId::C, LanguageId::Cpp];
const CSHARP_LANGUAGES: &[LanguageId] = &[LanguageId::CSharp];
const TYPESCRIPT_LANGUAGES: &[LanguageId] = &[LanguageId::JavaScript, LanguageId::TypeScript];
const GO_LANGUAGES: &[LanguageId] = &[LanguageId::Go];
const JAVA_LANGUAGES: &[LanguageId] = &[LanguageId::Java];
const PHP_LANGUAGES: &[LanguageId] = &[LanguageId::Php];
const SHELL_LANGUAGES: &[LanguageId] = &[
    LanguageId::Bash,
    LanguageId::Sh,
    LanguageId::Zsh,
    LanguageId::Fish,
];
const MARKDOWN_LANGUAGES: &[LanguageId] = &[LanguageId::Markdown];
const TOML_LANGUAGES: &[LanguageId] = &[LanguageId::Toml];
const HTML_LANGUAGES: &[LanguageId] = &[LanguageId::Html, LanguageId::Xhtml];
const CSS_LANGUAGES: &[LanguageId] = &[LanguageId::Css, LanguageId::Scss, LanguageId::Less];
const JSON_LANGUAGES: &[LanguageId] = &[LanguageId::Json, LanguageId::JsonC];
const YAML_LANGUAGES: &[LanguageId] = &[LanguageId::Yaml];
const XML_LANGUAGES: &[LanguageId] = &[LanguageId::Xml];
const GRAPHQL_LANGUAGES: &[LanguageId] = &[LanguageId::GraphQl];
const DOCKER_LANGUAGES: &[LanguageId] = &[LanguageId::Dockerfile];
const TERRAFORM_LANGUAGES: &[LanguageId] = &[LanguageId::Hcl];
const NIX_LANGUAGES: &[LanguageId] = &[LanguageId::Nix];
const LUA_LANGUAGES: &[LanguageId] = &[LanguageId::Lua];
const RUBY_LANGUAGES: &[LanguageId] = &[LanguageId::Ruby];
const SWIFT_LANGUAGES: &[LanguageId] = &[LanguageId::Swift];
const KOTLIN_LANGUAGES: &[LanguageId] = &[LanguageId::Kotlin];
const SCALA_LANGUAGES: &[LanguageId] = &[LanguageId::Scala];
const R_LANGUAGES: &[LanguageId] = &[LanguageId::R];
const SQL_LANGUAGES: &[LanguageId] = &[LanguageId::Sql];
const ZIG_LANGUAGES: &[LanguageId] = &[LanguageId::Zig];
const JULIA_LANGUAGES: &[LanguageId] = &[LanguageId::Julia];
const HASKELL_LANGUAGES: &[LanguageId] = &[LanguageId::Haskell];
const OCAML_LANGUAGES: &[LanguageId] = &[LanguageId::Ocaml];
const FSHARP_LANGUAGES: &[LanguageId] = &[LanguageId::FSharp];
const DART_LANGUAGES: &[LanguageId] = &[LanguageId::Dart];
const PERL_LANGUAGES: &[LanguageId] = &[LanguageId::Perl];
const CMAKE_LANGUAGES: &[LanguageId] = &[LanguageId::CMake];
const ELM_LANGUAGES: &[LanguageId] = &[LanguageId::Elm];
const ERLANG_LANGUAGES: &[LanguageId] = &[LanguageId::Erlang];
const CUE_LANGUAGES: &[LanguageId] = &[LanguageId::Cue];
const SOLIDITY_LANGUAGES: &[LanguageId] = &[LanguageId::Solidity];
const QML_LANGUAGES: &[LanguageId] = &[LanguageId::Qml];

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
const CSHARP_LS_MARKERS: &[&str] = &[
    "global.json",
    "Directory.Build.props",
    "Directory.Build.targets",
    "NuGet.Config",
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
];
const GOPLS_MARKERS: &[&str] = &["go.work", "go.mod"];
const JDTLS_MARKERS: &[&str] = &[
    "mvnw",
    "gradlew",
    "settings.gradle",
    "settings.gradle.kts",
    "build.xml",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
];
const PHPACTOR_MARKERS: &[&str] = &["composer.json", ".phpactor.json", ".phpactor.yml"];
const BASH_LANGUAGE_SERVER_MARKERS: &[&str] = &[];
const MARKSMAN_MARKERS: &[&str] = &[];
const TAPLO_MARKERS: &[&str] = &["taplo.toml", ".taplo.toml", "Cargo.toml"];
const HTML_LANGUAGE_SERVER_MARKERS: &[&str] = &["package.json"];
const CSS_LANGUAGE_SERVER_MARKERS: &[&str] = &["package.json"];
const JSON_LANGUAGE_SERVER_MARKERS: &[&str] = &[];
const YAML_LANGUAGE_SERVER_MARKERS: &[&str] = &[];
const LEMMINX_MARKERS: &[&str] = &[];
const GRAPHQL_LANGUAGE_SERVICE_MARKERS: &[&str] = &[
    "package.json",
    "graphql.config.yml",
    "graphql.config.yaml",
    "graphql.config.json",
    ".graphqlrc",
    ".graphqlrc.yml",
    ".graphqlrc.yaml",
    ".graphqlrc.json",
];
const DOCKER_LANGUAGE_SERVER_MARKERS: &[&str] = &[
    "Dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
];
const TERRAFORM_LS_MARKERS: &[&str] = &[
    "main.tf",
    "terraform.tf",
    "versions.tf",
    "terraform.tfvars",
    ".terraform",
];
const NIL_MARKERS: &[&str] = &["flake.nix", "shell.nix", "default.nix"];
const LUA_LANGUAGE_SERVER_MARKERS: &[&str] =
    &[".luarc.json", ".luarc.jsonc", "stylua.toml", ".stylua.toml"];
const SOLARGRAPH_MARKERS: &[&str] = &["Gemfile", ".solargraph.yml", "Rakefile"];
const SOURCEKIT_LSP_MARKERS: &[&str] = &["Package.swift"];
const KOTLIN_LSP_MARKERS: &[&str] = &[
    "settings.gradle",
    "settings.gradle.kts",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
];
const METALS_MARKERS: &[&str] = &["build.sbt", "build.sc", ".bsp", "project"];
const R_LANGUAGE_SERVER_MARKERS: &[&str] = &["DESCRIPTION", ".Rprofile", "renv.lock"];
const SQLS_MARKERS: &[&str] = &["sqls.yml", ".sqls.yml"];
const JULIA_LANGUAGE_SERVER_MARKERS: &[&str] = &["Project.toml", "Manifest.toml"];
const HASKELL_LANGUAGE_SERVER_MARKERS: &[&str] = &["hie.yaml", "stack.yaml", "cabal.project"];
const OCAML_LSP_MARKERS: &[&str] = &["dune-project", "dune-workspace", "opam", "esy.json"];
const FSAUTOCOMPLETE_MARKERS: &[&str] = &[
    "global.json",
    "Directory.Build.props",
    "Directory.Build.targets",
];
const DART_LANGUAGE_SERVER_MARKERS: &[&str] = &["pubspec.yaml"];
const PERL_NAVIGATOR_MARKERS: &[&str] = &["cpanfile", "dist.ini", "Build.PL", "Makefile.PL"];
const CMAKE_LANGUAGE_SERVER_MARKERS: &[&str] =
    &["CMakeLists.txt", ".neocmake.toml", ".neocmakelint.toml"];
const ELM_LANGUAGE_SERVER_MARKERS: &[&str] = &["elm.json"];
const ERLANG_LS_MARKERS: &[&str] = &["rebar.config", "erlang.mk"];
const CUE_LSP_MARKERS: &[&str] = &["cue.mod"];
const SOLIDITY_LANGUAGE_SERVER_MARKERS: &[&str] = &[
    "foundry.toml",
    "hardhat.config.js",
    "hardhat.config.ts",
    "truffle-config.js",
    "brownie-config.yaml",
];
const QML_LS_MARKERS: &[&str] = &[];

pub(crate) const RUST_ANALYZER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::RustAnalyzer,
    display_name: "rust-analyzer",
    command: &["rust-analyzer"],
    supported_languages: RUST_LANGUAGES,
    project_detection: ProjectDetection::RustWorkspace,
    features: FULL_SERVER_FEATURES,
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
    features: NAVIGATION_SERVER_FEATURES,
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
    features: FULL_SERVER_FEATURES,
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
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const CSHARP_LS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::CsharpLs,
    display_name: "csharp-ls",
    command: &["csharp-ls"],
    supported_languages: CSHARP_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: CSHARP_LS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const TYPESCRIPT_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::TypeScriptLanguageServer,
    display_name: "typescript-language-server",
    command: &["typescript-language-server", "--stdio"],
    supported_languages: TYPESCRIPT_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: TYPESCRIPT_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
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
    features: FULL_SERVER_FEATURES,
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
    features: FULL_SERVER_FEATURES,
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
    features: NAVIGATION_SERVER_FEATURES,
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
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const MARKSMAN: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Marksman,
    display_name: "marksman",
    command: &["marksman", "server"],
    supported_languages: MARKDOWN_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: MARKSMAN_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const TAPLO: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Taplo,
    display_name: "taplo",
    command: &["taplo", "lsp", "stdio"],
    supported_languages: TOML_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: TAPLO_MARKERS,
        fallback_to_file_directory: true,
    },
    features: HOVER_AND_DIAGNOSTIC_SERVER_FEATURES,
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
    features: HOVER_AND_DIAGNOSTIC_SERVER_FEATURES,
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
    features: FULL_SERVER_FEATURES,
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
    features: HOVER_AND_DIAGNOSTIC_SERVER_FEATURES,
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
    features: HOVER_AND_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const LEMMINX: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Lemminx,
    display_name: "lemminx",
    command: &["lemminx"],
    supported_languages: XML_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: LEMMINX_MARKERS,
        fallback_to_file_directory: true,
    },
    features: HOVER_AND_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const GRAPHQL_LANGUAGE_SERVICE: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::GraphqlLanguageService,
    display_name: "graphql-lsp",
    command: &["graphql-lsp", "server", "-m", "stream"],
    supported_languages: GRAPHQL_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: GRAPHQL_LANGUAGE_SERVICE_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_HOVER_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const DOCKER_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::DockerLanguageServer,
    display_name: "docker-langserver",
    command: &["docker-langserver", "--stdio"],
    supported_languages: DOCKER_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: DOCKER_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: HOVER_AND_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const TERRAFORM_LS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::TerraformLs,
    display_name: "terraform-ls",
    command: &["terraform-ls", "serve"],
    supported_languages: TERRAFORM_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: TERRAFORM_LS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const NIL: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Nil,
    display_name: "nil",
    command: &["nil"],
    supported_languages: NIX_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: NIL_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_HOVER_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const LUA_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::LuaLanguageServer,
    display_name: "lua-language-server",
    command: &["lua-language-server"],
    supported_languages: LUA_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: LUA_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const SOLARGRAPH: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Solargraph,
    display_name: "solargraph",
    command: &["solargraph", "stdio"],
    supported_languages: RUBY_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: SOLARGRAPH_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const SOURCEKIT_LSP: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::SourcekitLsp,
    display_name: "sourcekit-lsp",
    command: &["sourcekit-lsp"],
    supported_languages: SWIFT_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: SOURCEKIT_LSP_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const KOTLIN_LSP: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::KotlinLsp,
    display_name: "kotlin-lsp",
    command: &["kotlin-lsp"],
    supported_languages: KOTLIN_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: KOTLIN_LSP_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const METALS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Metals,
    display_name: "metals",
    command: &["metals"],
    supported_languages: SCALA_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: METALS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const R_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::RLanguageServer,
    display_name: "languageserver",
    command: &["R", "--slave", "-e", "languageserver::run()"],
    supported_languages: R_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: R_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const SQLS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Sqls,
    display_name: "sqls",
    command: &["sqls"],
    supported_languages: SQL_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: SQLS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_AND_HOVER_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const ZLS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::Zls,
    display_name: "zls",
    command: &["zls"],
    supported_languages: ZIG_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: &[],
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const JULIA_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::JuliaLanguageServer,
    display_name: "LanguageServer.jl",
    command: &[
        "julia",
        "--startup-file=no",
        "--history-file=no",
        "--quiet",
        "-e",
        "using LanguageServer; runserver()",
    ],
    supported_languages: JULIA_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: JULIA_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const HASKELL_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::HaskellLanguageServer,
    display_name: "haskell-language-server-wrapper",
    command: &["haskell-language-server-wrapper", "--lsp"],
    supported_languages: HASKELL_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: HASKELL_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const OCAML_LSP: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::OcamlLsp,
    display_name: "ocamllsp",
    command: &["ocamllsp"],
    supported_languages: OCAML_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: OCAML_LSP_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const FSAUTOCOMPLETE: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::FsAutocomplete,
    display_name: "fsautocomplete",
    command: &["dotnet", "fsautocomplete", "--background-service-enabled"],
    supported_languages: FSHARP_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: FSAUTOCOMPLETE_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const DART_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::DartLanguageServer,
    display_name: "dart language-server",
    command: &["dart", "language-server", "--protocol=lsp"],
    supported_languages: DART_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: DART_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const PERL_NAVIGATOR: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::PerlNavigator,
    display_name: "perlnavigator",
    command: &["perlnavigator", "--stdio"],
    supported_languages: PERL_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: PERL_NAVIGATOR_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const CMAKE_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::CmakeLanguageServer,
    display_name: "cmake-language-server",
    command: &["cmake-language-server"],
    supported_languages: CMAKE_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: CMAKE_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_HOVER_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const ELM_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::ElmLanguageServer,
    display_name: "elm-language-server",
    command: &["elm-language-server"],
    supported_languages: ELM_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: ELM_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const ERLANG_LS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::ErlangLs,
    display_name: "erlang_ls",
    command: &["erlang_ls"],
    supported_languages: ERLANG_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: ERLANG_LS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const CUE_LSP: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::CueLsp,
    display_name: "cue",
    command: &["cue", "lsp", "serve"],
    supported_languages: CUE_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: CUE_LSP_MARKERS,
        fallback_to_file_directory: true,
    },
    features: NAVIGATION_HOVER_DIAGNOSTIC_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const SOLIDITY_LANGUAGE_SERVER: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::SolidityLanguageServer,
    display_name: "nomicfoundation-solidity-language-server",
    command: &["nomicfoundation-solidity-language-server", "--stdio"],
    supported_languages: SOLIDITY_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: SOLIDITY_LANGUAGE_SERVER_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
    requires_workspace_data_dir: false,
};

pub(crate) const QML_LS: LspServerDescriptor = LspServerDescriptor {
    id: LspServerId::QmlLs,
    display_name: "qmlls",
    command: &["qmlls"],
    supported_languages: QML_LANGUAGES,
    project_detection: ProjectDetection::MarkerBased {
        markers: QML_LS_MARKERS,
        fallback_to_file_directory: true,
    },
    features: FULL_SERVER_FEATURES,
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
            LanguageId::Toml => Some("toml"),
            LanguageId::Markdown => Some("markdown"),
            LanguageId::JavaScript => Some("javascript"),
            LanguageId::TypeScript => Some("typescript"),
            LanguageId::Python => Some("python"),
            LanguageId::Java => Some("java"),
            LanguageId::CSharp => Some("csharp"),
            LanguageId::Cpp => Some("cpp"),
            LanguageId::Go => Some("go"),
            LanguageId::C => Some("c"),
            LanguageId::Php => Some("php"),
            LanguageId::Bash | LanguageId::Sh | LanguageId::Zsh | LanguageId::Fish => {
                Some("shellscript")
            }
            LanguageId::Json => Some("json"),
            LanguageId::JsonC => Some("jsonc"),
            LanguageId::Yaml => Some("yaml"),
            LanguageId::Css => Some("css"),
            LanguageId::Scss => Some("scss"),
            LanguageId::Less => Some("less"),
            LanguageId::Xml => Some("xml"),
            LanguageId::Erlang => Some("erlang"),
            LanguageId::Elm => Some("elm"),
            LanguageId::CMake => Some("cmake"),
            LanguageId::Dockerfile => Some("dockerfile"),
            LanguageId::Hcl => Some("terraform"),
            LanguageId::Nix => Some("nix"),
            LanguageId::Lua => Some("lua"),
            LanguageId::Ruby => Some("ruby"),
            LanguageId::Swift => Some("swift"),
            LanguageId::Kotlin => Some("kotlin"),
            LanguageId::Scala => Some("scala"),
            LanguageId::R => Some("r"),
            LanguageId::Sql => Some("sql"),
            LanguageId::Zig => Some("zig"),
            LanguageId::Julia => Some("julia"),
            LanguageId::Haskell => Some("haskell"),
            LanguageId::Ocaml => Some("ocaml"),
            LanguageId::FSharp => Some("fsharp"),
            LanguageId::Dart => Some("dart"),
            LanguageId::Perl => Some("perl"),
            LanguageId::GraphQl => Some("graphql"),
            LanguageId::Cue => Some("cue"),
            LanguageId::Html | LanguageId::Xhtml => Some("html"),
            LanguageId::Solidity => Some("solidity"),
            LanguageId::Qml => Some("qml"),
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

    /// Verify the expanded built-in catalog exposes representative LSP language ids.
    #[test]
    fn test_curated_catalog_uses_expected_language_ids() {
        let cases = [
            (&MARKSMAN, "README.md", Some("markdown")),
            (&TAPLO, "Cargo.toml", Some("toml")),
            (&CSHARP_LS, "Program.cs", Some("csharp")),
            (&LEMMINX, "pom.xml", Some("xml")),
            (&GRAPHQL_LANGUAGE_SERVICE, "schema.graphql", Some("graphql")),
            (&TERRAFORM_LS, "main.tf", Some("terraform")),
            (&LUA_LANGUAGE_SERVER, "main.lua", Some("lua")),
            (&SOURCEKIT_LSP, "main.swift", Some("swift")),
            (&R_LANGUAGE_SERVER, "analysis.R", Some("r")),
            (&SQLS, "schema.sql", Some("sql")),
            (&JULIA_LANGUAGE_SERVER, "main.jl", Some("julia")),
            (&FSAUTOCOMPLETE, "main.fs", Some("fsharp")),
            (&DART_LANGUAGE_SERVER, "main.dart", Some("dart")),
            (&PERL_NAVIGATOR, "main.pl", Some("perl")),
            (&CUE_LSP, "main.cue", Some("cue")),
            (&SOLIDITY_LANGUAGE_SERVER, "contract.sol", Some("solidity")),
            (&QML_LS, "Main.qml", Some("qml")),
        ];
        for (server, path, expected) in cases {
            assert_eq!(server.lsp_language_id(Path::new(path)), expected, "{path}");
        }
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
