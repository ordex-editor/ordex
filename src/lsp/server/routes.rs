//! Built-in language-server routing policy.

use super::catalog::{
    BASH_LANGUAGE_SERVER, CLANGD, CMAKE_LANGUAGE_SERVER, CSHARP_LS, CSS_LANGUAGE_SERVER, CUE_LSP,
    DART_LANGUAGE_SERVER, DOCKER_LANGUAGE_SERVER, ELM_LANGUAGE_SERVER, ERLANG_LS, FSAUTOCOMPLETE,
    GOPLS, GRAPHQL_LANGUAGE_SERVICE, HASKELL_LANGUAGE_SERVER, HTML_LANGUAGE_SERVER, JDTLS,
    JSON_LANGUAGE_SERVER, JULIA_LANGUAGE_SERVER, KOTLIN_LSP, LEMMINX, LUA_LANGUAGE_SERVER,
    LspServerDescriptor, MARKSMAN, METALS, NIL, OCAML_LSP, PERL_NAVIGATOR, PHPACTOR, PYLSP,
    ProjectDetection, QML_LS, R_LANGUAGE_SERVER, RUFF, RUST_ANALYZER, SOLARGRAPH,
    SOLIDITY_LANGUAGE_SERVER, SOURCEKIT_LSP, SQLS, TAPLO, TERRAFORM_LS, TY,
    TYPESCRIPT_LANGUAGE_SERVER, YAML_LANGUAGE_SERVER, ZLS,
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
    CodeAction,
    Completion,
    SignatureHelp,
}

/// Maximum number of cooperating built-in servers for one language route.
const MAX_ROUTE_SERVERS: usize = 3;

/// One non-allocating ordered route result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RouteServers {
    servers: [Option<&'static LspServerDescriptor>; MAX_ROUTE_SERVERS],
}

/// Owning iterator over one `RouteServers` result.
pub(crate) struct RouteServersIter {
    servers: [Option<&'static LspServerDescriptor>; MAX_ROUTE_SERVERS],
    index: usize,
}

impl RouteServers {
    /// Return an empty route result.
    const fn empty() -> Self {
        Self {
            servers: [None; MAX_ROUTE_SERVERS],
        }
    }

    /// Build one filtered route result from one sync server slice.
    fn from_sync(kind: LspRouteKind, sync: &'static [&'static LspServerDescriptor]) -> Self {
        let mut route = Self::empty();
        let mut index = 0;

        // Sync routes remain the source of truth. Other route kinds keep that
        // ownership order while filtering out servers that do not expose the
        // requested capability.
        for server in sync {
            if !route_kind_is_supported(kind, server) {
                continue;
            }
            if index == route.servers.len() {
                break;
            }
            route.servers[index] = Some(*server);
            index += 1;
        }

        route
    }

    /// Return whether the route contains no servers.
    ///
    /// Returns `true` when no built-in server can own the requested route, and
    /// `false` when at least one server is available.
    pub(crate) fn is_empty(&self) -> bool {
        self.servers[0].is_none()
    }
}

impl IntoIterator for RouteServers {
    type Item = &'static LspServerDescriptor;
    type IntoIter = RouteServersIter;

    fn into_iter(self) -> Self::IntoIter {
        RouteServersIter {
            servers: self.servers,
            index: 0,
        }
    }
}

impl Iterator for RouteServersIter {
    type Item = &'static LspServerDescriptor;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.servers.len() {
            let next = self.servers[self.index];
            self.index += 1;
            if let Some(server) = next {
                return Some(server);
            }
        }
        None
    }
}

