//! Watcher registrations and glob matching for `workspace/didChangeWatchedFiles`.

use std::path::Path;

/// Kind of filesystem change reported to a language server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LspFileChangeKind {
    Created,
    Changed,
}

impl LspFileChangeKind {
    /// Return the `FileChangeType` value the protocol uses for this kind.
    pub(crate) fn protocol_value(self) -> u8 {
        match self {
            Self::Created => 1,
            Self::Changed => 2,
        }
    }

    /// Return the `WatchKind` bit a registration must set to receive this kind.
    fn watch_kind_bit(self) -> u8 {
        match self {
            Self::Created => 1,
            Self::Changed => 2,
        }
    }
}

/// Every `WatchKind` bit, used when a registration omits an explicit kind.
const ALL_WATCH_KINDS: u8 = 0b111;

/// One registered watcher describing which paths a server wants reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspFileSystemWatcher {
    /// Brace-expanded absolute glob patterns this watcher accepts.
    patterns: Vec<String>,
    /// Bitmask of `WatchKind` values the server subscribed to.
    kind_mask: u8,
}

impl LspFileSystemWatcher {
    /// Build one watcher from a glob pattern and its optional base directory.
    ///
    /// A pattern with a base directory is resolved against it, matching the
    /// `RelativePattern` shape; a bare pattern is matched against whole paths.
    pub(crate) fn new(glob_pattern: &str, base_path: Option<&Path>, kind_mask: Option<u8>) -> Self {
        let anchored = match base_path {
            Some(base) => format!(
                "{}/{glob_pattern}",
                base.to_string_lossy().trim_end_matches('/')
            ),
            None => glob_pattern.to_string(),
        };
        Self {
            patterns: expand_braces(&anchored),
            kind_mask: kind_mask.unwrap_or(ALL_WATCH_KINDS),
        }
    }

    /// Return whether this watcher subscribed to `kind` for `path`.
    ///
    /// Returns `true` when the server asked to be told about this change, and
    /// `false` when either the kind or the path falls outside the registration.
    pub(crate) fn matches(&self, path: &Path, kind: LspFileChangeKind) -> bool {
        if self.kind_mask & kind.watch_kind_bit() == 0 {
            return false;
        }
        let path = path.to_string_lossy();
        let path_segments = path.split('/').collect::<Vec<_>>();
        self.patterns.iter().any(|pattern| {
            let pattern_segments = pattern.split('/').collect::<Vec<_>>();
            match_segments(&pattern_segments, &path_segments)
        })
    }
}

/// Expand `{a,b}` alternations into one concrete pattern per combination.
fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(close) = matching_brace(pattern, open) else {
        return vec![pattern.to_string()];
    };
    let prefix = &pattern[..open];
    let suffix = &pattern[close + 1..];
    let mut expanded = Vec::new();
    for alternative in split_alternatives(&pattern[open + 1..close]) {
        // Each alternative can itself contain braces, so re-expand the whole
        // recombined pattern instead of only the substituted fragment.
        expanded.extend(expand_braces(&format!("{prefix}{alternative}{suffix}")));
    }
    expanded
}

