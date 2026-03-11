//! Built-in syntax profile registry.
//!
//! Phase 1 keeps detection intentionally simple: exact filename matches win,
//! then file extensions are checked, and unmatched paths fall back to plain text.

use crate::syntax::engine::DetectionSource;
use crate::syntax::profile::LanguageProfile;
use std::path::Path;

pub(crate) mod d;
pub(crate) mod markdown;
pub(crate) mod rust;
pub(crate) mod toml;

/// Return all built-in language profiles.
pub(crate) fn builtin_profiles() -> &'static [LanguageProfile] {
    static PROFILES: [LanguageProfile; 4] =
        [rust::PROFILE, toml::PROFILE, markdown::PROFILE, d::PROFILE];
    &PROFILES
}

/// Detect a language profile for `path`, if any.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn detect_language(path: Option<&Path>) -> Option<&'static LanguageProfile> {
    detect_language_details(path).map(|(profile, _)| profile)
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
            .find(|profile| profile.detection.exact_filenames.contains(&file_name))
    {
        return Some((profile, DetectionSource::MatchByFilename));
    }

    builtin_profiles()
        .iter()
        .find(|profile| profile.matches_path(path))
        .map(|profile| (profile, DetectionSource::MatchByExtension))
}
