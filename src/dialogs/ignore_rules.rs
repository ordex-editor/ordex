//! `.ignore` rule loading and path matching for picker-style scans.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

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
    /// Compiled matcher shape used by hot-path rule evaluation.
    compiled_match: CompiledIgnoreMatch,
    /// The ignore file source this rule came from.
    source: IgnoreRuleSource,
}

/// One compiled matching mode for one ignore rule pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompiledIgnoreMatch {
    /// Compare full candidate path bytes directly against literal pattern text.
    WholeLiteral,
    /// Compare basename token directly against literal pattern text.
    BasenameLiteral,
    /// Compare full candidate or any `/`-separated suffix against literal text.
    SuffixLiteral,
    /// Evaluate full candidate path with wildcard-aware glob matching.
    WholeGlob,
    /// Evaluate basename token with wildcard-aware glob matching.
    BasenameGlob,
    /// Evaluate full candidate and each `/`-separated suffix with glob matching.
    SuffixGlob,
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
    /// Normalized root path used to rebuild absolute-like candidates efficiently.
    root_normalized: String,
    /// Component offsets for `root_normalized`.
    root_component_offsets: Vec<usize>,
    /// Number of `Component::Normal` segments in `root`.
    root_normal_depth: usize,
    /// Optional highest directory where ignore files are considered.
    ///
    /// Git scans set this to the detected worktree root so ignore files from
    /// unrelated parent directories cannot hide picker results inside the
    /// current repository.
    rules_ceiling: Option<Arc<PathBuf>>,
    /// Cached parsed rules per absolute directory path.
    rules_by_directory: HashMap<PathBuf, Vec<IgnoreRule>>,
    /// Cached directory ignore outcomes keyed by absolute path.
    ///
    /// Index `0` stores the result for `baseline_ignored = false` and index `1`
    /// stores the result for `baseline_ignored = true`.
    directory_match_cache: HashMap<PathBuf, [Option<bool>; 2]>,
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
        let (root_normalized, root_component_offsets) = normalize_relative_path_with_offsets(&root);
        let root_normal_depth = root_component_offsets.len();
        Self {
            root,
            root_normalized,
            root_component_offsets,
            root_normal_depth,
            rules_ceiling: None,
            rules_by_directory: HashMap::new(),
            directory_match_cache: HashMap::new(),
        }
    }

    /// Set one optional ceiling directory for ignore-file discovery.
    ///
    /// When set, ignore files are only loaded from this directory and its
    /// descendants. When unset, ignore files are loaded from filesystem root.
    pub(crate) fn set_rules_ceiling(&mut self, ceiling: Option<PathBuf>) {
        self.rules_ceiling = ceiling.map(Arc::new);
        self.rules_by_directory.clear();
        self.directory_match_cache.clear();
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
    /// before file-based rules are applied.
    ///
    /// - `true` means the path starts in an ignored state (for example, the path
    ///   came from `git ls-files --ignored`).
    /// - `false` means the path starts visible and only matching ignore rules can
    ///   exclude it.
    ///
    /// `.gitignore` and `.ignore` rules are then evaluated on top of that
    /// baseline and may keep the path ignored or un-ignore it through negation.
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
        let (normalized_relative, relative_offsets) =
            normalize_relative_path_with_offsets(relative_path);
        let (normalized_candidate, component_offsets) =
            self.absolute_candidate_from_relative(&normalized_relative, &relative_offsets);

        // Evaluate every descendant-side ancestor directory first because ignored
        // ancestors keep descendants excluded unless that ancestor is unignored.
        let mut ancestor_baseline = baseline_ignored;
        let mut ancestor_relative = PathBuf::new();
        let mut ancestor_component_count = 0usize;
        for component in relative_path
            .components()
            .take(relative_path.components().count().saturating_sub(1))
        {
            if let Component::Normal(name) = component {
                ancestor_relative.push(name);
                ancestor_component_count += 1;
                let ancestor_absolute = self.root.join(&ancestor_relative);
                let ancestor_depth = self
                    .root_normal_depth
                    .saturating_add(ancestor_component_count);
                let ancestor_candidate = candidate_prefix_for_depth(
                    &normalized_candidate,
                    &component_offsets,
                    ancestor_depth,
                );
                ancestor_baseline = self.cached_directory_match_state(
                    &ancestor_absolute,
                    ancestor_baseline,
                    Some((ancestor_candidate, &component_offsets[..ancestor_depth])),
                )?;
                if ancestor_baseline {
                    return Ok(true);
                }
            }
        }

        if path_kind == PathKind::Directory {
            return self.cached_directory_match_state(
                &absolute_path,
                ancestor_baseline,
                Some((&normalized_candidate, &component_offsets)),
            );
        }
        self.match_state_for_path(
            &absolute_path,
            path_kind,
            ancestor_baseline,
            &normalized_candidate,
            &component_offsets,
        )
    }

    /// Return cached directory ignore state for `absolute_directory`.
    ///
    /// Returns `true` when directory matching excludes the path after rule
    /// evaluation, and returns `false` when the directory remains visible.
    fn cached_directory_match_state(
        &mut self,
        absolute_directory: &Path,
        baseline_ignored: bool,
        candidate: Option<(&str, &[usize])>,
    ) -> io::Result<bool> {
        let baseline_index = usize::from(baseline_ignored);
        if let Some(states) = self.directory_match_cache.get(absolute_directory)
            && let Some(cached) = states[baseline_index]
        {
            return Ok(cached);
        }
        // Cache misses resolve through the same directory-rule pipeline as file
        // checks so future siblings can reuse one stable outcome.
        let matched = match candidate {
            Some((normalized_candidate, component_offsets)) => self.match_state_for_path(
                absolute_directory,
                PathKind::Directory,
                baseline_ignored,
                normalized_candidate,
                component_offsets,
            )?,
            None => {
                let relative_directory = absolute_directory
                    .strip_prefix(&self.root)
                    .unwrap_or(Path::new(""));
                let (normalized_relative, relative_offsets) =
                    normalize_relative_path_with_offsets(relative_directory);
                let (normalized_candidate, component_offsets) =
                    self.absolute_candidate_from_relative(&normalized_relative, &relative_offsets);
                self.match_state_for_path(
                    absolute_directory,
                    PathKind::Directory,
                    baseline_ignored,
                    &normalized_candidate,
                    &component_offsets,
                )?
            }
        };
        let entry = self
            .directory_match_cache
            .entry(absolute_directory.to_path_buf())
            .or_insert([None, None]);
        entry[baseline_index] = Some(matched);
        Ok(matched)
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
        normalized_candidate: &str,
        component_offsets: &[usize],
    ) -> io::Result<bool> {
        let parent = absolute_path.parent().unwrap_or(Path::new("/"));
        let mut ignored = baseline_ignored;

        // Ignore files are loaded from the effective root to leaf so later
        // (deeper) files correctly override earlier rules for descendant paths.
        self.evaluate_directories_for_parent(
            parent,
            normalized_candidate,
            component_offsets,
            path_kind,
            &mut ignored,
        )?;

        Ok(ignored)
    }

    /// Evaluate ignore rules from effective root to `parent` with stable precedence.
    fn evaluate_directories_for_parent(
        &mut self,
        parent: &Path,
        normalized_candidate: &str,
        component_offsets: &[usize],
        path_kind: PathKind,
        ignored: &mut bool,
    ) -> io::Result<()> {
        if let Some(ceiling) = self.rules_ceiling.as_ref().map(Arc::clone) {
            return self.evaluate_directories_from_ceiling(
                &ceiling,
                parent,
                normalized_candidate,
                component_offsets,
                path_kind,
                ignored,
            );
        }
        self.evaluate_directories_from_filesystem_root(
            parent,
            normalized_candidate,
            component_offsets,
            path_kind,
            ignored,
        )
    }

    /// Build one absolute-like normalized candidate from a relative normalized path.
    ///
    /// This keeps directory-depth alignment with absolute rule directories while
    /// avoiding repeated normalization of immutable root components per candidate.
    fn absolute_candidate_from_relative(
        &self,
        normalized_relative: &str,
        relative_offsets: &[usize],
    ) -> (String, Vec<usize>) {
        let mut normalized = String::with_capacity(
            self.root_normalized
                .len()
                .saturating_add(1)
                .saturating_add(normalized_relative.len()),
        );
        let mut offsets = Vec::with_capacity(
            self.root_normal_depth
                .saturating_add(relative_offsets.len()),
        );
        if !self.root_normalized.is_empty() {
            normalized.push_str(&self.root_normalized);
            offsets.extend_from_slice(&self.root_component_offsets);
        }
        if normalized_relative.is_empty() {
            return (normalized, offsets);
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        // Relative offsets are translated by the already-emitted root prefix.
        let relative_base = normalized.len();
        normalized.push_str(normalized_relative);
        for offset in relative_offsets {
            offsets.push(relative_base + *offset);
        }
        (normalized, offsets)
    }

    /// Evaluate ignore rules from one configured ceiling directory to `parent`.
    fn evaluate_directories_from_ceiling(
        &mut self,
        ceiling: &Path,
        parent: &Path,
        normalized_candidate: &str,
        component_offsets: &[usize],
        path_kind: PathKind,
        ignored: &mut bool,
    ) -> io::Result<()> {
        if !parent.starts_with(ceiling) {
            return self.apply_rules_for_directory(
                ceiling,
                normal_component_count(ceiling),
                normalized_candidate,
                component_offsets,
                path_kind,
                ignored,
            );
        }

        let mut current = ceiling.to_path_buf();
        let mut depth = normal_component_count(ceiling);
        self.apply_rules_for_directory(
            &current,
            depth,
            normalized_candidate,
            component_offsets,
            path_kind,
            ignored,
        )?;
        let relative_parent = parent
            .strip_prefix(ceiling)
            .expect("parent should start with ceiling");
        for component in relative_parent.components() {
            if let Component::Normal(name) = component {
                // Rule precedence remains root-to-leaf inside the ceiling subtree.
                current.push(name);
                depth += 1;
                self.apply_rules_for_directory(
                    &current,
                    depth,
                    normalized_candidate,
                    component_offsets,
                    path_kind,
                    ignored,
                )?;
            }
        }
        Ok(())
    }

    /// Evaluate ignore rules from filesystem root to `path`.
    fn evaluate_directories_from_filesystem_root(
        &mut self,
        path: &Path,
        normalized_candidate: &str,
        component_offsets: &[usize],
        path_kind: PathKind,
        ignored: &mut bool,
    ) -> io::Result<()> {
        let mut current = PathBuf::new();
        let mut depth = 0;
        let mut visited_directory = false;
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => {
                    // Preserve platform prefixes before the root component.
                    current.push(prefix.as_os_str());
                }
                Component::RootDir => {
                    // Start the precedence chain at filesystem root.
                    current.push(Path::new("/"));
                    visited_directory = true;
                    self.apply_rules_for_directory(
                        &current,
                        depth,
                        normalized_candidate,
                        component_offsets,
                        path_kind,
                        ignored,
                    )?;
                }
                Component::Normal(name) => {
                    // Append each descendant directory so callers can apply rules
                    // from root to leaf in deterministic precedence order.
                    current.push(name);
                    depth += 1;
                    visited_directory = true;
                    self.apply_rules_for_directory(
                        &current,
                        depth,
                        normalized_candidate,
                        component_offsets,
                        path_kind,
                        ignored,
                    )?;
                }
                Component::CurDir | Component::ParentDir => {
                    // Ignore non-canonical traversal markers for rule lookup.
                }
            }
        }
        if visited_directory {
            return Ok(());
        }
        // Paths without components still need root-level rule evaluation.
        self.apply_rules_for_directory(
            Path::new("/"),
            0,
            normalized_candidate,
            component_offsets,
            path_kind,
            ignored,
        )
    }

    /// Apply one directory's rule list to the current candidate path suffix.
    fn apply_rules_for_directory(
        &mut self,
        directory: &Path,
        directory_depth: usize,
        normalized_candidate: &str,
        component_offsets: &[usize],
        path_kind: PathKind,
        ignored: &mut bool,
    ) -> io::Result<()> {
        let rules = self.load_rules_for_directory(directory)?;
        let candidate = candidate_suffix_for_directory(
            normalized_candidate,
            component_offsets,
            directory_depth,
        );
        if candidate.is_empty() {
            return Ok(());
        }
        for rule in rules {
            if rule.matches(candidate, path_kind) {
                *ignored = !rule.negated;
            }
        }
        Ok(())
    }

    /// Load and cache parsed rules from `directory/.gitignore` and `directory/.ignore`.
    fn load_rules_for_directory(&mut self, directory: &Path) -> io::Result<&[IgnoreRule]> {
        use std::collections::hash_map::Entry;

        match self.rules_by_directory.entry(directory.to_path_buf()) {
            Entry::Occupied(entry) => Ok(entry.into_mut().as_slice()),
            Entry::Vacant(entry) => {
                let mut rules =
                    parse_ignore_file(&directory.join(".gitignore"), IgnoreRuleSource::GitIgnore)?;
                let mut picker_rules =
                    parse_ignore_file(&directory.join(".ignore"), IgnoreRuleSource::PickerIgnore)?;
                // `.ignore` is loaded after `.gitignore` so `.ignore` negations can
                // re-include paths excluded by `.gitignore`.
                rules.append(&mut picker_rules);
                Ok(entry.insert(rules).as_slice())
            }
        }
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
        // Matching mode is compiled at parse time so hot-path checks can avoid
        // re-evaluating anchoring/slash semantics for every candidate.
        match self.compiled_match {
            CompiledIgnoreMatch::WholeLiteral => self.pattern == candidate_path,
            CompiledIgnoreMatch::BasenameLiteral => candidate_path
                .rsplit('/')
                .next()
                .is_some_and(|component| component == self.pattern),
            CompiledIgnoreMatch::SuffixLiteral => {
                self.pattern == candidate_path
                    || candidate_path
                        .strip_suffix(&self.pattern)
                        .is_some_and(|prefix| prefix.ends_with('/'))
            }
            CompiledIgnoreMatch::WholeGlob => glob_match(&self.pattern, candidate_path),
            CompiledIgnoreMatch::BasenameGlob => candidate_path
                .rsplit('/')
                .next()
                .is_some_and(|component| glob_match(&self.pattern, component)),
            CompiledIgnoreMatch::SuffixGlob => self.matches_slash_pattern(candidate_path),
        }
    }

    /// Match one slash-containing rule against `candidate_path`.
    ///
    /// Returns `true` when the slash rule applies, and returns `false` otherwise.
    fn matches_slash_pattern(&self, candidate_path: &str) -> bool {
        if self.anchored {
            return glob_match(&self.pattern, candidate_path);
        }
        // Unanchored slash rules behave like `**/pattern`: test every suffix.
        if glob_match(&self.pattern, candidate_path) {
            return true;
        }
        for (index, byte) in candidate_path.as_bytes().iter().enumerate() {
            if *byte == b'/' && index + 1 < candidate_path.len() {
                let suffix = &candidate_path[index + 1..];
                if glob_match(&self.pattern, suffix) {
                    return true;
                }
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
    let has_glob_meta = pattern_contains_glob_meta(&line);
    let compiled_match = compile_ignore_match(anchored, has_slash, has_glob_meta);
    Some(IgnoreRule {
        pattern: line,
        negated,
        dir_only,
        anchored,
        has_slash,
        compiled_match,
        source,
    })
}

/// Return whether `pattern` contains wildcard tokens that require glob matching.
fn pattern_contains_glob_meta(pattern: &str) -> bool {
    pattern
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'*' | b'?' | b'\\'))
}