/// Static route table and generated project description label for one language family.
struct LanguageRoutes {
    languages: &'static [LanguageId],
    sync: &'static [&'static LspServerDescriptor],
    project_label: &'static str,
}

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
        languages: RUST_ANALYZER.supported_languages(),
        sync: RUST_SYNC_SERVERS,
        project_label: "Rust",
    },
    LanguageRoutes {
        languages: TY.supported_languages(),
        sync: PYTHON_SYNC_SERVERS,
        project_label: "Python",
    },
    LanguageRoutes {
        languages: CLANGD.supported_languages(),
        sync: C_FAMILY_SYNC_SERVERS,
        project_label: "C/C++",
    },
    LanguageRoutes {
        languages: CSHARP_LS.supported_languages(),
        sync: CSHARP_SYNC_SERVERS,
        project_label: "C#",
    },
    LanguageRoutes {
        languages: TYPESCRIPT_LANGUAGE_SERVER.supported_languages(),
        sync: TYPESCRIPT_SYNC_SERVERS,
        project_label: "JavaScript/TypeScript",
    },
    LanguageRoutes {
        languages: GOPLS.supported_languages(),
        sync: GO_SYNC_SERVERS,
        project_label: "Go",
    },
    LanguageRoutes {
        languages: JDTLS.supported_languages(),
        sync: JAVA_SYNC_SERVERS,
        project_label: "Java",
    },
    LanguageRoutes {
        languages: PHPACTOR.supported_languages(),
        sync: PHP_SYNC_SERVERS,
        project_label: "PHP",
    },
    LanguageRoutes {
        languages: BASH_LANGUAGE_SERVER.supported_languages(),
        sync: SHELL_SYNC_SERVERS,
        project_label: "shell script",
    },
    LanguageRoutes {
        languages: MARKSMAN.supported_languages(),
        sync: MARKDOWN_SYNC_SERVERS,
        project_label: "Markdown",
    },
    LanguageRoutes {
        languages: TAPLO.supported_languages(),
        sync: TOML_SYNC_SERVERS,
        project_label: "TOML",
    },
    LanguageRoutes {
        languages: HTML_LANGUAGE_SERVER.supported_languages(),
        sync: HTML_SYNC_SERVERS,
        project_label: "HTML",
    },
    LanguageRoutes {
        languages: CSS_LANGUAGE_SERVER.supported_languages(),
        sync: CSS_SYNC_SERVERS,
        project_label: "CSS",
    },
    LanguageRoutes {
        languages: JSON_LANGUAGE_SERVER.supported_languages(),
        sync: JSON_SYNC_SERVERS,
        project_label: "JSON",
    },
    LanguageRoutes {
        languages: YAML_LANGUAGE_SERVER.supported_languages(),
        sync: YAML_SYNC_SERVERS,
        project_label: "YAML",
    },
    LanguageRoutes {
        languages: LEMMINX.supported_languages(),
        sync: XML_SYNC_SERVERS,
        project_label: "XML",
    },
    LanguageRoutes {
        languages: GRAPHQL_LANGUAGE_SERVICE.supported_languages(),
        sync: GRAPHQL_SYNC_SERVERS,
        project_label: "GraphQL",
    },
    LanguageRoutes {
        languages: DOCKER_LANGUAGE_SERVER.supported_languages(),
        sync: DOCKER_SYNC_SERVERS,
        project_label: "container",
    },
    LanguageRoutes {
        languages: TERRAFORM_LS.supported_languages(),
        sync: TERRAFORM_SYNC_SERVERS,
        project_label: "Terraform",
    },
    LanguageRoutes {
        languages: NIL.supported_languages(),
        sync: NIX_SYNC_SERVERS,
        project_label: "Nix",
    },
    LanguageRoutes {
        languages: LUA_LANGUAGE_SERVER.supported_languages(),
        sync: LUA_SYNC_SERVERS,
        project_label: "Lua",
    },
    LanguageRoutes {
        languages: SOLARGRAPH.supported_languages(),
        sync: RUBY_SYNC_SERVERS,
        project_label: "Ruby",
    },
    LanguageRoutes {
        languages: SOURCEKIT_LSP.supported_languages(),
        sync: SWIFT_SYNC_SERVERS,
        project_label: "Swift",
    },
    LanguageRoutes {
        languages: KOTLIN_LSP.supported_languages(),
        sync: KOTLIN_SYNC_SERVERS,
        project_label: "Kotlin",
    },
    LanguageRoutes {
        languages: METALS.supported_languages(),
        sync: SCALA_SYNC_SERVERS,
        project_label: "Scala",
    },
    LanguageRoutes {
        languages: R_LANGUAGE_SERVER.supported_languages(),
        sync: R_SYNC_SERVERS,
        project_label: "R",
    },
    LanguageRoutes {
        languages: SQLS.supported_languages(),
        sync: SQL_SYNC_SERVERS,
        project_label: "SQL",
    },
    LanguageRoutes {
        languages: ZLS.supported_languages(),
        sync: ZIG_SYNC_SERVERS,
        project_label: "Zig",
    },
    LanguageRoutes {
        languages: JULIA_LANGUAGE_SERVER.supported_languages(),
        sync: JULIA_SYNC_SERVERS,
        project_label: "Julia",
    },
    LanguageRoutes {
        languages: HASKELL_LANGUAGE_SERVER.supported_languages(),
        sync: HASKELL_SYNC_SERVERS,
        project_label: "Haskell",
    },
    LanguageRoutes {
        languages: OCAML_LSP.supported_languages(),
        sync: OCAML_SYNC_SERVERS,
        project_label: "OCaml",
    },
    LanguageRoutes {
        languages: FSAUTOCOMPLETE.supported_languages(),
        sync: FSHARP_SYNC_SERVERS,
        project_label: "F#",
    },
    LanguageRoutes {
        languages: DART_LANGUAGE_SERVER.supported_languages(),
        sync: DART_SYNC_SERVERS,
        project_label: "Dart",
    },
    LanguageRoutes {
        languages: PERL_NAVIGATOR.supported_languages(),
        sync: PERL_SYNC_SERVERS,
        project_label: "Perl",
    },
    LanguageRoutes {
        languages: CMAKE_LANGUAGE_SERVER.supported_languages(),
        sync: CMAKE_SYNC_SERVERS,
        project_label: "CMake",
    },
    LanguageRoutes {
        languages: ELM_LANGUAGE_SERVER.supported_languages(),
        sync: ELM_SYNC_SERVERS,
        project_label: "Elm",
    },
    LanguageRoutes {
        languages: ERLANG_LS.supported_languages(),
        sync: ERLANG_SYNC_SERVERS,
        project_label: "Erlang",
    },
    LanguageRoutes {
        languages: CUE_LSP.supported_languages(),
        sync: CUE_SYNC_SERVERS,
        project_label: "CUE",
    },
    LanguageRoutes {
        languages: SOLIDITY_LANGUAGE_SERVER.supported_languages(),
        sync: SOLIDITY_SYNC_SERVERS,
        project_label: "Solidity",
    },
    LanguageRoutes {
        languages: QML_LS.supported_languages(),
        sync: QML_SYNC_SERVERS,
        project_label: "QML",
    },
];