/// Return the index of the `}` closing the brace opened at `open_index`.
fn matching_brace(pattern: &str, open_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in pattern[open_index..].char_indices() {
        let index = open_index + offset;
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split one brace body on the commas that sit at its own nesting level.
fn split_alternatives(body: &str) -> Vec<&str> {
    let mut alternatives = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, character) in body.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                alternatives.push(&body[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    alternatives.push(&body[start..]);
    alternatives
}

/// Match one segmented glob against one segmented path.
///
/// Returns `true` when every pattern segment consumes the path, and `false`
/// when any segment fails or the two run out of step.
fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    let Some((first, rest)) = pattern.split_first() else {
        return path.is_empty();
    };
    if *first == "**" {
        // `**` spans zero or more whole segments, so try every split point.
        return (0..=path.len()).any(|skip| match_segments(rest, &path[skip..]));
    }
    let Some((candidate, remaining)) = path.split_first() else {
        return false;
    };
    match_segment(first.as_bytes(), candidate.as_bytes()) && match_segments(rest, remaining)
}

/// Match one glob segment against one path segment.
///
/// Returns `true` when the whole segment is consumed by the pattern, and
/// `false` when a literal, class, or wildcard cannot cover the text.
fn match_segment(pattern: &[u8], text: &[u8]) -> bool {
    let Some((token, pattern_rest)) = pattern.split_first() else {
        return text.is_empty();
    };
    match token {
        b'*' => {
            // A trailing `*` absorbs the rest of the segment, otherwise every
            // split point is a candidate for the remaining pattern.
            (0..=text.len()).any(|skip| match_segment(pattern_rest, &text[skip..]))
        }
        b'?' => !text.is_empty() && match_segment(pattern_rest, &text[1..]),
        b'[' => match_character_class(pattern, text),
        _ => {
            let (literal, pattern_rest) = match token {
                b'\\' => match pattern_rest.split_first() {
                    Some((escaped, rest)) => (*escaped, rest),
                    None => (b'\\', pattern_rest),
                },
                other => (*other, pattern_rest),
            };
            match text.split_first() {
                Some((candidate, text_rest)) if *candidate == literal => {
                    match_segment(pattern_rest, text_rest)
                }
                _ => false,
            }
        }
    }
}

/// Match one `[...]` character class at the head of `pattern`.
///
/// Returns `true` when the first text byte satisfies the class and the rest of
/// the segment matches, and `false` when the class is unterminated or rejects.
fn match_character_class(pattern: &[u8], text: &[u8]) -> bool {
    let Some(close) = pattern
        .iter()
        .position(|byte| *byte == b']')
        .filter(|index| *index > 1)
    else {
        return false;
    };
    let Some((candidate, text_rest)) = text.split_first() else {
        return false;
    };
    let mut body = &pattern[1..close];
    let negated = matches!(body.first(), Some(b'!' | b'^'));
    if negated {
        body = &body[1..];
    }
    let mut matched = false;
    let mut index = 0;
    while index < body.len() {
        // A `-` between two bytes forms an inclusive range, and anywhere else it
        // is an ordinary member of the class.
        if index + 2 < body.len() && body[index + 1] == b'-' {
            if (body[index]..=body[index + 2]).contains(candidate) {
                matched = true;
            }
            index += 3;
        } else {
            if body[index] == *candidate {
                matched = true;
            }
            index += 1;
        }
    }
    matched != negated && match_segment(&pattern[close + 1..], text_rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Build one watcher subscribed to every change kind.
    fn watcher(pattern: &str) -> LspFileSystemWatcher {
        LspFileSystemWatcher::new(pattern, None, None)
    }

    /// Verify the absolute globs rust-analyzer registers match workspace paths.
    #[test]
    fn test_watcher_matches_rust_analyzer_registered_globs() {
        let source = PathBuf::from("/home/user/project/src/lib.rs");
        let manifest = PathBuf::from("/home/user/project/Cargo.toml");
        let lock = PathBuf::from("/home/user/project/nested/Cargo.lock");
        let outside = PathBuf::from("/home/user/other/src/lib.rs");

        let sources = watcher("/home/user/project/**/*.rs");
        let manifests = watcher("/home/user/project/**/Cargo.{toml,lock}");

        assert!(sources.matches(&source, LspFileChangeKind::Changed));
        assert!(!sources.matches(&manifest, LspFileChangeKind::Changed));
        assert!(!sources.matches(&outside, LspFileChangeKind::Changed));
        assert!(manifests.matches(&manifest, LspFileChangeKind::Changed));
        assert!(manifests.matches(&lock, LspFileChangeKind::Changed));
    }

    /// Verify `**` spans zero segments so root-level files still match.
    #[test]
    fn test_watcher_double_star_spans_zero_segments() {
        assert!(watcher("**/*.rs").matches(&PathBuf::from("main.rs"), LspFileChangeKind::Created));
    }

    /// Verify single-star globs stay inside one path segment.
    #[test]
    fn test_watcher_single_star_stays_within_one_segment() {
        let nested = PathBuf::from("/project/src/core/lib.rs");

        assert!(!watcher("/project/src/*.rs").matches(&nested, LspFileChangeKind::Changed));
        assert!(watcher("/project/src/**/*.rs").matches(&nested, LspFileChangeKind::Changed));
    }

    /// Verify brace alternation expands into independent patterns.
    #[test]
    fn test_watcher_expands_brace_alternation() {
        let pattern = watcher("**/*.{ts,js}");

        assert!(pattern.matches(&PathBuf::from("/app/main.ts"), LspFileChangeKind::Changed));
        assert!(pattern.matches(&PathBuf::from("/app/main.js"), LspFileChangeKind::Changed));
        assert!(!pattern.matches(&PathBuf::from("/app/main.rs"), LspFileChangeKind::Changed));
    }

    /// Verify brace bodies are located by byte offset even after multibyte text.
    #[test]
    fn test_watcher_expands_braces_after_multibyte_segments() {
        let pattern = watcher("/projets/café/**/*.{rs,toml}");

        assert!(pattern.matches(
            &PathBuf::from("/projets/café/src/lib.rs"),
            LspFileChangeKind::Changed
        ));
        assert!(pattern.matches(
            &PathBuf::from("/projets/café/Cargo.toml"),
            LspFileChangeKind::Changed
        ));
    }

    /// Verify character classes accept ranges and negation.
    #[test]
    fn test_watcher_matches_character_classes() {
        assert!(watcher("**/file[0-9].rs").matches(
            &PathBuf::from("/project/file7.rs"),
            LspFileChangeKind::Changed
        ));
        assert!(!watcher("**/file[!0-9].rs").matches(
            &PathBuf::from("/project/file7.rs"),
            LspFileChangeKind::Changed
        ));
    }

    /// Verify relative patterns resolve against their registered base directory.
    #[test]
    fn test_watcher_resolves_relative_pattern_against_base() {
        let base = PathBuf::from("/home/user/project");
        let pattern = LspFileSystemWatcher::new("**/*.rs", Some(&base), None);

        assert!(pattern.matches(
            &PathBuf::from("/home/user/project/src/lib.rs"),
            LspFileChangeKind::Changed
        ));
        assert!(!pattern.matches(
            &PathBuf::from("/home/user/other/src/lib.rs"),
            LspFileChangeKind::Changed
        ));
    }

    /// Verify a watcher only reports the change kinds it subscribed to.
    #[test]
    fn test_watcher_honors_subscribed_change_kinds() {
        let create_only = LspFileSystemWatcher::new("**/*.rs", None, Some(1));
        let source = PathBuf::from("/project/src/lib.rs");

        assert!(create_only.matches(&source, LspFileChangeKind::Created));
        assert!(!create_only.matches(&source, LspFileChangeKind::Changed));
    }
}
