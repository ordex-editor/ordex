//! Built-in syntax profile registry.
//!
//! Detection stays intentionally simple: exact filename matches win, then file
//! extensions are checked, and unmatched paths fall back to plain text.

use crate::syntax::engine::DetectionSource;
use crate::syntax::profile::{LanguageId, LanguageProfile};
use std::path::Path;

pub(crate) mod asciidoc;
pub(crate) mod awk;
pub(crate) mod bash;
pub(crate) mod c;
pub(crate) mod cmake;
pub(crate) mod coffeescript;
pub(crate) mod cpp;
pub(crate) mod crystal;
pub(crate) mod csharp;
pub(crate) mod css;
pub(crate) mod cue;
pub(crate) mod d;
pub(crate) mod dart;
pub(crate) mod dockerfile;
pub(crate) mod elixir;
pub(crate) mod elm;
pub(crate) mod erlang;
pub(crate) mod fish;
pub(crate) mod fsharp;
pub(crate) mod gas;
pub(crate) mod go;
pub(crate) mod graphql;
pub(crate) mod groovy;
pub(crate) mod haskell;
pub(crate) mod hcl;
pub(crate) mod html;
pub(crate) mod ini;
pub(crate) mod java;
pub(crate) mod javascript;
pub(crate) mod json;
pub(crate) mod jsonc;
pub(crate) mod julia;
pub(crate) mod kconfig;
pub(crate) mod kotlin;
pub(crate) mod less;
pub(crate) mod lisp;
pub(crate) mod lua;
pub(crate) mod make;
pub(crate) mod markdown;
pub(crate) mod masm;
pub(crate) mod meson;
pub(crate) mod nasm;
pub(crate) mod nim;
pub(crate) mod ninja;
pub(crate) mod nix;
pub(crate) mod ocaml;
pub(crate) mod perl;
pub(crate) mod php;
pub(crate) mod pkgbuild;
pub(crate) mod proto;
pub(crate) mod python;
pub(crate) mod qml;
pub(crate) mod r;
pub(crate) mod ruby;
pub(crate) mod rust;
pub(crate) mod sass;
pub(crate) mod scala;
pub(crate) mod scss;
pub(crate) mod sh;
pub(crate) mod solidity;
pub(crate) mod sql;
pub(crate) mod swift;
pub(crate) mod thrift;
pub(crate) mod toml;
pub(crate) mod typescript;
pub(crate) mod vala;
pub(crate) mod xhtml;
pub(crate) mod xml;
pub(crate) mod yaml;
pub(crate) mod yasm;
pub(crate) mod zig;
pub(crate) mod zsh;

/// Return all built-in language profiles.
pub(crate) fn builtin_profiles() -> &'static [LanguageProfile] {
    static PROFILES: [LanguageProfile; 72] = [
        rust::PROFILE,
        toml::PROFILE,
        markdown::PROFILE,
        d::PROFILE,
        javascript::PROFILE,
        typescript::PROFILE,
        python::PROFILE,
        java::PROFILE,
        csharp::PROFILE,
        cpp::PROFILE,
        go::PROFILE,
        c::PROFILE,
        php::PROFILE,
        asciidoc::PROFILE,
        bash::PROFILE,
        sh::PROFILE,
        zsh::PROFILE,
        fish::PROFILE,
        json::PROFILE,
        jsonc::PROFILE,
        yaml::PROFILE,
        ini::PROFILE,
        css::PROFILE,
        scss::PROFILE,
        less::PROFILE,
        xml::PROFILE,
        proto::PROFILE,
        thrift::PROFILE,
        erlang::PROFILE,
        elm::PROFILE,
        cmake::PROFILE,
        meson::PROFILE,
        ninja::PROFILE,
        dockerfile::PROFILE,
        hcl::PROFILE,
        nix::PROFILE,
        kconfig::PROFILE,
        pkgbuild::PROFILE,
        lua::PROFILE,
        ruby::PROFILE,
        swift::PROFILE,
        kotlin::PROFILE,
        scala::PROFILE,
        r::PROFILE,
        sql::PROFILE,
        zig::PROFILE,
        julia::PROFILE,
        haskell::PROFILE,
        ocaml::PROFILE,
        fsharp::PROFILE,
        elixir::PROFILE,
        groovy::PROFILE,
        dart::PROFILE,
        perl::PROFILE,
        awk::PROFILE,
        solidity::PROFILE,
        vala::PROFILE,
        nim::PROFILE,
        crystal::PROFILE,
        coffeescript::PROFILE,
        graphql::PROFILE,
        cue::PROFILE,
        sass::PROFILE,
        qml::PROFILE,
        make::PROFILE,
        html::PROFILE,
        xhtml::PROFILE,
        gas::PROFILE,
        nasm::PROFILE,
        masm::PROFILE,
        yasm::PROFILE,
        lisp::PROFILE,
    ];
    &PROFILES
}