/// Return whether one route kind should use the server feature flags.
///
/// Returns `true` when the server should receive requests for this route kind,
/// and `false` when the route should skip that server.
fn route_kind_is_supported(kind: LspRouteKind, server: &LspServerDescriptor) -> bool {
    match kind {
        LspRouteKind::Sync => true,
        LspRouteKind::Navigation => server.features.navigation,
        LspRouteKind::Hover => server.features.hover,
        LspRouteKind::Rename => server.features.rename,
        LspRouteKind::CodeAction => server.features.code_action,
        LspRouteKind::Completion => server.features.completion,
        // Built-in descriptors do not yet split completion and signature-help
        // ownership, so both interactive insert-mode routes share the same flag.
        LspRouteKind::SignatureHelp => server.features.completion,
    }
}

/// Render one natural-language marker list.
fn format_marker_list(markers: &[&'static str]) -> String {
    match markers {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let mut rendered = String::new();
            for (index, marker) in markers.iter().enumerate() {
                if index > 0 {
                    if index + 1 == markers.len() {
                        rendered.push_str(", or ");
                    } else {
                        rendered.push_str(", ");
                    }
                }
                rendered.push_str(marker);
            }
            rendered
        }
    }
}

/// Build the user-facing project description for one language route.
fn generated_project_description(routes: &LanguageRoutes) -> String {
    let mut markers = Vec::new();
    let mut fallback_to_file_directory = false;

    for server in routes.sync {
        match server.project_detection() {
            ProjectDetection::RustWorkspace => {
                return format!(
                    "a supported {} project root (Cargo workspace or rust-project.json)",
                    routes.project_label
                );
            }
            ProjectDetection::MarkerBased {
                markers: server_markers,
                fallback_to_file_directory: server_fallback,
            } => {
                fallback_to_file_directory |= server_fallback;
                for marker in server_markers {
                    if !markers.contains(marker) {
                        markers.push(*marker);
                    }
                }
            }
        }
    }

    if markers.is_empty() {
        return "the opened file directory".to_string();
    }

    let marker_list = format_marker_list(&markers);
    if fallback_to_file_directory {
        return format!(
            "the opened file directory or a supported {} project root ({marker_list})",
            routes.project_label
        );
    }

    format!(
        "a supported {} project root ({marker_list})",
        routes.project_label
    )
}

