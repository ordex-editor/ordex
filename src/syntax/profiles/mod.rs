//! Built-in syntax profile registry.
//!
//! Detection stays intentionally simple: exact filename matches win, then file
//! extensions are checked, and unmatched paths fall back to plain text.

use crate::syntax::engine::DetectionSource;
use crate::syntax::profile::LanguageProfile;
use std::path::Path;

pub(crate) mod asciidoc;
pub(crate) mod c;
pub(crate) mod cpp;
pub(crate) mod csharp;
pub(crate) mod d;
pub(crate) mod go;
pub(crate) mod java;
pub(crate) mod javascript;
pub(crate) mod markdown;
pub(crate) mod php;
pub(crate) mod python;
pub(crate) mod rust;
pub(crate) mod toml;
pub(crate) mod typescript;

/// Return all built-in language profiles.
pub(crate) fn builtin_profiles() -> &'static [LanguageProfile] {
    static PROFILES: [LanguageProfile; 14] = [
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