/// Return one compiled matching mode for one parsed ignore pattern.
fn compile_ignore_match(
    anchored: bool,
    has_slash: bool,
    has_glob_meta: bool,
) -> CompiledIgnoreMatch {
    if has_slash {
        if anchored {
            // Anchored slash rules always evaluate against the full candidate path.
            if has_glob_meta {
                return CompiledIgnoreMatch::WholeGlob;
            }
            return CompiledIgnoreMatch::WholeLiteral;
        }
        // Unanchored slash rules behave like `**/pattern` across suffixes.
        if has_glob_meta {
            return CompiledIgnoreMatch::SuffixGlob;
        }
        return CompiledIgnoreMatch::SuffixLiteral;
    }
    if anchored {
        // Anchored single-segment rules test only the candidate root token.
        if has_glob_meta {
            return CompiledIgnoreMatch::WholeGlob;
        }
        return CompiledIgnoreMatch::WholeLiteral;
    }
    // Remaining rules are basename-scoped for both files and directories.
    if has_glob_meta {
        return CompiledIgnoreMatch::BasenameGlob;
    }
    CompiledIgnoreMatch::BasenameLiteral
}

/// Return one normalized relative path plus component-start offsets.
fn normalize_relative_path_with_offsets(path: &Path) -> (String, Vec<usize>) {
    let mut normalized = String::new();
    let mut component_offsets = Vec::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            // Record each component start so callers can slice suffixes by
            // directory depth without repeated path decomposition work.
            component_offsets.push(normalized.len());
            if !normalized.is_empty() {
                normalized.push('/');
            }
            normalized.push_str(&name.to_string_lossy());
        }
    }
    (normalized, component_offsets)
}