/// Detect the built-in syntax language for one path, if any.
pub(crate) fn language_for_path(path: &Path) -> Option<LanguageId> {
    detect_language_details(Some(path)).map(|(profile, _)| profile.id)
}

/// Return the ordered built-in server route for `language` and request `kind`.
pub(crate) fn route_servers(language: LanguageId, kind: LspRouteKind) -> RouteServers {
    let Some(routes) = routes_for_language(language) else {
        return RouteServers::empty();
    };
    RouteServers::from_sync(kind, routes.sync)
}

/// Return the user-facing project-root requirement text for one language.
pub(crate) fn supported_project_description(language: LanguageId) -> String {
    routes_for_language(language)
        .map(generated_project_description)
        .unwrap_or_else(|| "a supported project root".to_string())
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
            .into_iter()
            .map(|server| server.id)
            .collect::<Vec<_>>();
        let diagnostics = route_servers(LanguageId::Python, LspRouteKind::Sync)
            .into_iter()
            .filter(|server| server.features.diagnostics)
            .map(|server| server.id)
            .collect::<Vec<_>>();

        assert_eq!(navigation, vec![LspServerId::Ty, LspServerId::Pylsp]);
        assert_eq!(diagnostics, vec![LspServerId::Ruff, LspServerId::Pylsp]);
    }

    /// Verify JavaScript and TypeScript reuse the same TypeScript server route.
    #[test]
    fn test_route_servers_for_javascript_and_typescript_share_one_server() {
        // The runtime should treat JS and TS as separate syntax languages while
        // still reusing the same transport descriptor and startup policy.
        let javascript = route_servers(LanguageId::JavaScript, LspRouteKind::Navigation)
            .into_iter()
            .map(|server| server.id)
            .collect::<Vec<_>>();
        let typescript = route_servers(LanguageId::TypeScript, LspRouteKind::Navigation)
            .into_iter()
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
                .into_iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::CssLanguageServer]
        );
        assert_eq!(
            route_servers(LanguageId::Css, LspRouteKind::Rename)
                .into_iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::CssLanguageServer]
        );
    }

    /// Verify partial-feature languages keep hover/diagnostics without rename routes.
    #[test]
    fn test_partial_feature_servers_filter_non_owned_routes() {
        assert!(route_servers(LanguageId::Json, LspRouteKind::Navigation).is_empty());
        assert!(route_servers(LanguageId::Xml, LspRouteKind::Rename).is_empty());
        assert_eq!(
            route_servers(LanguageId::Toml, LspRouteKind::Hover)
                .into_iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::Taplo]
        );
        assert_eq!(
            route_servers(LanguageId::GraphQl, LspRouteKind::Navigation)
                .into_iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::GraphqlLanguageService]
        );
    }

    /// Verify generated project guidance mentions representative route markers.
    #[test]
    fn test_supported_project_descriptions_cover_curated_languages() {
        assert!(supported_project_description(LanguageId::Go).contains("go.mod"));
        assert!(supported_project_description(LanguageId::Java).contains("pom.xml"));
        assert!(supported_project_description(LanguageId::Kotlin).contains("build.gradle"));
        assert!(supported_project_description(LanguageId::Hcl).contains("main.tf"));
        assert!(supported_project_description(LanguageId::Nix).contains("flake.nix"));
    }
}
