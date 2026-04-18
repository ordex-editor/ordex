//! Built-in language-server routing policy.

use super::catalog::{
    BASH_LANGUAGE_SERVER, CLANGD, CMAKE_LANGUAGE_SERVER, CSHARP_LS, CSS_LANGUAGE_SERVER, CUE_LSP,
    DART_LANGUAGE_SERVER, DOCKER_LANGUAGE_SERVER, ELM_LANGUAGE_SERVER, ERLANG_LS, FSAUTOCOMPLETE,
    GOPLS, GRAPHQL_LANGUAGE_SERVICE, HASKELL_LANGUAGE_SERVER, HTML_LANGUAGE_SERVER, JDTLS,
    JSON_LANGUAGE_SERVER, JULIA_LANGUAGE_SERVER, KOTLIN_LSP, LEMMINX, LUA_LANGUAGE_SERVER,
    LspServerDescriptor, MARKSMAN, METALS, NIL, OCAML_LSP, PERL_NAVIGATOR, PHPACTOR, PYLSP, QML_LS,
    R_LANGUAGE_SERVER, RUFF, RUST_ANALYZER, SOLARGRAPH, SOLIDITY_LANGUAGE_SERVER, SOURCEKIT_LSP,
    SQLS, TAPLO, TERRAFORM_LS, TY, TYPESCRIPT_LANGUAGE_SERVER, YAML_LANGUAGE_SERVER, ZLS,
};
use crate::syntax::profile::LanguageId;
use crate::syntax::profiles::detect_language_details;
use std::path::Path;

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

/// Static route table and project description for one language family.
struct LanguageRoutes {
    languages: &'static [LanguageId],
    sync: &'static [&'static LspServerDescriptor],
    project_description: &'static str,
}

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

const RUST_SYNC_SERVERS: &[&LspServerDescriptor] = &[&RUST_ANALYZER];
const PYTHON_SYNC_SERVERS: &[&LspServerDescriptor] = &[&TY, &RUFF, &PYLSP];
const C_FAMILY_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CLANGD];
const CSHARP_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CSHARP_LS];
const TYPESCRIPT_SYNC_SERVERS: &[&LspServerDescriptor] = &[&TYPESCRIPT_LANGUAGE_SERVER];
const GO_SYNC_SERVERS: &[&LspServerDescriptor] = &[&GOPLS];
const JAVA_SYNC_SERVERS: &[&LspServerDescriptor] = &[&JDTLS];
const PHP_SYNC_SERVERS: &[&LspServerDescriptor] = &[&PHPACTOR];
const SHELL_SYNC_SERVERS: &[&LspServerDescriptor] = &[&BASH_LANGUAGE_SERVER];
const MARKDOWN_SYNC_SERVERS: &[&LspServerDescriptor] = &[&MARKSMAN];
const TOML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&TAPLO];
const HTML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&HTML_LANGUAGE_SERVER];
const CSS_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CSS_LANGUAGE_SERVER];
const JSON_SYNC_SERVERS: &[&LspServerDescriptor] = &[&JSON_LANGUAGE_SERVER];
const YAML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&YAML_LANGUAGE_SERVER];
const XML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&LEMMINX];
const GRAPHQL_SYNC_SERVERS: &[&LspServerDescriptor] = &[&GRAPHQL_LANGUAGE_SERVICE];
const DOCKER_SYNC_SERVERS: &[&LspServerDescriptor] = &[&DOCKER_LANGUAGE_SERVER];
const TERRAFORM_SYNC_SERVERS: &[&LspServerDescriptor] = &[&TERRAFORM_LS];
const NIX_SYNC_SERVERS: &[&LspServerDescriptor] = &[&NIL];
const LUA_SYNC_SERVERS: &[&LspServerDescriptor] = &[&LUA_LANGUAGE_SERVER];
const RUBY_SYNC_SERVERS: &[&LspServerDescriptor] = &[&SOLARGRAPH];
const SWIFT_SYNC_SERVERS: &[&LspServerDescriptor] = &[&SOURCEKIT_LSP];
const KOTLIN_SYNC_SERVERS: &[&LspServerDescriptor] = &[&KOTLIN_LSP];
const SCALA_SYNC_SERVERS: &[&LspServerDescriptor] = &[&METALS];
const R_SYNC_SERVERS: &[&LspServerDescriptor] = &[&R_LANGUAGE_SERVER];
const SQL_SYNC_SERVERS: &[&LspServerDescriptor] = &[&SQLS];
const ZIG_SYNC_SERVERS: &[&LspServerDescriptor] = &[&ZLS];
const JULIA_SYNC_SERVERS: &[&LspServerDescriptor] = &[&JULIA_LANGUAGE_SERVER];
const HASKELL_SYNC_SERVERS: &[&LspServerDescriptor] = &[&HASKELL_LANGUAGE_SERVER];
const OCAML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&OCAML_LSP];
const FSHARP_SYNC_SERVERS: &[&LspServerDescriptor] = &[&FSAUTOCOMPLETE];
const DART_SYNC_SERVERS: &[&LspServerDescriptor] = &[&DART_LANGUAGE_SERVER];
const PERL_SYNC_SERVERS: &[&LspServerDescriptor] = &[&PERL_NAVIGATOR];
const CMAKE_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CMAKE_LANGUAGE_SERVER];
const ELM_SYNC_SERVERS: &[&LspServerDescriptor] = &[&ELM_LANGUAGE_SERVER];
const ERLANG_SYNC_SERVERS: &[&LspServerDescriptor] = &[&ERLANG_LS];
const CUE_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CUE_LSP];
const SOLIDITY_SYNC_SERVERS: &[&LspServerDescriptor] = &[&SOLIDITY_LANGUAGE_SERVER];
const QML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&QML_LS];