/// Return one candidate suffix for `directory_depth` components.
fn candidate_suffix_for_directory<'a>(
    normalized_candidate: &'a str,
    component_offsets: &[usize],
    directory_depth: usize,
) -> &'a str {
    if normalized_candidate.is_empty() || directory_depth >= component_offsets.len() {
        return "";
    }
    let suffix_start =
        component_start_from_offset(normalized_candidate, component_offsets[directory_depth]);
    &normalized_candidate[suffix_start..]
}

/// Return one normalized candidate prefix containing exactly `directory_depth` components.
fn candidate_prefix_for_depth<'a>(
    normalized_candidate: &'a str,
    component_offsets: &[usize],
    directory_depth: usize,
) -> &'a str {
    if normalized_candidate.is_empty() || directory_depth == 0 {
        return "";
    }
    if directory_depth >= component_offsets.len() {
        return normalized_candidate;
    }
    let next_component_offset = component_offsets[directory_depth];
    // Offsets may point to the slash separator or directly to the component.
    let prefix_end = if normalized_candidate.as_bytes()[next_component_offset] == b'/' {
        next_component_offset
    } else {
        next_component_offset.saturating_sub(1)
    };
    &normalized_candidate[..prefix_end]
}

/// Return one component start index for a normalized candidate offset.
fn component_start_from_offset(candidate: &str, offset: usize) -> usize {
    if candidate.as_bytes()[offset] == b'/' {
        return offset + 1;
    }
    offset
}

