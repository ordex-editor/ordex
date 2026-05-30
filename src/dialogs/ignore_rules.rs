//! `.ignore` rule loading and path matching for picker-style scans.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// One parsed `.ignore` rule scoped to the directory that defined it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IgnoreRule {
    /// Pattern text after stripping prefix/suffix rule markers.
    pattern: String,
    /// Whether this rule is a negation (`!pattern`).
    negated: bool,
    /// Whether this rule only targets directories (`pattern/`).
    dir_only: bool,
    /// Whether the rule is anchored to the defining directory (`/pattern`).
    anchored: bool,
    /// Whether the rule includes at least one `/` segment separator.
    has_slash: bool,
}

/// One cache-backed matcher for `.ignore` files under one scan root.
#[derive(Debug)]
pub(crate) struct IgnoreMatcher {
    root: PathBuf,
    /// Cached parsed rules per directory relative to `root`.
    rules_by_directory: HashMap<PathBuf, Vec<IgnoreRule>>,
}

impl IgnoreMatcher {
    /// Build one matcher rooted at `root`.
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            rules_by_directory: HashMap::new(),
        }
    }

    /// Return whether `relative_path` should be ignored by loaded `.ignore` files.
    ///
    /// Returns `true` when the path is excluded by rule evaluation, and returns
    /// `false` when no exclusion applies or a later negation restores visibility.
    pub(crate) fn is_ignored(
        &mut self,
        relative_path: &Path,
        is_directory: bool,
    ) -> io::Result<bool> {
        if relative_path.as_os_str().is_empty() {
            return Ok(false);
        }

        // Evaluate every ancestor directory first because ignored ancestors keep
        // descendants excluded unless the ancestor itself is explicitly unignored.
        let mut ancestor = PathBuf::new();
        for component in relative_path
            .components()
            .take(relative_path.components().count().saturating_sub(1))
        {
            if let Component::Normal(name) = component {
                ancestor.push(name);
                if self.match_state_for_path(&ancestor, true)? {
                    return Ok(true);
                }
            }
        }

        self.match_state_for_path(relative_path, is_directory)
    }

    /// Evaluate ignore state for one path without consulting ancestor short-circuiting.
    ///
    /// Returns `true` when the most recent matching rule excludes the path, and
    /// returns `false` when no rule excludes it after precedence resolution.
    fn match_state_for_path(
        &mut self,
        relative_path: &Path,
        is_directory: bool,
    ) -> io::Result<bool> {
        let parent = relative_path.parent().unwrap_or(Path::new(""));
        let mut ignored = false;

        // `.ignore` files are loaded from root to leaf so later (deeper) files
        // correctly override earlier rules for descendant paths.
        for directory in directories_from_root(parent) {
            let rules = self.load_rules_for_directory(&directory)?;
            let path_from_rule_dir = relative_path
                .strip_prefix(&directory)
                .expect("ancestor directory should prefix candidate path");
            let candidate = normalize_relative_path(path_from_rule_dir);
            if candidate.is_empty() {
                continue;
            }
            for rule in rules {
                if rule.matches(&candidate, is_directory) {
                    ignored = !rule.negated;
                }
            }
        }

        Ok(ignored)
    }

    /// Load and cache parsed rules from `directory/.ignore` under `root`.
    fn load_rules_for_directory(&mut self, directory: &Path) -> io::Result<&[IgnoreRule]> {
        if !self.rules_by_directory.contains_key(directory) {
            let ignore_path = self.root.join(directory).join(".ignore");
            let rules = parse_ignore_file(&ignore_path)?;
            self.rules_by_directory
                .insert(directory.to_path_buf(), rules);
        }
        Ok(self
            .rules_by_directory
            .get(directory)
            .expect("rules cache entry should exist")
            .as_slice())
    }
}

impl IgnoreRule {
    /// Return whether this rule matches `candidate_path`.
    ///
    /// Returns `true` when the rule applies to the candidate path, and returns
    /// `false` when the rule does not apply.
    fn matches(&self, candidate_path: &str, is_directory: bool) -> bool {
        if self.dir_only && !is_directory {
            return false;
        }
        if self.pattern.is_empty() {
            return false;
        }
        if self.anchored && !self.has_slash {
            return glob_match(&self.pattern, candidate_path);
        }
        if self.has_slash {
            return self.matches_slash_pattern(candidate_path);
        }
        candidate_path
            .split('/')
            .any(|component| glob_match(&self.pattern, component))
    }