const LANGUAGE_ROUTES: &[LanguageRoutes] = &[
    LanguageRoutes {
        languages: RUST_LANGUAGES,
        sync: RUST_SYNC_SERVERS,
        project_description: "a supported Rust project root (Cargo workspace or rust-project.json)",
    },
    LanguageRoutes {
        languages: PYTHON_LANGUAGES,
        sync: PYTHON_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Python project root (ty.toml, pyproject.toml, setup.py, setup.cfg, requirements.txt, Pipfile, ruff.toml, or .ruff.toml)",
    },
    LanguageRoutes {
        languages: C_FAMILY_LANGUAGES,
        sync: C_FAMILY_SYNC_SERVERS,
        project_description: "the opened file directory or a supported C/C++ project root (.clangd, .clang-tidy, .clang-format, compile_commands.json, compile_flags.txt, or configure.ac)",
    },
    LanguageRoutes {
        languages: CSHARP_LANGUAGES,
        sync: CSHARP_SYNC_SERVERS,
        project_description: "the opened file directory or a supported C# project root (global.json, Directory.Build.props, Directory.Build.targets, or NuGet.Config)",
    },
    LanguageRoutes {
        languages: TYPESCRIPT_LANGUAGES,
        sync: TYPESCRIPT_SYNC_SERVERS,
        project_description: "the opened file directory or a supported JavaScript/TypeScript project root (package-lock.json, yarn.lock, pnpm-lock.yaml, bun.lockb, bun.lock, package.json, tsconfig.json, or jsconfig.json)",
    },
    LanguageRoutes {
        languages: GO_LANGUAGES,
        sync: GO_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Go project root (go.work or go.mod)",
    },
    LanguageRoutes {
        languages: JAVA_LANGUAGES,
        sync: JAVA_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Java project root (mvnw, gradlew, settings.gradle, settings.gradle.kts, build.xml, pom.xml, build.gradle, or build.gradle.kts)",
    },
    LanguageRoutes {
        languages: PHP_LANGUAGES,
        sync: PHP_SYNC_SERVERS,
        project_description: "a supported PHP project root (composer.json, .phpactor.json, or .phpactor.yml)",
    },
    LanguageRoutes {
        languages: SHELL_LANGUAGES,
        sync: SHELL_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
    LanguageRoutes {
        languages: MARKDOWN_LANGUAGES,
        sync: MARKDOWN_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
    LanguageRoutes {
        languages: TOML_LANGUAGES,
        sync: TOML_SYNC_SERVERS,
        project_description: "the opened file directory or a supported TOML project root (taplo.toml, .taplo.toml, or Cargo.toml)",
    },
    LanguageRoutes {
        languages: HTML_LANGUAGES,
        sync: HTML_SYNC_SERVERS,
        project_description: "the opened file directory or a supported HTML project root (package.json)",
    },
    LanguageRoutes {
        languages: CSS_LANGUAGES,
        sync: CSS_SYNC_SERVERS,
        project_description: "the opened file directory or a supported CSS project root (package.json)",
    },
    LanguageRoutes {
        languages: JSON_LANGUAGES,
        sync: JSON_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
    LanguageRoutes {
        languages: YAML_LANGUAGES,
        sync: YAML_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
    LanguageRoutes {
        languages: XML_LANGUAGES,
        sync: XML_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
    LanguageRoutes {
        languages: GRAPHQL_LANGUAGES,
        sync: GRAPHQL_SYNC_SERVERS,
        project_description: "the opened file directory or a supported GraphQL project root (package.json, graphql.config.*, or .graphqlrc*)",
    },
    LanguageRoutes {
        languages: DOCKER_LANGUAGES,
        sync: DOCKER_SYNC_SERVERS,
        project_description: "the opened file directory or a supported container project root (Dockerfile, docker-compose.yml, docker-compose.yaml, compose.yml, or compose.yaml)",
    },
    LanguageRoutes {
        languages: TERRAFORM_LANGUAGES,
        sync: TERRAFORM_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Terraform project root (main.tf, terraform.tf, versions.tf, terraform.tfvars, or .terraform)",
    },
    LanguageRoutes {
        languages: NIX_LANGUAGES,
        sync: NIX_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Nix project root (flake.nix, shell.nix, or default.nix)",
    },
    LanguageRoutes {
        languages: LUA_LANGUAGES,
        sync: LUA_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Lua project root (.luarc.json, .luarc.jsonc, stylua.toml, or .stylua.toml)",
    },
    LanguageRoutes {
        languages: RUBY_LANGUAGES,
        sync: RUBY_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Ruby project root (Gemfile, .solargraph.yml, or Rakefile)",
    },
    LanguageRoutes {
        languages: SWIFT_LANGUAGES,
        sync: SWIFT_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Swift project root (Package.swift)",
    },
    LanguageRoutes {
        languages: KOTLIN_LANGUAGES,
        sync: KOTLIN_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Kotlin project root (settings.gradle, settings.gradle.kts, build.gradle, build.gradle.kts, or pom.xml)",
    },
    LanguageRoutes {
        languages: SCALA_LANGUAGES,
        sync: SCALA_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Scala project root (build.sbt, build.sc, .bsp, or project/)",
    },
    LanguageRoutes {
        languages: R_LANGUAGES,
        sync: R_SYNC_SERVERS,
        project_description: "the opened file directory or a supported R project root (DESCRIPTION, .Rprofile, or renv.lock)",
    },
    LanguageRoutes {
        languages: SQL_LANGUAGES,
        sync: SQL_SYNC_SERVERS,
        project_description: "the opened file directory or a supported SQL project root (sqls.yml or .sqls.yml)",
    },
    LanguageRoutes {
        languages: ZIG_LANGUAGES,
        sync: ZIG_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
    LanguageRoutes {
        languages: JULIA_LANGUAGES,
        sync: JULIA_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Julia project root (Project.toml or Manifest.toml)",
    },
    LanguageRoutes {
        languages: HASKELL_LANGUAGES,
        sync: HASKELL_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Haskell project root (hie.yaml, stack.yaml, or cabal.project)",
    },
    LanguageRoutes {
        languages: OCAML_LANGUAGES,
        sync: OCAML_SYNC_SERVERS,
        project_description: "the opened file directory or a supported OCaml project root (dune-project, dune-workspace, opam, or esy.json)",
    },
    LanguageRoutes {
        languages: FSHARP_LANGUAGES,
        sync: FSHARP_SYNC_SERVERS,
        project_description: "the opened file directory or a supported F# project root (global.json, Directory.Build.props, or Directory.Build.targets)",
    },
    LanguageRoutes {
        languages: DART_LANGUAGES,
        sync: DART_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Dart project root (pubspec.yaml)",
    },
    LanguageRoutes {
        languages: PERL_LANGUAGES,
        sync: PERL_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Perl project root (cpanfile, dist.ini, Build.PL, or Makefile.PL)",
    },
    LanguageRoutes {
        languages: CMAKE_LANGUAGES,
        sync: CMAKE_SYNC_SERVERS,
        project_description: "the opened file directory or a supported CMake project root (CMakeLists.txt, .neocmake.toml, or .neocmakelint.toml)",
    },
    LanguageRoutes {
        languages: ELM_LANGUAGES,
        sync: ELM_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Elm project root (elm.json)",
    },
    LanguageRoutes {
        languages: ERLANG_LANGUAGES,
        sync: ERLANG_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Erlang project root (rebar.config or erlang.mk)",
    },
    LanguageRoutes {
        languages: CUE_LANGUAGES,
        sync: CUE_SYNC_SERVERS,
        project_description: "the opened file directory or a supported CUE project root (cue.mod)",
    },
    LanguageRoutes {
        languages: SOLIDITY_LANGUAGES,
        sync: SOLIDITY_SYNC_SERVERS,
        project_description: "the opened file directory or a supported Solidity project root (foundry.toml, hardhat.config.js, hardhat.config.ts, truffle-config.js, or brownie-config.yaml)",
    },
    LanguageRoutes {
        languages: QML_LANGUAGES,
        sync: QML_SYNC_SERVERS,
        project_description: "the opened file directory",
    },
];

/// Return whether one route kind should use `features`.
///
/// Returns `true` when the server should receive requests for this route kind,
/// and `false` when the route should skip that server.
fn route_kind_is_supported(kind: LspRouteKind, server: &LspServerDescriptor) -> bool {
    match kind {
        LspRouteKind::Sync => true,
        LspRouteKind::Navigation => server.features.navigation,
        LspRouteKind::Hover => server.features.hover,
        LspRouteKind::Rename => server.features.rename,
    }
}

/// Detect the built-in syntax language for one path, if any.
pub(crate) fn language_for_path(path: &Path) -> Option<LanguageId> {
    detect_language_details(Some(path)).map(|(profile, _)| profile.id)
}

/// Return the ordered built-in server route for `language` and request `kind`.
pub(crate) fn route_servers(
    language: LanguageId,
    kind: LspRouteKind,
) -> Vec<&'static LspServerDescriptor> {
    let Some(routes) = routes_for_language(language) else {
        return Vec::new();
    };

    // Sync routes are the source of truth for each language family. The other
    // route kinds filter that shared list by feature flags so multi-server
    // languages such as Python keep one ordered default definition.
    routes
        .sync
        .iter()
        .copied()
        .filter(|server| route_kind_is_supported(kind, server))
        .collect()
}

/// Return the user-facing project-root requirement text for one language.
pub(crate) fn supported_project_description(language: LanguageId) -> &'static str {
    routes_for_language(language)
        .map(|routes| routes.project_description)
        .unwrap_or("a supported project root")
}

/// Return the static route table for one syntax language.
fn routes_for_language(language: LanguageId) -> Option<&'static LanguageRoutes> {
    // Route lookup stays data-driven so the language catalog can grow without
    // duplicating match arms for every feature-specific route list.
    LANGUAGE_ROUTES
        .iter()
        .find(|routes| routes.languages.contains(&language))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::server::LspServerId;

    /// Verify Python routing preserves the intended built-in ownership order.
    #[test]
    fn test_route_servers_for_python_match_feature_policies() {
        let navigation = route_servers(LanguageId::Python, LspRouteKind::Navigation)
            .iter()
            .map(|server| server.id)
            .collect::<Vec<_>>();
        let diagnostics = route_servers(LanguageId::Python, LspRouteKind::Sync)
            .iter()
            .filter(|server| server.features.diagnostics)
            .map(|server| server.id)
            .collect::<Vec<_>>();

        assert_eq!(navigation, vec![LspServerId::Ty, LspServerId::Pylsp]);
        assert_eq!(diagnostics, vec![LspServerId::Ruff, LspServerId::Pylsp]);
    }

    /// Verify JavaScript and TypeScript reuse the same TypeScript server route.
    #[test]
    fn test_route_servers_for_javascript_and_typescript_share_one_server() {
        let javascript = route_servers(LanguageId::JavaScript, LspRouteKind::Navigation)
            .iter()
            .map(|server| server.id)
            .collect::<Vec<_>>();
        let typescript = route_servers(LanguageId::TypeScript, LspRouteKind::Navigation)
            .iter()
            .map(|server| server.id)
            .collect::<Vec<_>>();

        assert_eq!(javascript, vec![LspServerId::TypeScriptLanguageServer]);
        assert_eq!(typescript, vec![LspServerId::TypeScriptLanguageServer]);
    }

    /// Verify the curated built-in defaults cover roughly 50 popular languages.
    #[test]
    fn test_curated_lsp_language_count_is_about_fifty() {
        let supported = LANGUAGE_ROUTES
            .iter()
            .flat_map(|routes| routes.languages.iter())
            .count();

        assert_eq!(supported, 49);
    }

    /// Verify CSS routes expose navigation and rename through the CSS server.
    #[test]
    fn test_route_servers_for_css_enable_navigation_and_rename() {
        assert_eq!(
            route_servers(LanguageId::Css, LspRouteKind::Navigation)
                .iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::CssLanguageServer]
        );
        assert_eq!(
            route_servers(LanguageId::Css, LspRouteKind::Rename)
                .iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::CssLanguageServer]
        );
    }

    /// Verify partial-feature languages keep hover/diagnostics without fake rename support.
    #[test]
    fn test_partial_feature_servers_filter_non_owned_routes() {
        assert!(route_servers(LanguageId::Json, LspRouteKind::Navigation).is_empty());
        assert!(route_servers(LanguageId::Xml, LspRouteKind::Rename).is_empty());
        assert_eq!(
            route_servers(LanguageId::Toml, LspRouteKind::Hover)
                .iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::Taplo]
        );
        assert_eq!(
            route_servers(LanguageId::GraphQl, LspRouteKind::Navigation)
                .iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::GraphqlLanguageService]
        );
    }

    /// Verify user-facing project guidance covers newly added language families.
    #[test]
    fn test_supported_project_descriptions_cover_added_languages() {
        assert!(supported_project_description(LanguageId::Go).contains("go.mod"));
        assert!(supported_project_description(LanguageId::Java).contains("pom.xml"));
        assert!(supported_project_description(LanguageId::Kotlin).contains("build.gradle"));
        assert!(supported_project_description(LanguageId::Hcl).contains("main.tf"));
        assert!(supported_project_description(LanguageId::Nix).contains("flake.nix"));
    }
}