/// Count `Component::Normal` segments in one path.
fn normal_component_count(path: &Path) -> usize {
    path.components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count()
}

/// Match one gitignore-style glob against `text`.
///
/// Returns `true` when the pattern matches the full text, and returns `false`
/// when at least one required token does not match.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let mut pattern_index = 0;
    let mut text_index = 0;
    let mut backtrack = None;

    loop {
        if pattern_index == pattern.len() {
            if text_index == text.len() {
                return true;
            }
            if let Some((next_pattern_index, next_text_index)) =
                advance_glob_backtrack(text, &mut backtrack)
            {
                pattern_index = next_pattern_index;
                text_index = next_text_index;
                continue;
            }
            return false;
        }

        match pattern[pattern_index] {
            b'\\' => {
                let literal = pattern.get(pattern_index + 1).copied().unwrap_or(b'\\');
                let next_pattern_index = if pattern_index + 1 < pattern.len() {
                    pattern_index + 2
                } else {
                    pattern_index + 1
                };
                if text.get(text_index).copied() == Some(literal) {
                    pattern_index = next_pattern_index;
                    text_index += 1;
                    continue;
                }
            }
            b'?' => {
                if text.get(text_index).is_some_and(|byte| *byte != b'/') {
                    pattern_index += 1;
                    text_index += 1;
                    continue;
                }
            }
            b'*' => {
                let (next_pattern_index, allow_separator) =
                    consume_glob_star_run(pattern, pattern_index);
                backtrack = Some(GlobBacktrack {
                    pattern_index: next_pattern_index,
                    text_index,
                    allow_separator,
                });
                pattern_index = next_pattern_index;
                continue;
            }
            literal => {
                if text.get(text_index).copied() == Some(literal) {
                    pattern_index += 1;
                    text_index += 1;
                    continue;
                }
            }
        }

        // On mismatch, expand the most recent wildcard by one input byte.
        if let Some((next_pattern_index, next_text_index)) =
            advance_glob_backtrack(text, &mut backtrack)
        {
            pattern_index = next_pattern_index;
            text_index = next_text_index;
            continue;
        }
        return false;
    }
}

