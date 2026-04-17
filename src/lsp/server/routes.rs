//! Built-in language-server routing policy.

use super::catalog::{
    BASH_LANGUAGE_SERVER, CLANGD, CSS_LANGUAGE_SERVER, GOPLS, HTML_LANGUAGE_SERVER, JDTLS,
    JSON_LANGUAGE_SERVER, LspServerDescriptor, PHPACTOR, PYLSP, RUFF, RUST_ANALYZER, TY,
    TYPESCRIPT_LANGUAGE_SERVER, YAML_LANGUAGE_SERVER,
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

/// Static route tables and project description for one language family.
struct LanguageRoutes {
    sync: &'static [&'static LspServerDescriptor],
    navigation: &'static [&'static LspServerDescriptor],
    hover: &'static [&'static LspServerDescriptor],
    rename: &'static [&'static LspServerDescriptor],
    project_description: &'static str,
}

const RUST_SYNC_SERVERS: &[&LspServerDescriptor] = &[&RUST_ANALYZER];
const PYTHON_SYNC_SERVERS: &[&LspServerDescriptor] = &[&TY, &RUFF, &PYLSP];
const PYTHON_NAVIGATION_SERVERS: &[&LspServerDescriptor] = &[&TY, &PYLSP];
const C_FAMILY_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CLANGD];
const WEB_SYNC_SERVERS: &[&LspServerDescriptor] = &[&TYPESCRIPT_LANGUAGE_SERVER];
const GO_SYNC_SERVERS: &[&LspServerDescriptor] = &[&GOPLS];
const JAVA_SYNC_SERVERS: &[&LspServerDescriptor] = &[&JDTLS];
const PHP_SYNC_SERVERS: &[&LspServerDescriptor] = &[&PHPACTOR];
const SHELL_SYNC_SERVERS: &[&LspServerDescriptor] = &[&BASH_LANGUAGE_SERVER];
const HTML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&HTML_LANGUAGE_SERVER];
const CSS_SYNC_SERVERS: &[&LspServerDescriptor] = &[&CSS_LANGUAGE_SERVER];
const JSON_SYNC_SERVERS: &[&LspServerDescriptor] = &[&JSON_LANGUAGE_SERVER];
const YAML_SYNC_SERVERS: &[&LspServerDescriptor] = &[&YAML_LANGUAGE_SERVER];
const NO_SERVERS: &[&LspServerDescriptor] = &[];

const RUST_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: RUST_SYNC_SERVERS,
    navigation: RUST_SYNC_SERVERS,
    hover: RUST_SYNC_SERVERS,
    rename: RUST_SYNC_SERVERS,
    project_description: "a supported Rust project root (Cargo workspace or rust-project.json)",
};
const PYTHON_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: PYTHON_SYNC_SERVERS,
    navigation: PYTHON_NAVIGATION_SERVERS,
    hover: PYTHON_NAVIGATION_SERVERS,
    rename: PYTHON_NAVIGATION_SERVERS,
    project_description: "the opened file directory or a supported Python project root (ty.toml, pyproject.toml, setup.py, setup.cfg, requirements.txt, Pipfile, ruff.toml, or .ruff.toml)",
};
const C_FAMILY_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: C_FAMILY_SYNC_SERVERS,
    navigation: C_FAMILY_SYNC_SERVERS,
    hover: C_FAMILY_SYNC_SERVERS,
    rename: C_FAMILY_SYNC_SERVERS,
    project_description: "the opened file directory or a supported C/C++ project root (.clangd, .clang-tidy, .clang-format, compile_commands.json, compile_flags.txt, or configure.ac)",
};
const WEB_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: WEB_SYNC_SERVERS,
    navigation: WEB_SYNC_SERVERS,
    hover: WEB_SYNC_SERVERS,
    rename: WEB_SYNC_SERVERS,
    project_description: "the opened file directory or a supported JavaScript/TypeScript project root (package-lock.json, yarn.lock, pnpm-lock.yaml, bun.lockb, bun.lock, package.json, tsconfig.json, jsconfig.json, or .git)",
};
const GO_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: GO_SYNC_SERVERS,
    navigation: GO_SYNC_SERVERS,
    hover: GO_SYNC_SERVERS,
    rename: GO_SYNC_SERVERS,
    project_description: "the opened file directory or a supported Go project root (go.work, go.mod, or .git)",
};
const JAVA_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: JAVA_SYNC_SERVERS,
    navigation: JAVA_SYNC_SERVERS,
    hover: JAVA_SYNC_SERVERS,
    rename: JAVA_SYNC_SERVERS,
    project_description: "the opened file directory or a supported Java project root (mvnw, gradlew, settings.gradle, settings.gradle.kts, build.xml, pom.xml, build.gradle, build.gradle.kts, or .git)",
};
const PHP_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: PHP_SYNC_SERVERS,
    navigation: PHP_SYNC_SERVERS,
    hover: PHP_SYNC_SERVERS,
    rename: PHP_SYNC_SERVERS,
    project_description: "a supported PHP project root (.git, composer.json, .phpactor.json, or .phpactor.yml)",
};
const SHELL_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: SHELL_SYNC_SERVERS,
    navigation: SHELL_SYNC_SERVERS,
    hover: SHELL_SYNC_SERVERS,
    rename: SHELL_SYNC_SERVERS,
    project_description: "the opened file directory or a supported shell project root (.git)",
};
const HTML_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: HTML_SYNC_SERVERS,
    navigation: NO_SERVERS,
    hover: HTML_SYNC_SERVERS,
    rename: NO_SERVERS,
    project_description: "the opened file directory or a supported HTML project root (package.json or .git)",
};
const CSS_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: CSS_SYNC_SERVERS,
    navigation: NO_SERVERS,
    hover: CSS_SYNC_SERVERS,
    rename: NO_SERVERS,
    project_description: "the opened file directory or a supported CSS project root (package.json or .git)",
};
const JSON_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: JSON_SYNC_SERVERS,
    navigation: NO_SERVERS,
    hover: JSON_SYNC_SERVERS,
    rename: NO_SERVERS,
    project_description: "the opened file directory or a supported JSON project root (.git)",
};
const YAML_ROUTES: LanguageRoutes = LanguageRoutes {
    sync: YAML_SYNC_SERVERS,
    navigation: NO_SERVERS,
    hover: YAML_SYNC_SERVERS,
    rename: NO_SERVERS,
    project_description: "the opened file directory or a supported YAML project root (.git)",
};