    /// Match one slash-containing rule against `candidate_path`.
    ///
    /// Returns `true` when the slash rule applies, and returns `false` otherwise.
    fn matches_slash_pattern(&self, candidate_path: &str) -> bool {
        if self.anchored {
            return glob_match(&self.pattern, candidate_path);
        }
        // Unanchored slash rules behave like `**/pattern`: test every suffix.
        for suffix in path_suffixes(candidate_path) {
            if glob_match(&self.pattern, suffix) {
                return true;
            }
        }
        false
    }
}

/// Parse one `.ignore` file into ordered rules.
fn parse_ignore_file(path: &Path) -> io::Result<Vec<IgnoreRule>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut rules = Vec::new();
    for raw_line in contents.lines() {
        if let Some(rule) = parse_ignore_line(raw_line) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

/// Parse one `.ignore` line into an optional rule.
fn parse_ignore_line(raw_line: &str) -> Option<IgnoreRule> {
    let trimmed_line = raw_line.trim();
    if trimmed_line.is_empty() {
        return None;
    }

    let mut line = trimmed_line.to_string();
    let mut escaped_leading_bang = false;
    let mut escaped_leading_hash = false;
    if line.starts_with("\\!") {
        escaped_leading_bang = true;
        line = line[1..].to_string();
    } else if line.starts_with("\\#") {
        escaped_leading_hash = true;
        line = line[1..].to_string();
    }
    if !escaped_leading_hash && line.starts_with('#') {
        return None;
    }

    let mut negated = false;
    if !escaped_leading_bang && let Some(rest) = line.strip_prefix('!') {
        negated = true;
        line = rest.to_string();
    }

    let mut dir_only = false;
    if line.ends_with('/') {
        dir_only = true;
        line.pop();
    }

    let mut anchored = false;
    if let Some(rest) = line.strip_prefix('/') {
        anchored = true;
        line = rest.to_string();
    }

    if line.is_empty() {
        return None;
    }
    let has_slash = line.contains('/');
    Some(IgnoreRule {
        pattern: line,
        negated,
        dir_only,
        anchored,
        has_slash,
    })
}

/// Return one normalized relative path using `/` separators.
fn normalize_relative_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            parts.push(name.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

/// Return all suffixes of one slash-separated path, longest to shortest.
fn path_suffixes(path: &str) -> Vec<&str> {
    let mut suffixes = vec![path];
    for (index, byte) in path.as_bytes().iter().enumerate() {
        if *byte == b'/' && index + 1 < path.len() {
            suffixes.push(&path[index + 1..]);
        }
    }
    suffixes
}

/// Return all directories from root (`""`) to `path`, in precedence order.
fn directories_from_root(path: &Path) -> Vec<PathBuf> {
    let mut directories = vec![PathBuf::new()];
    let mut current = PathBuf::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            directories.push(current.clone());
        }
    }
    directories
}

/// Match one gitignore-style glob against `text`.
///
/// Returns `true` when the pattern matches the full text, and returns `false`
/// when at least one required token does not match.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_bytes = pattern.as_bytes();
    let text_bytes = text.as_bytes();
    let mut memo = vec![None; (pattern_bytes.len() + 1) * (text_bytes.len() + 1)];
    glob_match_inner(
        pattern_bytes,
        0,
        text_bytes,
        0,
        &mut memo,
        text_bytes.len() + 1,
    )
}

/// Run memoized wildcard matching with gitignore-style `*`, `?`, and `**`.
///
/// Returns `true` when the suffixes from `pattern_index` and `text_index`
/// match, and returns `false` otherwise.
fn glob_match_inner(
    pattern: &[u8],
    pattern_index: usize,
    text: &[u8],
    text_index: usize,
    memo: &mut [Option<bool>],
    stride: usize,
) -> bool {
    let memo_index = pattern_index * stride + text_index;
    if let Some(cached) = memo[memo_index] {
        return cached;
    }

    let result = if pattern_index == pattern.len() {
        text_index == text.len()
    } else {
        // Escape sequences treat the next token literally so wildcard markers
        // may be matched as plain characters when prefixed by `\`.
        match pattern[pattern_index] {
            b'\\' => match_escaped(pattern, pattern_index, text, text_index, memo, stride),
            b'*' => match_star(pattern, pattern_index, text, text_index, memo, stride),
            b'?' => {
                text.get(text_index).is_some_and(|byte| *byte != b'/')
                    && glob_match_inner(
                        pattern,
                        pattern_index + 1,
                        text,
                        text_index + 1,
                        memo,
                        stride,
                    )
            }
            literal => {
                text.get(text_index).copied() == Some(literal)
                    && glob_match_inner(
                        pattern,
                        pattern_index + 1,
                        text,
                        text_index + 1,
                        memo,
                        stride,
                    )
            }
        }
    };

    memo[memo_index] = Some(result);
    result
}