/// One wildcard backtracking checkpoint for iterative glob matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GlobBacktrack {
    /// Pattern index immediately after the wildcard run.
    pattern_index: usize,
    /// Next text index to try for expanded wildcard width.
    text_index: usize,
    /// Whether this wildcard run may cross `/` separators.
    allow_separator: bool,
}

/// Consume one contiguous `*` run and return its continuation metadata.
///
/// Returns the index after the star run and whether that run includes `**`
/// semantics that permit matching path separators.
fn consume_glob_star_run(pattern: &[u8], start_index: usize) -> (usize, bool) {
    let mut index = start_index;
    let mut allow_separator = false;
    while index < pattern.len() && pattern[index] == b'*' {
        // Any adjacent `**` segment upgrades the run to separator-aware mode.
        if index + 1 < pattern.len() && pattern[index + 1] == b'*' {
            allow_separator = true;
        }
        index += 1;
    }
    (index, allow_separator)
}

/// Advance one wildcard checkpoint by one byte of input text.
///
/// Returns `Some((pattern_index, text_index))` when a wider wildcard expansion
/// is available, and returns `None` when no further expansion is legal.
fn advance_glob_backtrack(
    text: &[u8],
    backtrack: &mut Option<GlobBacktrack>,
) -> Option<(usize, usize)> {
    let state = backtrack.as_mut()?;
    if state.text_index < text.len() {
        let byte = text[state.text_index];
        if !state.allow_separator && byte == b'/' {
            // Single-star wildcards cannot cross directory boundaries.
            *backtrack = None;
            return None;
        }
        state.text_index += 1;
        return Some((state.pattern_index, state.text_index));
    }
    *backtrack = None;
    None
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
    /// `.gitignore` `target` exclusions should survive `.ignore` reinclusion of an ancestor.
    fn test_gitignore_target_exclusion_persists_inside_reincluded_directory() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".gitignore", "ignored-by-gitignore/\ntarget\n")
            .expect("write gitignore file");
        tree.write_file(
            ".ignore",
            "!/ignored-by-gitignore/\n!/ignored-by-gitignore/reincluded/\n",
        )
        .expect("write ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().to_path_buf());
        // A reincluded source file should be visible when Git baseline starts as ignored.
        let reincluded_source = matcher
            .is_ignored_with_baseline(
                Path::new("ignored-by-gitignore/reincluded/src/main.rs"),
                PathKind::File,
                true,
            )
            .expect("evaluate reincluded source path");
        // A target artifact under the same reincluded tree remains ignored by `target`.
        let target_artifact = matcher
            .is_ignored_with_baseline(
                Path::new("ignored-by-gitignore/reincluded/target/CACHEDIR.TAG"),
                PathKind::File,
                true,
            )
            .expect("evaluate target artifact path");

        assert!(!reincluded_source);
        assert!(target_artifact);
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

    #[test]
    /// Single-star globs should not match across directory separators.
    fn test_glob_match_single_star_stays_within_directory_segment() {
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/core/main.rs"));
    }

    #[test]
    /// Double-star globs should match across nested directory boundaries.
    fn test_glob_match_double_star_crosses_directory_boundaries() {
        assert!(glob_match("src/**/main.rs", "src/core/main.rs"));
        assert!(glob_match("src/**/main.rs", "src/core/ui/main.rs"));
    }

    #[test]
    /// Unanchored literal slash rules should match descendant suffixes.
    fn test_literal_slash_rule_matches_descendant_suffix() {
        let rule = parse_ignore_line("src/lib.rs", IgnoreRuleSource::PickerIgnore)
            .expect("parse literal slash rule");
        assert!(rule.matches("src/lib.rs", PathKind::File));
        assert!(rule.matches("nested/src/lib.rs", PathKind::File));
        assert!(!rule.matches("nested/src/lib2.rs", PathKind::File));
    }

    #[test]
    /// Literal basename rules should match only the final path token.
    fn test_literal_basename_rule_matches_only_basename() {
        let rule = parse_ignore_line("target", IgnoreRuleSource::PickerIgnore)
            .expect("parse basename rule");
        assert!(rule.matches("target", PathKind::Directory));
        assert!(rule.matches("build/target", PathKind::Directory));
        assert!(!rule.matches("target/debug", PathKind::Directory));
    }

    #[test]
    /// Candidate prefix/suffix slicing should skip separators encoded in component offsets.
    fn test_candidate_prefix_and_suffix_handle_separator_offsets() {
        let (candidate, offsets) = normalize_relative_path_with_offsets(Path::new("a/b/c"));
        assert_eq!(candidate_prefix_for_depth(&candidate, &offsets, 1), "a");
        assert_eq!(candidate_prefix_for_depth(&candidate, &offsets, 2), "a/b");
        assert_eq!(
            candidate_suffix_for_directory(&candidate, &offsets, 1),
            "b/c"
        );
        assert_eq!(candidate_suffix_for_directory(&candidate, &offsets, 2), "c");
    }

    #[test]
    /// Ceiling-based lookup should load root and descendant rules within the ceiling subtree.
    fn test_rules_ceiling_applies_root_and_descendant_rules_in_subtree() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file(".ignore", "outside.txt\n")
            .expect("write outer ignore file");
        tree.write_file("nested/.ignore", "inside.txt\n")
            .expect("write ceiling ignore file");
        tree.write_file("nested/src/.ignore", "deep.txt\n")
            .expect("write descendant ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().join("nested/src"));
        matcher.set_rules_ceiling(Some(tree.path().join("nested")));
        let inside = matcher
            .is_ignored(Path::new("inside.txt"), PathKind::File)
            .expect("evaluate ceiling root rule");
        let deep = matcher
            .is_ignored(Path::new("deep.txt"), PathKind::File)
            .expect("evaluate descendant rule");
        let outside = matcher
            .is_ignored(Path::new("outside.txt"), PathKind::File)
            .expect("evaluate outside ceiling rule");

        assert!(inside);
        assert!(deep);
        assert!(!outside);
    }

    #[test]
    /// A ceiling outside the scan root should still evaluate only the ceiling root rules.
    fn test_rules_ceiling_outside_scan_root_uses_ceiling_root_rules_only() {
        let tree = TempTree::new().expect("create temp tree");
        tree.write_file("ceiling/.ignore", "visible.txt\n")
            .expect("write ceiling ignore file");
        tree.write_file("scan/.ignore", "*.txt\n")
            .expect("write scan ignore file");

        let mut matcher = IgnoreMatcher::new(tree.path().join("scan"));
        matcher.set_rules_ceiling(Some(tree.path().join("ceiling")));
        let visible = matcher
            .is_ignored(Path::new("visible.txt"), PathKind::File)
            .expect("evaluate ceiling-root lookup");
        let regular = matcher
            .is_ignored(Path::new("regular.txt"), PathKind::File)
            .expect("evaluate path outside ceiling subtree");

        assert!(visible);
        assert!(!regular);
    }
}
