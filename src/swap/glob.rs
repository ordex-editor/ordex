//! Swap exclusion glob matching.

/// Return whether `path` matches `pattern`, where `*` matches any bytes.
pub(crate) fn matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.as_bytes();
    let path = path.as_bytes();
    let mut pattern_idx = 0;
    let mut path_idx = 0;
    let mut last_star = None;
    let mut backtrack_path_idx = 0;

    // This standard wildcard loop keeps matching anchored to the full path
    // while allowing `*` to expand across any characters, including `/`.
    while path_idx < path.len() {
        if pattern_idx < pattern.len() && pattern[pattern_idx] == b'*' {
            last_star = Some(pattern_idx);
            pattern_idx += 1;
            backtrack_path_idx = path_idx;
            continue;
        }
        if pattern_idx < pattern.len() && pattern[pattern_idx] == path[path_idx] {
            pattern_idx += 1;
            path_idx += 1;
            continue;
        }
        let Some(star_idx) = last_star else {
            return false;
        };
        pattern_idx = star_idx + 1;
        backtrack_path_idx += 1;
        path_idx = backtrack_path_idx;
    }

    while pattern_idx < pattern.len() && pattern[pattern_idx] == b'*' {
        pattern_idx += 1;
    }
    pattern_idx == pattern.len()
}

/// Return whether `path` matches any pattern in `patterns`.
pub(crate) fn matches_any(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|pattern| matches(pattern, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_suffix_pattern() {
        assert!(matches("*.gpg", "/tmp/secret.gpg"));
        assert!(!matches("*.gpg", "/tmp/secret.gpg.bak"));
    }

    #[test]
    fn matches_prefix_across_path_separators() {
        assert!(matches("/dev/shm/gopass*", "/dev/shm/gopass_edit123/file"));
        assert!(!matches("/dev/shm/gopass*", "/dev/shm/other"));
    }

    #[test]
    fn matches_exact_literal_without_wildcards() {
        assert!(matches("/tmp/notes.txt", "/tmp/notes.txt"));
        assert!(!matches("/tmp/notes.txt", "/tmp/notes.txt.bak"));
    }

    #[test]
    fn accepts_empty_star_matches() {
        assert!(matches("prefix*suffix", "prefixsuffix"));
    }

    #[test]
    fn checks_multiple_patterns() {
        let patterns = vec!["*.gpg".to_string(), "/dev/shm/gopass*".to_string()];
        assert!(matches_any(&patterns, "/tmp/secret.gpg"));
        assert!(!matches_any(&patterns, "/tmp/notes.txt"));
    }
}