/// Match one escaped token where `pattern[pattern_index] == '\\'`.
///
/// Returns `true` when the escaped literal matches at `text_index`, and returns
/// `false` when it does not.
fn match_escaped(
    pattern: &[u8],
    pattern_index: usize,
    text: &[u8],
    text_index: usize,
    memo: &mut [Option<bool>],
    stride: usize,
) -> bool {
    let literal = pattern.get(pattern_index + 1).copied().unwrap_or(b'\\');
    let next_index = if pattern_index + 1 < pattern.len() {
        pattern_index + 2
    } else {
        pattern_index + 1
    };
    text.get(text_index).copied() == Some(literal)
        && glob_match_inner(pattern, next_index, text, text_index + 1, memo, stride)
}

/// Match one `*` or `**` token at `pattern_index`.
///
/// Returns `true` when wildcard expansion can satisfy the remaining pattern,
/// and returns `false` when no expansion leads to a full match.
fn match_star(
    pattern: &[u8],
    pattern_index: usize,
    text: &[u8],
    text_index: usize,
    memo: &mut [Option<bool>],
    stride: usize,
) -> bool {
    let is_double_star = pattern.get(pattern_index + 1).copied() == Some(b'*');
    let next_index = if is_double_star {
        pattern_index + 2
    } else {
        pattern_index + 1
    };

    // First attempt consumes zero bytes, then progressively grows the wildcard.
    if glob_match_inner(pattern, next_index, text, text_index, memo, stride) {
        return true;
    }
    let mut cursor = text_index;
    while cursor < text.len() {
        if !is_double_star && text[cursor] == b'/' {
            break;
        }
        cursor += 1;
        if glob_match_inner(pattern, next_index, text, cursor, memo, stride) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::TempTree;

    #[test]
    /// Root rules should hide matching files and keep unrelated files visible.
    fn test_root_ignore_rule_excludes_matching_file() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "ignored.log\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let ignored = matcher
            .is_ignored(Path::new("ignored.log"), false)
            .expect("evaluate ignored path");
        let visible = matcher
            .is_ignored(Path::new("notes.txt"), false)
            .expect("evaluate visible path");

        assert!(ignored);
        assert!(!visible);
    }

    #[test]
    /// Nested `.ignore` files should apply only within their own subtree.
    fn test_nested_ignore_file_is_scoped_to_its_directory() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("src/.ignore", "tmp/\n")
            .expect("write nested ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let src_tmp = matcher
            .is_ignored(Path::new("src/tmp"), true)
            .expect("evaluate nested directory");
        let tests_tmp = matcher
            .is_ignored(Path::new("tests/tmp"), true)
            .expect("evaluate sibling directory");

        assert!(src_tmp);
        assert!(!tests_tmp);
    }

    #[test]
    /// A later negation should restore visibility for specific matching paths.
    fn test_negation_rule_reincludes_explicit_file() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "*.log\n!keep.log\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let dropped = matcher
            .is_ignored(Path::new("drop.log"), false)
            .expect("evaluate excluded file");
        let kept = matcher
            .is_ignored(Path::new("keep.log"), false)
            .expect("evaluate negated file");

        assert!(dropped);
        assert!(!kept);
    }

    #[test]
    /// Anchored rules should only match from the defining directory root.
    fn test_anchored_rule_matches_only_directory_root() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "/build/\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let root_build = matcher
            .is_ignored(Path::new("build"), true)
            .expect("evaluate rooted directory");
        let nested_build = matcher
            .is_ignored(Path::new("src/build"), true)
            .expect("evaluate nested directory");

        assert!(root_build);
        assert!(!nested_build);
    }

    #[test]
    /// Re-including only a child file should fail when its parent directory stays ignored.
    fn test_reinclude_file_fails_when_parent_directory_stays_ignored() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "build/\n!build/keep.txt\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let kept = matcher
            .is_ignored(Path::new("build/keep.txt"), false)
            .expect("evaluate child file");

        assert!(kept);
    }

    #[test]
    /// Re-including a parent directory should permit descendant overrides.
    fn test_reinclude_file_succeeds_after_parent_directory_reincluded() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "build/\n!build/\n!build/keep.txt\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let kept = matcher
            .is_ignored(Path::new("build/keep.txt"), false)
            .expect("evaluate child file");

        assert!(!kept);
    }

    #[test]
    /// Escaped leading markers should be matched as literal text.
    fn test_escaped_markers_are_not_treated_as_control_tokens() {
        let line = parse_ignore_line("\\!literal").expect("parse escaped negation");
        let comment = parse_ignore_line("\\#literal").expect("parse escaped comment");

        assert_eq!(line.pattern, "!literal");
        assert!(!line.negated);
        assert_eq!(comment.pattern, "#literal");
        assert!(!comment.negated);
    }
}