/// Detect a language profile and report how the match was found.
pub(crate) fn detect_language_details(
    path: Option<&Path>,
) -> Option<(&'static LanguageProfile, DetectionSource)> {
    let path = path?;
    let file_name = path.file_name().and_then(|name| name.to_str());
    if let Some(file_name) = file_name
        && let Some(profile) = builtin_profiles()
            .iter()
            .find(|profile| profile.exact_filenames.contains(&file_name))
    {
        return Some((profile, DetectionSource::MatchByFilename));
    }

    builtin_profiles()
        .iter()
        .find(|profile| profile.matches_path(path))
        .map(|profile| (profile, DetectionSource::MatchByExtension))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CorrespondingExtensionRule {
    source_extension: &'static str,
    target_extensions: &'static [&'static str],
}

const C_TO_H: &[&str] = &["h"];
const H_TO_C: &[&str] = &["c"];
const C_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "c",
        target_extensions: C_TO_H,
    },
    CorrespondingExtensionRule {
        source_extension: "h",
        target_extensions: H_TO_C,
    },
];

const CPP_TO_HEADERS: &[&str] = &["hpp", "hh", "hxx"];
const CPP_HEADERS_TO_SOURCE: &[&str] = &["cc", "cpp", "cxx"];
const CPP_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "cc",
        target_extensions: CPP_TO_HEADERS,
    },
    CorrespondingExtensionRule {
        source_extension: "cpp",
        target_extensions: CPP_TO_HEADERS,
    },
    CorrespondingExtensionRule {
        source_extension: "cxx",
        target_extensions: CPP_TO_HEADERS,
    },
    CorrespondingExtensionRule {
        source_extension: "hpp",
        target_extensions: CPP_HEADERS_TO_SOURCE,
    },
    CorrespondingExtensionRule {
        source_extension: "hh",
        target_extensions: CPP_HEADERS_TO_SOURCE,
    },
    CorrespondingExtensionRule {
        source_extension: "hxx",
        target_extensions: CPP_HEADERS_TO_SOURCE,
    },
];

const OCAML_ML_TO_MLI: &[&str] = &["mli"];
const OCAML_MLI_TO_ML: &[&str] = &["ml"];
const OCAML_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "ml",
        target_extensions: OCAML_ML_TO_MLI,
    },
    CorrespondingExtensionRule {
        source_extension: "mli",
        target_extensions: OCAML_MLI_TO_ML,
    },
];

const ERLANG_ERL_TO_HRL: &[&str] = &["hrl"];
const ERLANG_HRL_TO_ERL: &[&str] = &["erl"];
const ERLANG_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "erl",
        target_extensions: ERLANG_ERL_TO_HRL,
    },
    CorrespondingExtensionRule {
        source_extension: "hrl",
        target_extensions: ERLANG_HRL_TO_ERL,
    },
];

const PYTHON_PY_TO_PYI: &[&str] = &["pyi"];
const PYTHON_PYI_TO_PY: &[&str] = &["py"];
const PYTHON_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "py",
        target_extensions: PYTHON_PY_TO_PYI,
    },
    CorrespondingExtensionRule {
        source_extension: "pyi",
        target_extensions: PYTHON_PYI_TO_PY,
    },
];

const FSHARP_FS_TO_FSI: &[&str] = &["fsi"];
const FSHARP_FSI_TO_FS: &[&str] = &["fs"];
const FSHARP_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "fs",
        target_extensions: FSHARP_FS_TO_FSI,
    },
    CorrespondingExtensionRule {
        source_extension: "fsi",
        target_extensions: FSHARP_FSI_TO_FS,
    },
];

const VALA_TO_VAPI: &[&str] = &["vapi"];
const VAPI_TO_VALA: &[&str] = &["vala"];
const VALA_RULES: &[CorrespondingExtensionRule] = &[
    CorrespondingExtensionRule {
        source_extension: "vala",
        target_extensions: VALA_TO_VAPI,
    },
    CorrespondingExtensionRule {
        source_extension: "vapi",
        target_extensions: VAPI_TO_VALA,
    },
];

/// Return ordered corresponding extensions for `source_extension` in `profile`.
pub(crate) fn corresponding_extensions_for(
    profile: &LanguageProfile,
    source_extension: &str,
) -> Option<&'static [&'static str]> {
    // Language-specific pairing rules live in the profile subsystem so
    // navigation features do not encode any language constants directly.
    let rules = match profile.id {
        LanguageId::C => C_RULES,
        LanguageId::Cpp => CPP_RULES,
        LanguageId::Ocaml => OCAML_RULES,
        LanguageId::Erlang => ERLANG_RULES,
        LanguageId::Python => PYTHON_RULES,
        LanguageId::FSharp => FSHARP_RULES,
        LanguageId::Vala => VALA_RULES,
        _ => &[],
    };
    rules
        .iter()
        .find(|rule| rule.source_extension == source_extension)
        .map(|rule| rule.target_extensions)
}