/// Detect the built-in syntax language for one path, if any.
pub(crate) fn language_for_path(path: &Path) -> Option<LanguageId> {
    detect_language_details(Some(path)).map(|(profile, _)| profile.id)
}

/// Return the ordered built-in server route for `language` and request `kind`.
pub(crate) fn route_servers(
    language: LanguageId,
    kind: LspRouteKind,
) -> &'static [&'static LspServerDescriptor] {
    let Some(routes) = routes_for_language(language) else {
        return NO_SERVERS;
    };
    match kind {
        LspRouteKind::Sync => routes.sync,
        LspRouteKind::Navigation => routes.navigation,
        LspRouteKind::Hover => routes.hover,
        LspRouteKind::Rename => routes.rename,
    }
}

/// Return the user-facing project-root requirement text for one language.
pub(crate) fn supported_project_description(language: LanguageId) -> &'static str {
    routes_for_language(language)
        .map(|routes| routes.project_description)
        .unwrap_or("a supported project root")
}

/// Return the static route table for one syntax language.
fn routes_for_language(language: LanguageId) -> Option<&'static LanguageRoutes> {
    // Route lookup stays explicit so each language family can keep its own
    // capability mix without forcing unrelated servers into shared tables.
    match language {
        LanguageId::Rust => Some(&RUST_ROUTES),
        LanguageId::Python => Some(&PYTHON_ROUTES),
        LanguageId::C | LanguageId::Cpp => Some(&C_FAMILY_ROUTES),
        LanguageId::JavaScript | LanguageId::TypeScript => Some(&WEB_ROUTES),
        LanguageId::Go => Some(&GO_ROUTES),
        LanguageId::Java => Some(&JAVA_ROUTES),
        LanguageId::Php => Some(&PHP_ROUTES),
        LanguageId::Bash | LanguageId::Sh | LanguageId::Zsh | LanguageId::Fish => {
            Some(&SHELL_ROUTES)
        }
        LanguageId::Html | LanguageId::Xhtml => Some(&HTML_ROUTES),
        LanguageId::Css | LanguageId::Scss | LanguageId::Less => Some(&CSS_ROUTES),
        LanguageId::Json | LanguageId::JsonC => Some(&JSON_ROUTES),
        LanguageId::Yaml => Some(&YAML_ROUTES),
        _ => None,
    }
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
            .copied()
            .map(|server| server.id)
            .collect::<Vec<_>>();
        let diagnostics = route_servers(LanguageId::Python, LspRouteKind::Sync)
            .iter()
            .copied()
            .filter(|server| server.features.diagnostics)
            .map(|server| server.id)
            .collect::<Vec<_>>();

        assert_eq!(navigation, vec![LspServerId::Ty, LspServerId::Pylsp]);
        assert_eq!(diagnostics, vec![LspServerId::Ruff, LspServerId::Pylsp]);
    }

    /// Verify JavaScript and TypeScript reuse the same shared server route.
    #[test]
    fn test_route_servers_for_web_languages_share_one_server() {
        // The runtime should treat JS and TS as separate syntax languages while
        // still reusing the same transport descriptor and startup policy.
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

    /// Verify structured-data languages expose hover/diagnostics without navigation routes.
    #[test]
    fn test_route_servers_for_structured_data_languages_disable_navigation() {
        // These servers are intentionally scoped to validation-oriented editor
        // features, so navigation stays disabled even though sync is enabled.
        assert!(route_servers(LanguageId::Json, LspRouteKind::Navigation).is_empty());
        assert!(route_servers(LanguageId::Yaml, LspRouteKind::Navigation).is_empty());
        assert_eq!(
            route_servers(LanguageId::Json, LspRouteKind::Hover)
                .iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::JsonLanguageServer]
        );
        assert_eq!(
            route_servers(LanguageId::Yaml, LspRouteKind::Hover)
                .iter()
                .map(|server| server.id)
                .collect::<Vec<_>>(),
            vec![LspServerId::YamlLanguageServer]
        );
    }

    /// Verify user-facing project guidance covers the newly added language families.
    #[test]
    fn test_supported_project_descriptions_cover_added_languages() {
        assert!(supported_project_description(LanguageId::Go).contains("go.mod"));
        assert!(supported_project_description(LanguageId::Java).contains("pom.xml"));
        assert!(supported_project_description(LanguageId::Php).contains("composer.json"));
        assert!(supported_project_description(LanguageId::JavaScript).contains("package.json"));
    }
}
