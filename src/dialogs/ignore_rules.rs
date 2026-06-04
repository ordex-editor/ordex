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
    /// The ignore file source this rule came from.
    source: IgnoreRuleSource,
}

/// One ignore-source identifier used for precedence-aware rule evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnoreRuleSource {
    /// Rule loaded from `.gitignore`.
    GitIgnore,
    /// Rule loaded from `.ignore`.
    PickerIgnore,
}

/// One cache-backed matcher for `.gitignore` and `.ignore` files under one scan root.
#[derive(Debug)]
pub(crate) struct IgnoreMatcher {
    root: PathBuf,
    /// Cached parsed rules per absolute directory path.
    rules_by_directory: HashMap<PathBuf, Vec<IgnoreRule>>,
}

/// One filesystem candidate class used by `.ignore` matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathKind {
    /// One regular file candidate.
    File,
    /// One directory candidate.
    Directory,
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
        path_kind: PathKind,
    ) -> io::Result<bool> {
        // Pure `.ignore` checks start from "visible" and let rules decide.
        self.is_ignored_with_baseline(relative_path, path_kind, false)
    }

    /// Return whether `relative_path` remains ignored after applying ignore rules.
    ///
    /// `baseline_ignored` is the ignore state coming from an external source
    /// before file-based rules are applied (for example, Git's ignored-path set).
    /// `.gitignore` and `.ignore` rules are then evaluated on top of that baseline
    /// and may keep the path ignored or un-ignore it through negation.
    ///
    /// Returns `true` when the path is ignored after overlaying file rules on
    /// `baseline_ignored`, and returns `false` when rule evaluation makes it visible.
    pub(crate) fn is_ignored_with_baseline(
        &mut self,
        relative_path: &Path,
        path_kind: PathKind,
        baseline_ignored: bool,
    ) -> io::Result<bool> {
        if relative_path.as_os_str().is_empty() {
            return Ok(baseline_ignored);
        }

        let absolute_path = self.root.join(relative_path);
        let include_gitignore_rules = !baseline_ignored;

        // Evaluate every descendant-side ancestor directory first because ignored
        // ancestors keep descendants excluded unless that ancestor is unignored.
        let mut ancestor_baseline = baseline_ignored;
        let mut ancestor_relative = PathBuf::new();
        for component in relative_path
            .components()
            .take(relative_path.components().count().saturating_sub(1))
        {
            if let Component::Normal(name) = component {
                ancestor_relative.push(name);
                let ancestor_absolute = self.root.join(&ancestor_relative);
                ancestor_baseline = self.match_state_for_path(
                    &ancestor_absolute,
                    PathKind::Directory,
                    ancestor_baseline,
                    include_gitignore_rules,
                )?;
                if ancestor_baseline {
                    return Ok(true);
                }
            }
        }

        self.match_state_for_path(
            &absolute_path,
            path_kind,
            ancestor_baseline,
            include_gitignore_rules,
        )
    }

    /// Evaluate ignore state for one absolute path without ancestor short-circuiting.
    ///
    /// Returns `true` when the most recent matching rule excludes the path, and
    /// returns `false` when no rule excludes it after precedence resolution.
    fn match_state_for_path(
        &mut self,
        absolute_path: &Path,
        path_kind: PathKind,
        baseline_ignored: bool,
        include_gitignore_rules: bool,
    ) -> io::Result<bool> {
        let parent = absolute_path.parent().unwrap_or(Path::new("/"));
        let mut ignored = baseline_ignored;

        // Ignore files are loaded from filesystem root to leaf so later (deeper)
        // files correctly override earlier rules for descendant paths.
        for directory in directories_from_filesystem_root(parent) {
            let rules = self.load_rules_for_directory(&directory)?;
            let path_from_rule_dir = absolute_path
                .strip_prefix(&directory)
                .expect("ancestor directory should prefix candidate path");
            let candidate = normalize_relative_path(path_from_rule_dir);
            if candidate.is_empty() {
                continue;
            }
            for rule in rules {
                if rule.source == IgnoreRuleSource::GitIgnore && !include_gitignore_rules {
                    continue;
                }
                if rule.matches(&candidate, path_kind) {
                    ignored = !rule.negated;
                }
            }
        }

        Ok(ignored)
    }

    /// Load and cache parsed rules from `directory/.gitignore` and `directory/.ignore`.
    fn load_rules_for_directory(&mut self, directory: &Path) -> io::Result<&[IgnoreRule]> {
        if !self.rules_by_directory.contains_key(directory) {
            let mut rules =
                parse_ignore_file(&directory.join(".gitignore"), IgnoreRuleSource::GitIgnore)?;
            let mut picker_rules =
                parse_ignore_file(&directory.join(".ignore"), IgnoreRuleSource::PickerIgnore)?;
            // `.ignore` is loaded after `.gitignore` so `.ignore` negations can
            // re-include paths excluded by `.gitignore`.
            rules.append(&mut picker_rules);
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
    fn matches(&self, candidate_path: &str, path_kind: PathKind) -> bool {
        // Directory-only rules (`foo/`) are rejected early for file candidates.
        if self.dir_only && path_kind == PathKind::File {
            return false;
        }
        // Empty patterns are invalid after marker stripping and never match.
        if self.pattern.is_empty() {
            return false;
        }
        // Anchored single-segment rules (`/build`) only match the path root.
        if self.anchored && !self.has_slash {
            return glob_match(&self.pattern, candidate_path);
        }
        // Slash-bearing patterns are interpreted against full relative paths.
        if self.has_slash {
            return self.matches_slash_pattern(candidate_path);
        }
        // Remaining unanchored segment rules (`target`) match any path component.
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
fn parse_ignore_file(path: &Path, source: IgnoreRuleSource) -> io::Result<Vec<IgnoreRule>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut rules = Vec::new();
    for raw_line in contents.lines() {
        if let Some(rule) = parse_ignore_line(raw_line, source) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

/// Parse one `.ignore` line into an optional rule.
fn parse_ignore_line(raw_line: &str, source: IgnoreRuleSource) -> Option<IgnoreRule> {
    // Whitespace-only lines are ignored before any control-token parsing.
    let trimmed_line = raw_line.trim();
    if trimmed_line.is_empty() {
        return None;
    }

    // Leading escapes are handled before comment/negation detection so `\#` and
    // `\!` are interpreted as literal first characters.
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

    // Negation is recognized only for unescaped leading `!`.
    let mut negated = false;
    if !escaped_leading_bang && let Some(rest) = line.strip_prefix('!') {
        negated = true;
        line = rest.to_string();
    }

    // Directory-only rules are tracked by stripping one trailing slash.
    let mut dir_only = false;
    if line.ends_with('/') {
        dir_only = true;
        line.pop();
    }

    // Anchoring is tracked after control-marker parsing so `/foo` means
    // "from this .ignore directory root" and not "absolute filesystem root".
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
        source,
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

/// Return all absolute directories from filesystem root to `path`, in precedence order.
fn directories_from_filesystem_root(path: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut current = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
            }
            Component::RootDir => {
                current.push(Path::new("/"));
                directories.push(current.clone());
            }
            Component::Normal(name) => {
                current.push(name);
                directories.push(current.clone());
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }

    if directories.is_empty() {
        directories.push(PathBuf::from("/"));
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
/// - `pattern`: pattern bytes being evaluated.
/// - `pattern_index`: current cursor into `pattern`.
/// - `text`: candidate path bytes being evaluated.
/// - `text_index`: current cursor into `text`.
/// - `memo`: cache storing previously evaluated `(pattern_index, text_index)` states.
/// - `stride`: row width used to map `(pattern_index, text_index)` into `memo`.
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
                    // `?` consumes exactly one non-separator byte, then checks
                    // whether the remaining suffixes continue to match.
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
                    // Literal matches advance both cursors by one byte and then
                    // recursively verify the remaining suffixes.
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
/// - `pattern`: full pattern bytes.
/// - `pattern_index`: index of the escape token in `pattern`.
/// - `text`: full candidate text bytes.
/// - `text_index`: current candidate index being matched.
/// - `memo`: dynamic-programming cache shared across recursive calls.
/// - `stride`: row width used for memo indexing.
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
        // After consuming the escaped literal, recurse on the remaining suffix.
        && glob_match_inner(pattern, next_index, text, text_index + 1, memo, stride)
}

/// Match one `*` or `**` token at `pattern_index`.
///
/// - `pattern`: full pattern bytes.
/// - `pattern_index`: index of the wildcard token in `pattern`.
/// - `text`: full candidate text bytes.
/// - `text_index`: current candidate index being matched.
/// - `memo`: dynamic-programming cache shared across recursive calls.
/// - `stride`: row width used for memo indexing.
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
    // Zero-width expansion checks whether wildcard can match nothing.
    if glob_match_inner(pattern, next_index, text, text_index, memo, stride) {
        return true;
    }
    let mut cursor = text_index;
    while cursor < text.len() {
        if !is_double_star && text[cursor] == b'/' {
            break;
        }
        cursor += 1;
        // Each recursive call tests one longer wildcard expansion.
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
            .is_ignored(Path::new("ignored.log"), PathKind::File)
            .expect("evaluate ignored path");
        let visible = matcher
            .is_ignored(Path::new("notes.txt"), PathKind::File)
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
            .is_ignored(Path::new("src/tmp"), PathKind::Directory)
            .expect("evaluate nested directory");
        let tests_tmp = matcher
            .is_ignored(Path::new("tests/tmp"), PathKind::Directory)
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
            .is_ignored(Path::new("drop.log"), PathKind::File)
            .expect("evaluate excluded file");
        let kept = matcher
            .is_ignored(Path::new("keep.log"), PathKind::File)
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
            .is_ignored(Path::new("build"), PathKind::Directory)
            .expect("evaluate rooted directory");
        let nested_build = matcher
            .is_ignored(Path::new("src/build"), PathKind::Directory)
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
            .is_ignored(Path::new("build/keep.txt"), PathKind::File)
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
            .is_ignored(Path::new("build/keep.txt"), PathKind::File)
            .expect("evaluate child file");

        assert!(!kept);
    }

    #[test]
    /// Double-star globs should match across any number of nested segments.
    fn test_double_star_matches_nested_segments() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "src/**/generated.rs\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let nested = matcher
            .is_ignored(Path::new("src/core/generated.rs"), PathKind::File)
            .expect("evaluate nested file");
        let deeper = matcher
            .is_ignored(Path::new("src/core/ui/generated.rs"), PathKind::File)
            .expect("evaluate deeper file");

        assert!(nested);
        assert!(deeper);
    }

    #[test]
    /// Un-ignoring a directory should clear ignored baseline for its descendants.
    fn test_unignored_ancestor_clears_baseline_for_descendants() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "!/old\n")
            .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        let old_directory = matcher
            .is_ignored_with_baseline(Path::new("old"), PathKind::Directory, true)
            .expect("evaluate unignored directory");
        let descendant = matcher
            .is_ignored_with_baseline(Path::new("old/plan.md"), PathKind::File, true)
            .expect("evaluate descendant file");

        assert!(!old_directory);
        assert!(!descendant);
    }

    #[test]
    /// Parent `.ignore` rules should apply even when scanning from a nested directory.
    fn test_parent_ignore_file_applies_above_scan_root() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "parent-hidden.txt\n")
            .expect("write parent ignore file");
        tree.write_file("nested/project/parent-hidden.txt", "hidden\n")
            .expect("write hidden fixture");
        tree.write_file("nested/project/visible.txt", "visible\n")
            .expect("write visible fixture");

        let mut matcher = IgnoreMatcher::new(tree.path().join("nested/project"));
        let hidden = matcher
            .is_ignored(Path::new("parent-hidden.txt"), PathKind::File)
            .expect("evaluate hidden path");
        let visible = matcher
            .is_ignored(Path::new("visible.txt"), PathKind::File)
            .expect("evaluate visible path");

        assert!(hidden);
        assert!(!visible);
    }

    #[test]
    /// Parent `.gitignore` rules should apply even when scanning from a nested directory.
    fn test_parent_gitignore_file_applies_above_scan_root() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "parent-git-hidden.txt\n")
            .expect("write parent gitignore file");
        tree.write_file("nested/project/parent-git-hidden.txt", "hidden\n")
            .expect("write hidden fixture");
        tree.write_file("nested/project/visible.txt", "visible\n")
            .expect("write visible fixture");

        let mut matcher = IgnoreMatcher::new(tree.path().join("nested/project"));
        let hidden = matcher
            .is_ignored(Path::new("parent-git-hidden.txt"), PathKind::File)
            .expect("evaluate hidden path");
        let visible = matcher
            .is_ignored(Path::new("visible.txt"), PathKind::File)
            .expect("evaluate visible path");

        assert!(hidden);
        assert!(!visible);
    }

    #[test]
    /// Escaped leading markers should be matched as literal text.
    fn test_escaped_markers_are_not_treated_as_control_tokens() {
        let line = parse_ignore_line("\\!literal", IgnoreRuleSource::PickerIgnore)
            .expect("parse escaped negation");
        let comment = parse_ignore_line("\\#literal", IgnoreRuleSource::PickerIgnore)
            .expect("parse escaped comment");

        assert_eq!(line.pattern, "!literal");
        assert!(!line.negated);
        assert_eq!(comment.pattern, "#literal");
        assert!(!comment.negated);
    }
}
